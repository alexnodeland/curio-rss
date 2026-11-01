//! The fetch error taxonomy: [`FetchError`], its machine-readable
//! [`FetchErrorKind`] class, and the typing of a raw reqwest failure by its
//! source chain (refused, reset, TLS certificate or handshake, connect vs
//! read timeout) — so a message names the cause, and the URL only once.

use std::net::IpAddr;

use url::Url;

use crate::model::FetchErrorKind;

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
    /// A redirect pointed off the request's origin while
    /// [`FetchRequest::same_origin_redirects`](super::FetchRequest::same_origin_redirects) was set. The target was
    /// never requested.
    #[error("{url:?} redirected off its origin to {location:?} — not followed")]
    CrossOriginRedirect {
        /// The URL that answered with the redirect.
        url: String,
        /// The cross-origin redirect target.
        location: String,
        /// Redirect hops already followed before the refused one.
        hops: u32,
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
    /// The request timed out.
    #[error("{phase} timeout fetching {url:?}")]
    Timeout {
        /// The URL that timed out.
        url: String,
        /// Whether connecting or answering took too long.
        phase: TimeoutPhase,
    },
    /// A transport-level failure: the connection was refused or reset,
    /// TLS failed, or something else went wrong on the wire.
    #[error("{kind} fetching {url:?}: {cause}")]
    Transport {
        /// The URL that failed.
        url: String,
        /// What went wrong, read off the error's source chain.
        kind: TransportKind,
        /// The root cause as the OS / TLS stack reported it (never the
        /// URL again).
        cause: String,
    },
}

impl FetchError {
    /// The failure class, for heads to branch on and for `fetch_log`.
    #[must_use]
    pub fn kind(&self) -> FetchErrorKind {
        match self {
            Self::InvalidUrl { .. }
            | Self::UnsupportedScheme { .. }
            | Self::UserinfoForbidden { .. }
            | Self::NoHost { .. } => FetchErrorKind::InvalidUrl,
            Self::Dns { .. } | Self::DnsNoAddresses { .. } => FetchErrorKind::Dns,
            Self::PrivateAddress { .. } => FetchErrorKind::Blocked,
            Self::RedirectNotAllowed { .. }
            | Self::TooManyRedirects { .. }
            | Self::CrossOriginRedirect { .. }
            | Self::MissingRedirectLocation { .. } => FetchErrorKind::Redirect,
            Self::BodyTooLarge { .. } => FetchErrorKind::TooLarge,
            Self::Timeout {
                phase: TimeoutPhase::Connect,
                ..
            } => FetchErrorKind::ConnectTimeout,
            Self::Timeout {
                phase: TimeoutPhase::Read,
                ..
            } => FetchErrorKind::ReadTimeout,
            Self::Transport { kind, .. } => match kind {
                TransportKind::Refused => FetchErrorKind::Refused,
                TransportKind::Reset => FetchErrorKind::Reset,
                TransportKind::TlsCertificate => FetchErrorKind::TlsCertificate,
                TransportKind::TlsHandshake => FetchErrorKind::TlsHandshake,
                TransportKind::Other => FetchErrorKind::Transport,
            },
        }
    }
}

/// Which phase of a request ran out of time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeoutPhase {
    /// Establishing the connection (TCP + TLS) exceeded
    /// [`FetchConfig::connect_timeout`](super::FetchConfig::connect_timeout).
    Connect,
    /// The server accepted the connection but the whole request did not
    /// finish within [`FetchConfig::request_timeout`](super::FetchConfig::request_timeout).
    Read,
}

impl std::fmt::Display for TimeoutPhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Connect => "connect",
            Self::Read => "read",
        })
    }
}

/// What kind of transport failure a [`FetchError::Transport`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportKind {
    /// Nothing accepted the connection (`ECONNREFUSED`).
    Refused,
    /// The connection was reset, aborted or broken mid-request.
    Reset,
    /// The server's certificate failed verification (self-signed, expired,
    /// unknown issuer, wrong name).
    TlsCertificate,
    /// The TLS handshake failed for another reason.
    TlsHandshake,
    /// Anything else.
    Other,
}

impl std::fmt::Display for TransportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Refused => "connection refused",
            Self::Reset => "connection reset",
            Self::TlsCertificate => "TLS certificate rejected",
            Self::TlsHandshake => "TLS handshake failed",
            Self::Other => "transport error",
        })
    }
}

