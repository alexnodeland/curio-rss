//! The policy-hardened HTTP client — the *only* outbound network path.
//!
//! Every outbound request (feeds, favicons, images) goes through
//! [`PolicedClient`]; a bare `reqwest` client anywhere else in the tree is
//! a boundary violation. The client enforces, per contract W1 and D7:
//!
//! - **SSRF guard by default**: DNS is resolved *first* and every resolved
//!   address is checked against [`policy::is_public_address`] —
//!   loopback, RFC 1918, link-local, CGNAT, ULA, multicast and friends are
//!   rejected unless the per-feed `allow_private_network` flag is set
//!   (explicit config edit only, never settable from feed content).
//! - **Redirect re-validation**: redirects are followed manually (cap
//!   [`FetchConfig::max_redirects`], default 5) and every hop re-runs the
//!   full scheme + DNS + address validation.
//! - **Streaming size cap** (default 10 MB), connect/read timeouts, and a
//!   per-host politeness delay.
//! - **Conditional GET**: `ETag`/`Last-Modified` from the previous fetch
//!   ride the request; a 304 comes back as a normal response the caller
//!   maps to [`crate::model::FetchStatus::NotModified`] while *preserving*
//!   the stored validators (the sketch clobbered them on error paths).
//! - An honest User-Agent naming Curio and the repository.

mod client;
pub mod policy;

pub use client::{DEFAULT_ACCEPT, FetchConfig, HostOverride, PolicedClient};

use std::net::IpAddr;

/// Everything the client needs to know for one policed fetch.
#[derive(Debug, Clone, Default)]
pub struct FetchRequest {
    /// The URL to fetch (http/https only).
    pub url: String,
    /// Contract W1: exempts this fetch from the private-network guard.
    /// Comes from per-feed config — never from feed content.
    pub allow_private_network: bool,
    /// `ETag` from the last successful fetch (conditional GET).
    pub etag: Option<String>,
    /// `Last-Modified` from the last successful fetch (conditional GET).
    pub last_modified: Option<String>,
    /// OAuth bearer token, sent as `Authorization: Bearer …` (the
    /// authenticated enrichment path). Never logged.
    pub bearer: Option<String>,
}

impl FetchRequest {
    /// A plain request for a URL with the default (deny-private) policy.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            ..Self::default()
        }
    }
}

/// The final response of a policed fetch (redirects already followed).
#[derive(Debug, Clone)]
pub struct FetchResponse {
    /// Final HTTP status (2xx, 304, 410, …). Redirect statuses never
    /// surface here — they are followed or rejected.
    pub status: u16,
    /// The URL that produced this response, after redirects.
    pub final_url: String,
    /// `true` iff at least one redirect was followed and *every* hop was
    /// permanent (301/308) — the caller may update the stored feed URL.
    pub permanent_redirect: bool,
    /// Response `ETag`, for the next conditional GET.
    pub etag: Option<String>,
    /// Response `Last-Modified`, for the next conditional GET.
    pub last_modified: Option<String>,
    /// `Retry-After` (delta-seconds form only), when the server sent one —
    /// how long a 429/503 asked us to back off.
    pub retry_after: Option<std::time::Duration>,
    /// Response body. Read (and size-capped) only for 2xx responses;
    /// empty for 304 and error statuses.
    pub body: Vec<u8>,
}

impl FetchResponse {
    /// 2xx.
    #[must_use]
    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// 304 — the conditional GET hit; nothing new.
    #[must_use]
    pub fn is_not_modified(&self) -> bool {
        self.status == 304
    }

    /// 410 — the feed is gone; callers auto-pause it for good.
    #[must_use]
    pub fn is_gone(&self) -> bool {
        self.status == 410
    }
}

/// Errors of the policed fetch path.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    /// The URL failed to parse.
    #[error("invalid url {url:?}: {message}")]
    InvalidUrl {
        /// The offending URL.
        url: String,
        /// Parse-failure detail.
        message: String,
    },
    /// The URL scheme is not http or https.
    #[error("unsupported scheme {scheme:?} in {url:?} — only http/https are fetched")]
    UnsupportedScheme {
        /// The offending scheme.
        scheme: String,
        /// The offending URL.
        url: String,
    },
    /// The URL carries userinfo (`user:pass@host`), a classic SSRF
    /// confusion vector — always rejected.
    #[error("url {url:?} carries userinfo, which is never allowed")]
    UserinfoForbidden {
        /// The offending URL.
        url: String,
    },
    /// The URL has no host.
    #[error("url {url:?} has no host")]
    NoHost {
        /// The offending URL.
        url: String,
    },
    /// DNS resolution failed.
    #[error("dns resolution failed for {host:?}: {message}")]
    Dns {
        /// The host that failed to resolve.
        host: String,
        /// Resolver detail.
        message: String,
    },
    /// DNS resolved to no addresses at all.
    #[error("dns returned no addresses for {host:?}")]
    DnsNoAddresses {
        /// The host that resolved empty.
        host: String,
    },
    /// The SSRF guard rejected a non-public address. Set the per-feed
    /// `allow_private_network` flag (config edit) to exempt a feed.
    #[error(
        "refusing to fetch {host:?}: resolved address {addr} is not public \
         (set allow_private_network on the feed to permit this)"
    )]
    PrivateAddress {
        /// The host (or IP literal) that was rejected.
        host: String,
        /// The offending resolved address.
        addr: IpAddr,
    },
    /// A POST answered with a redirect. Never followed: silently
    /// re-POSTing credentials across hops is exactly the surprise the
    /// policed client exists to prevent.
    #[error("{url:?} answered a POST with a redirect — refusing to follow")]
    RedirectNotAllowed {
        /// The offending URL.
        url: String,
    },
    /// The redirect cap was exceeded.
    #[error("too many redirects (limit {limit}) fetching {url:?}")]
    TooManyRedirects {
        /// The configured hop limit.
        limit: u32,
        /// The URL the fetch started from.
        url: String,
    },
    /// A redirect status arrived without a usable `Location` header.
    #[error("redirect from {url:?} carried no usable Location header")]
    MissingRedirectLocation {
        /// The URL that responded with the bad redirect.
        url: String,
    },
    /// The response body exceeded the size cap.
    #[error("response body exceeds the {limit}-byte cap fetching {url:?}")]
    BodyTooLarge {
        /// The configured cap in bytes.
        limit: u64,
        /// The URL whose body was too large.
        url: String,
    },
    /// The request timed out (connect or read).
    #[error("timeout fetching {url:?}")]
    Timeout {
        /// The URL that timed out.
        url: String,
    },
    /// Any other transport-level failure.
    #[error("transport error fetching {url:?}: {message}")]
    Transport {
        /// The URL that failed.
        url: String,
        /// Transport detail.
        message: String,
    },
}
