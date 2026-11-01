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
//! - **Typed failures**: every [`FetchError`] has a stable
//!   [`crate::model::FetchErrorKind`] ([`FetchError::kind`]); transport
//!   failures are classified by their source chain (refused, reset, TLS
//!   certificate vs handshake, connect vs read timeout) and carry the root
//!   cause, not reqwest's URL-repeating summary.

mod client;
mod errors;
pub mod policy;

pub use client::{DEFAULT_ACCEPT, FetchConfig, HostOverride, PolicedClient};
pub use errors::{FetchError, TimeoutPhase, TransportKind};

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
    /// Refuse — without requesting it — any redirect hop whose origin
    /// (scheme, host, port) differs from [`Self::url`]'s, failing with
    /// [`FetchError::CrossOriginRedirect`]. Discovery probes set it so a
    /// guessed path can never send a request off the page's origin.
    pub same_origin_redirects: bool,
    /// Tightens [`FetchConfig::max_redirects`] for this request (never
    /// loosens it: the lower of the two applies). `Some(0)` refuses every
    /// redirect.
    pub max_redirects: Option<u32>,
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
    /// How many redirect hops were followed — the fetch cost
    /// `1 + redirects` HTTP requests.
    pub redirects: u32,
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