/// Types a reqwest failure. reqwest's own `Display` is just "error sending
/// request for url (…)" — the URL again and no cause — so the class and
/// the message both come from walking its `source()` chain.
pub(super) fn map_reqwest(url: &Url, err: &reqwest::Error) -> FetchError {
    if err.is_timeout() {
        // A connect error that timed out is the connect timeout; anything
        // else is the overall request timeout expiring mid-answer.
        let phase = if err.is_connect() {
            TimeoutPhase::Connect
        } else {
            TimeoutPhase::Read
        };
        return FetchError::Timeout {
            url: url.to_string(),
            phase,
        };
    }
    let (kind, cause) = classify_transport(err);
    FetchError::Transport {
        url: url.to_string(),
        kind,
        cause,
    }
}

/// Classifies a transport failure by its source chain and picks its root
/// cause as the message.
///
/// The OS-level classes come from the [`std::io::ErrorKind`] of an
/// `io::Error` in the chain. TLS failures are recognised by text: both
/// stacks (rustls by default, native-tls for an override host) surface them
/// as opaque `io::Error`s whose message names the problem
/// (`invalid peer certificate: UnknownIssuer`, `certificate verify
/// failed`, `received fatal alert: HandshakeFailure`). The top-level error
/// is skipped for both — its text is reqwest's URL-bearing summary.
fn classify_transport(err: &(dyn std::error::Error + 'static)) -> (TransportKind, String) {
    let mut kind = TransportKind::Other;
    let mut cause: Option<String> = None;
    let mut next = err.source();
    while let Some(source) = next {
        if let Some(io) = source.downcast_ref::<std::io::Error>() {
            match io.kind() {
                std::io::ErrorKind::ConnectionRefused => kind = TransportKind::Refused,
                std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::UnexpectedEof
                    if kind == TransportKind::Other =>
                {
                    kind = TransportKind::Reset;
                }
                _ => {}
            }
        }
        let text = source.to_string();
        let lower = text.to_ascii_lowercase();
        if lower.contains("certificate") {
            kind = TransportKind::TlsCertificate;
        } else if kind != TransportKind::TlsCertificate
            && (lower.contains("tls") || lower.contains("ssl") || lower.contains("handshake"))
        {
            kind = TransportKind::TlsHandshake;
        }
        if !text.is_empty() {
            cause = Some(text);
        }
        next = source.source();
    }
    // No chain at all: reqwest's own text is all there is.
    (kind, cause.unwrap_or_else(|| err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One link of a synthetic error chain, shaped like reqwest's: the top
    /// layer names the URL, the layers below carry the cause.
    #[derive(Debug)]
    struct Layer(String, Option<Box<dyn std::error::Error + 'static>>);

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(&self.0)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.1.as_deref()
        }
    }

    fn chain(below: Box<dyn std::error::Error + 'static>) -> Layer {
        Layer(
            "error sending request for url (https://tls.example/feed)".to_owned(),
            Some(Box::new(Layer(
                "client error (Connect)".to_owned(),
                Some(below),
            ))),
        )
    }

    fn io(kind: std::io::ErrorKind, text: &str) -> Box<dyn std::error::Error + 'static> {
        Box::new(std::io::Error::new(kind, text.to_owned()))
    }

    #[test]
    fn transport_failures_are_classified_by_their_source_chain() {
        use std::io::ErrorKind;
        let cases = [
            (
                io(
                    ErrorKind::ConnectionRefused,
                    "Connection refused (os error 111)",
                ),
                TransportKind::Refused,
                "Connection refused (os error 111)",
            ),
            (
                io(ErrorKind::ConnectionReset, "Connection reset by peer"),
                TransportKind::Reset,
                "Connection reset by peer",
            ),
            (
                io(ErrorKind::BrokenPipe, "Broken pipe"),
                TransportKind::Reset,
                "Broken pipe",
            ),
            (
                io(
                    ErrorKind::InvalidData,
                    "invalid peer certificate: UnknownIssuer",
                ),
                TransportKind::TlsCertificate,
                "invalid peer certificate: UnknownIssuer",
            ),
            (
                io(
                    ErrorKind::Other,
                    "error:0A000086:SSL routines::certificate verify failed",
                ),
                TransportKind::TlsCertificate,
                "error:0A000086:SSL routines::certificate verify failed",
            ),
            (
                io(
                    ErrorKind::InvalidData,
                    "received fatal alert: HandshakeFailure",
                ),
                TransportKind::TlsHandshake,
                "received fatal alert: HandshakeFailure",
            ),
            // A TLS EOF is a handshake failure, not a plain reset.
            (
                io(ErrorKind::UnexpectedEof, "tls handshake eof"),
                TransportKind::TlsHandshake,
                "tls handshake eof",
            ),
            (
                Box::new(Layer("dns error: no record".to_owned(), None)),
                TransportKind::Other,
                "dns error: no record",
            ),
        ];
        for (below, kind, cause) in cases {
            let err = chain(below);
            // The URL host says "tls", but the top layer is never sniffed.
            assert_eq!(classify_transport(&err), (kind, cause.to_owned()));
        }
    }

    #[test]
    fn a_chainless_error_falls_back_to_its_own_text() {
        let err = Layer("something odd".to_owned(), None);
        assert_eq!(
            classify_transport(&err),
            (TransportKind::Other, "something odd".to_owned())
        );
    }

    #[test]
    fn every_fetch_error_has_a_kind() {
        use crate::model::FetchErrorKind as K;
        let url = || "https://example.com/feed".to_owned();
        let cases = [
            (
                FetchError::InvalidUrl {
                    url: url(),
                    message: String::new(),
                },
                K::InvalidUrl,
            ),
            (
                FetchError::UnsupportedScheme {
                    scheme: "ftp".to_owned(),
                    url: url(),
                },
                K::InvalidUrl,
            ),
            (FetchError::UserinfoForbidden { url: url() }, K::InvalidUrl),
            (FetchError::NoHost { url: url() }, K::InvalidUrl),
            (
                FetchError::Dns {
                    host: "example.com".to_owned(),
                    message: String::new(),
                },
                K::Dns,
            ),
            (
                FetchError::DnsNoAddresses {
                    host: "example.com".to_owned(),
                },
                K::Dns,
            ),
            (
                FetchError::PrivateAddress {
                    host: "x".to_owned(),
                    addr: IpAddr::from([127, 0, 0, 1]),
                },
                K::Blocked,
            ),
            (FetchError::RedirectNotAllowed { url: url() }, K::Redirect),
            (
                FetchError::TooManyRedirects {
                    limit: 5,
                    url: url(),
                },
                K::Redirect,
            ),
            (
                FetchError::CrossOriginRedirect {
                    url: url(),
                    location: url(),
                    hops: 0,
                },
                K::Redirect,
            ),
            (
                FetchError::MissingRedirectLocation { url: url() },
                K::Redirect,
            ),
            (
                FetchError::BodyTooLarge {
                    limit: 1,
                    url: url(),
                },
                K::TooLarge,
            ),
            (
                FetchError::Timeout {
                    url: url(),
                    phase: TimeoutPhase::Connect,
                },
                K::ConnectTimeout,
            ),
            (
                FetchError::Timeout {
                    url: url(),
                    phase: TimeoutPhase::Read,
                },
                K::ReadTimeout,
            ),
        ];
        for (err, kind) in cases {
            assert_eq!(err.kind(), kind, "{err:?}");
        }
    }

    #[test]
    fn transport_and_timeout_errors_name_the_url_once_then_the_cause() {
        use crate::model::FetchErrorKind as K;
        let url = || "https://example.com/feed".to_owned();
        for (transport, kind) in [
            (TransportKind::Refused, K::Refused),
            (TransportKind::Reset, K::Reset),
            (TransportKind::TlsCertificate, K::TlsCertificate),
            (TransportKind::TlsHandshake, K::TlsHandshake),
            (TransportKind::Other, K::Transport),
        ] {
            let err = FetchError::Transport {
                url: url(),
                kind: transport,
                cause: "cause".to_owned(),
            };
            assert_eq!(err.kind(), kind);
            // One URL, then the cause.
            assert_eq!(err.to_string().matches("example.com").count(), 1, "{err}");
            assert!(err.to_string().ends_with(": cause"), "{err}");
        }
        assert_eq!(
            FetchError::Timeout {
                url: url(),
                phase: TimeoutPhase::Connect
            }
            .to_string(),
            "connect timeout fetching \"https://example.com/feed\""
        );
    }
}
