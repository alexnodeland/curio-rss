//! [`PolicedClient`] — the reqwest client factory with the SSRF policy,
//! manual redirect handling, size caps and politeness built in. rustls is the
//! default TLS stack; a per-host override may opt into the platform-native
//! stack (reddit.com, whose CDN fingerprint-blocks rustls).

use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use reqwest::header::{
    ACCEPT, AUTHORIZATION, ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED, LOCATION,
    RETRY_AFTER,
};
use url::{Host, Url};

use super::{FetchError, FetchRequest, FetchResponse, policy};

/// Default streaming response-body cap: 10 MB.
pub const DEFAULT_MAX_BODY_BYTES: u64 = 10 * 1024 * 1024;

/// Contract W1 / D7: redirect hop cap.
pub const DEFAULT_MAX_REDIRECTS: u32 = 5;

/// The default `Accept` header sent on every request. Harmless and
/// spec-correct — it just tells picky feed hosts we want a feed, improving
/// the odds a content-negotiating server hands back XML rather than HTML.
pub const DEFAULT_ACCEPT: &str =
    "application/rss+xml, application/atom+xml;q=0.9, text/xml;q=0.8, */*;q=0.5";

/// The browser-class User-Agent sent to reddit.com. Paired with the native
/// TLS stack (see [`HostOverride::use_native_tls`]): a live diagnosis showed
/// Reddit blocks curio at *both* the rustls TLS fingerprint (a hard 403) and
/// the honest UA, and that native-TLS + this UA together get a clean 200.
/// Disclosed in PRIVACY.md — reddit.com is the only host that sees anything
/// but the honest curio UA.
pub const REDDIT_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
     AppleWebKit/537.36 (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";

/// A per-host policy override, matched by case-insensitive host **suffix**:
/// `reddit.com` matches `www.reddit.com` and `old.reddit.com`, but not
/// `notreddit.com` or `reddit.com.evil.example`. Lets one picky host get a
/// longer politeness delay, extra request headers, or a different
/// User-Agent — without disturbing the honest defaults every other host sees.
#[derive(Debug, Clone)]
pub struct HostOverride {
    /// Host suffix to match (no leading dot), e.g. `"reddit.com"`.
    pub host_suffix: String,
    /// Replaces the default User-Agent for matching hosts, if set (e.g.
    /// reddit.com, which hard-blocks the honest curio UA with a 403).
    pub user_agent: Option<String>,
    /// Extra request headers sent to matching hosts (name, value).
    pub extra_headers: Vec<(String, String)>,
    /// Overrides the politeness delay for matching hosts, if set.
    pub politeness_delay: Option<Duration>,
    /// Use the platform-native TLS stack (`SecureTransport` / `SChannel` /
    /// `OpenSSL`) for matching hosts instead of the default rustls. Some CDNs
    /// (reddit.com) fingerprint the rustls `ClientHello` and 403 it; the native
    /// stack presents a browser-like fingerprint they accept. Off by default —
    /// rustls stays the deliberate choice for every other host.
    pub use_native_tls: bool,
}

/// Configuration of a [`PolicedClient`].
#[derive(Debug, Clone)]
pub struct FetchConfig {
    /// The honest User-Agent every request carries.
    pub user_agent: String,
    /// The `Accept` header sent on every request. See [`DEFAULT_ACCEPT`].
    pub accept: String,
    /// Per-host policy overrides, matched by host suffix (first match wins).
    pub host_overrides: Vec<HostOverride>,
    /// TCP connect timeout.
    pub connect_timeout: Duration,
    /// Total per-request timeout (covers reading the body).
    pub request_timeout: Duration,
    /// Streaming response-body cap in bytes.
    pub max_body_bytes: u64,
    /// Redirect hop cap; every hop is re-validated.
    pub max_redirects: u32,
    /// Minimum spacing between requests to the same host. Zero disables.
    pub politeness_delay: Duration,
    /// Harness escape hatch: individual addresses exempted from the SSRF
    /// guard so hermetic test fixtures on `127.0.0.1` can exercise the
    /// deny-private policy against *other* private targets. Never set in
    /// production configuration — the per-feed contract-W1 exemption is
    /// [`FetchRequest::allow_private_network`].
    pub trusted_addrs: HashSet<IpAddr>,
}

impl Default for FetchConfig {
    fn default() -> Self {
        Self {
            user_agent: format!(
                "curio/{} (+https://github.com/alexnodeland/curio-rss)",
                env!("CARGO_PKG_VERSION")
            ),
            accept: DEFAULT_ACCEPT.to_owned(),
            // reddit.com blocks curio at two layers: the rustls TLS
            // fingerprint (a hard 403) and the honest curio UA. So it — and
            // only it — gets the platform-native TLS stack (whose ClientHello
            // Reddit accepts, verified live: native-TLS + browser UA -> 200)
            // plus a browser-class UA and a browser-ish Accept-Language.
            // The 6.5s politeness delay keeps the request rate (~9/min)
            // under Reddit's unauthenticated ~10/min limit — this paces the
            // .rss fetches AND the enrich-reddit JSON calls, which share the
            // host. Disclosed in PRIVACY.md; every other host keeps rustls +
            // the honest curio UA.
            host_overrides: vec![
                // Authenticated API calls (BYO OAuth, D15) get their own,
                // faster lane: the free tier allows 100 QPM per client, so
                // 700ms (~85/min) leaves margin. Listed FIRST — overrides
                // are first-match-wins and `reddit.com` would swallow it.
                HostOverride {
                    host_suffix: "oauth.reddit.com".to_owned(),
                    user_agent: None,
                    extra_headers: Vec::new(),
                    politeness_delay: Some(Duration::from_millis(700)),
                    use_native_tls: true,
                },
                HostOverride {
                    host_suffix: "reddit.com".to_owned(),
                    user_agent: Some(REDDIT_USER_AGENT.to_owned()),
                    extra_headers: vec![(
                        "accept-language".to_owned(),
                        "en-US,en;q=0.9".to_owned(),
                    )],
                    politeness_delay: Some(Duration::from_millis(6500)),
                    use_native_tls: true,
                },
            ],
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(30),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_redirects: DEFAULT_MAX_REDIRECTS,
            politeness_delay: Duration::from_millis(500),
            trusted_addrs: HashSet::new(),
        }
    }
}

/// The policy-hardened client. One instance serves the whole engine;
/// it is `Send + Sync` and cheap to share behind an `Arc`.
#[derive(Debug)]
pub struct PolicedClient {
    config: FetchConfig,
    /// Per-host next-allowed-request instants (politeness reservations).
    next_allowed: Mutex<HashMap<String, Instant>>,
}

impl Default for PolicedClient {
    fn default() -> Self {
        Self::new(FetchConfig::default())
    }
}

impl PolicedClient {
    /// A client with the given policy configuration.
    #[must_use]
    pub fn new(config: FetchConfig) -> Self {
        Self {
            config,
            next_allowed: Mutex::new(HashMap::new()),
        }
    }

    /// The active configuration.
    #[must_use]
    pub fn config(&self) -> &FetchConfig {
        &self.config
    }

    /// The first host override whose suffix matches `url`'s host, if any.
    /// Suffix match is case-insensitive and boundary-aware: `reddit.com`
    /// matches `reddit.com` and `*.reddit.com`, never `notreddit.com`.
    fn override_for(&self, url: &Url) -> Option<&HostOverride> {
        let host = url.host_str()?.to_ascii_lowercase();
        self.config.host_overrides.iter().find(|entry| {
            let suffix = entry.host_suffix.to_ascii_lowercase();
            host == suffix || host.ends_with(&format!(".{suffix}"))
        })
    }

    /// Performs one policed GET: validate → resolve → check policy →
    /// politeness → request → (re-validated) redirects → capped body read.
    ///
    /// Redirect statuses never surface: they are followed (cap
    /// [`FetchConfig::max_redirects`], every hop re-validated) or turned
    /// into an error. 304 and HTTP error statuses come back as a normal
    /// [`FetchResponse`] with an empty body — the caller owns the mapping
    /// to feed health / fetch-log semantics.
    ///
    /// # Errors
    ///
    /// Any [`FetchError`]: invalid/forbidden URL, DNS failure, the SSRF
    /// guard, redirect-cap or missing-Location, body cap, timeouts, or
    /// transport failures.
    pub async fn fetch(&self, request: &FetchRequest) -> Result<FetchResponse, FetchError> {
        let mut url = parse_and_screen(&request.url)?;
        let mut hops: u32 = 0;
        let mut all_permanent = true;

        loop {
            let pinned = self
                .resolve_and_check(&url, request.allow_private_network)
                .await?;
            let host_override = self.override_for(&url);
            self.be_polite(&url, host_override).await;

            let user_agent = host_override
                .and_then(|entry| entry.user_agent.as_deref())
                .unwrap_or(self.config.user_agent.as_str());
            let native_tls = host_override.is_some_and(|entry| entry.use_native_tls);
            let client = self.build_hop_client(&url, pinned, user_agent, native_tls)?;
            let mut req = client
                .get(url.clone())
                .header(ACCEPT, self.config.accept.as_str());
            if let Some(host_override) = host_override {
                for (name, value) in &host_override.extra_headers {
                    req = req.header(name.as_str(), value.as_str());
                }
            }
            if let Some(etag) = &request.etag {
                req = req.header(IF_NONE_MATCH, etag);
            }
            if let Some(last_modified) = &request.last_modified {
                req = req.header(IF_MODIFIED_SINCE, last_modified);
            }
            if let Some(bearer) = &request.bearer {
                req = req.header(AUTHORIZATION, format!("Bearer {bearer}"));
            }
            let response = req.send().await.map_err(|err| map_reqwest(&url, &err))?;

            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                hops += 1;
                if hops > self.config.max_redirects {
                    return Err(FetchError::TooManyRedirects {
                        limit: self.config.max_redirects,
                        url: request.url.clone(),
                    });
                }
                all_permanent &= matches!(status, 301 | 308);
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| FetchError::MissingRedirectLocation {
                        url: url.to_string(),
                    })?;
                let next = url
                    .join(location)
                    .map_err(|err| FetchError::InvalidUrl {
                        url: location.to_owned(),
                        message: err.to_string(),
                    })?
                    .to_string();
                // Re-validation: the next hop goes through the exact same
                // scheme/userinfo screen — and, at the top of the loop,
                // the same DNS + address policy — as the first hop.
                url = parse_and_screen(&next)?;
                continue;
            }

            let etag = header_string(&response, ETAG.as_str());
            let last_modified = header_string(&response, LAST_MODIFIED.as_str());
            // Retry-After in its delta-seconds form (the shape rate limiters
            // send); the rare HTTP-date form is ignored rather than parsed.
            let retry_after = header_string(&response, RETRY_AFTER.as_str())
                .and_then(|value| value.trim().parse::<u64>().ok())
                .map(Duration::from_secs);
            let body = if response.status().is_success() {
                self.read_capped(&url, response).await?
            } else {
                Vec::new()
            };
            return Ok(FetchResponse {
                status,
                final_url: url.to_string(),
                permanent_redirect: hops > 0 && all_permanent,
                etag,
                last_modified,
                retry_after,
                body,
            });
        }
    }

    /// Performs one policed form POST — the OAuth token-grant shape:
    /// HTTP Basic credentials plus an `application/x-www-form-urlencoded`
    /// body, through the exact same validate → resolve → policy →
    /// politeness pipeline as [`Self::fetch`]. Redirects are refused
    /// ([`FetchError::RedirectNotAllowed`]) rather than followed.
    ///
    /// # Errors
    ///
    /// Any [`FetchError`], plus `RedirectNotAllowed` on a 3xx answer.
    pub async fn post_form(
        &self,
        raw_url: &str,
        basic_auth: (&str, &str),
        form: &[(&str, &str)],
    ) -> Result<FetchResponse, FetchError> {
        let url = parse_and_screen(raw_url)?;
        let pinned = self.resolve_and_check(&url, false).await?;
        let host_override = self.override_for(&url);
        self.be_polite(&url, host_override).await;

        let user_agent = host_override
            .and_then(|entry| entry.user_agent.as_deref())
            .unwrap_or(self.config.user_agent.as_str());
        let native_tls = host_override.is_some_and(|entry| entry.use_native_tls);
        let client = self.build_hop_client(&url, pinned, user_agent, native_tls)?;
        let mut req = client
            .post(url.clone())
            .basic_auth(basic_auth.0, Some(basic_auth.1))
            .form(form);
        if let Some(host_override) = host_override {
            for (name, value) in &host_override.extra_headers {
                req = req.header(name.as_str(), value.as_str());
            }
        }
        let response = req.send().await.map_err(|err| map_reqwest(&url, &err))?;

        let status = response.status().as_u16();
        if matches!(status, 301 | 302 | 303 | 307 | 308) {
            return Err(FetchError::RedirectNotAllowed {
                url: url.to_string(),
            });
        }
        let retry_after = header_string(&response, RETRY_AFTER.as_str())
            .and_then(|value| value.trim().parse::<u64>().ok())
            .map(Duration::from_secs);
        let body = if response.status().is_success() {
            self.read_capped(&url, response).await?
        } else {
            Vec::new()
        };
        Ok(FetchResponse {
            status,
            final_url: url.to_string(),
            permanent_redirect: false,
            etag: None,
            last_modified: None,
            retry_after,
            body,
        })
    }

    /// Resolves the URL's host (DNS first for domain names) and applies
    /// the address policy to *every* resolved address — one private
    /// record poisons the lot, so split-horizon/rebinding games with
    /// mixed record sets cannot slip through. Returns the address to pin
    /// the connection to for domain-name hosts.
    async fn resolve_and_check(
        &self,
        url: &Url,
        allow_private: bool,
    ) -> Result<Option<SocketAddr>, FetchError> {
        let host = url.host().ok_or_else(|| FetchError::NoHost {
            url: url.to_string(),
        })?;
        let port = url.port_or_known_default().unwrap_or(80);
        match host {
            Host::Ipv4(v4) => {
                self.check_addr(url, IpAddr::V4(v4), allow_private)?;
                Ok(None)
            }
            Host::Ipv6(v6) => {
                self.check_addr(url, IpAddr::V6(v6), allow_private)?;
                Ok(None)
            }
            Host::Domain(domain) => {
                let addrs: Vec<SocketAddr> = tokio::net::lookup_host((domain, port))
                    .await
                    .map_err(|err| FetchError::Dns {
                        host: domain.to_owned(),
                        message: err.to_string(),
                    })?
                    .collect();
                if addrs.is_empty() {
                    return Err(FetchError::DnsNoAddresses {
                        host: domain.to_owned(),
                    });
                }
                for addr in &addrs {
                    self.check_addr(url, addr.ip(), allow_private)?;
                }
                Ok(addrs.first().copied())
            }
        }
    }

    fn check_addr(&self, url: &Url, addr: IpAddr, allow_private: bool) -> Result<(), FetchError> {
        if allow_private || self.config.trusted_addrs.contains(&addr) {
            return Ok(());
        }
        if policy::is_public_address(addr) {
            return Ok(());
        }
        Err(FetchError::PrivateAddress {
            host: url.host_str().unwrap_or_default().to_owned(),
            addr,
        })
    }

    /// One reqwest client per hop: redirects disabled (we follow them
    /// ourselves), and for domain-name hosts the connection is pinned to
    /// the address that was just validated — closing the DNS-rebinding
    /// window between the check and the connect.
    fn build_hop_client(
        &self,
        url: &Url,
        pinned: Option<SocketAddr>,
        user_agent: &str,
        native_tls: bool,
    ) -> Result<reqwest::Client, FetchError> {
        // rustls is the deliberate default for every host; a per-host override
        // (reddit.com) opts into the platform-native stack to dodge a
        // ClientHello-fingerprint block. Both backends are compiled in.
        let tls = if native_tls {
            reqwest::Client::builder().use_native_tls()
        } else {
            reqwest::Client::builder().use_rustls_tls()
        };
        let mut builder = tls
            .user_agent(user_agent)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(self.config.connect_timeout)
            .timeout(self.config.request_timeout);
        if let (Some(addr), Some(domain)) = (pinned, url.domain()) {
            builder = builder.resolve(domain, addr);
        }
        builder.build().map_err(|err| map_reqwest(url, &err))
    }

    /// Streams the body, enforcing the size cap (a lying or absent
    /// `Content-Length` cannot bypass it).
    async fn read_capped(
        &self,
        url: &Url,
        mut response: reqwest::Response,
    ) -> Result<Vec<u8>, FetchError> {
        let cap = self.config.max_body_bytes;
        let too_large = || FetchError::BodyTooLarge {
            limit: cap,
            url: url.to_string(),
        };
        if response.content_length().is_some_and(|len| len > cap) {
            return Err(too_large());
        }
        let mut body: Vec<u8> = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|err| map_reqwest(url, &err))?
        {
            if (body.len() as u64).saturating_add(chunk.len() as u64) > cap {
                return Err(too_large());
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }

    /// Per-host politeness: reserves the next allowed slot under the lock
    /// (so concurrent fetches to one host queue up honestly), then sleeps
    /// out its own wait without holding the lock.
    async fn be_polite(&self, url: &Url, host_override: Option<&HostOverride>) {
        let delay = host_override
            .and_then(|entry| entry.politeness_delay)
            .unwrap_or(self.config.politeness_delay);
        if delay.is_zero() {
            return;
        }
        let Some(host) = url.host_str() else { return };
        let wait = {
            #[allow(clippy::unwrap_used, reason = "politeness lock is never poisoned")]
            let mut slots = self.next_allowed.lock().unwrap();
            let now = Instant::now();
            let at = slots
                .get(host)
                .copied()
                .map_or(now, |reserved| reserved.max(now));
            slots.insert(host.to_owned(), at + delay);
            at.saturating_duration_since(now)
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
    }
}

/// Parses a URL and screens its shape: http/https only, no userinfo.
/// Runs on the first hop and on every redirect target.
fn parse_and_screen(raw: &str) -> Result<Url, FetchError> {
    let url = Url::parse(raw).map_err(|err| FetchError::InvalidUrl {
        url: raw.to_owned(),
        message: err.to_string(),
    })?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(FetchError::UnsupportedScheme {
            scheme: url.scheme().to_owned(),
            url: raw.to_owned(),
        });
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(FetchError::UserinfoForbidden {
            url: raw.to_owned(),
        });
    }
    if url.host().is_none() {
        return Err(FetchError::NoHost {
            url: raw.to_owned(),
        });
    }
    Ok(url)
}

fn header_string(response: &reqwest::Response, name: &str) -> Option<String> {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(ToOwned::to_owned)
}

fn map_reqwest(url: &Url, err: &reqwest::Error) -> FetchError {
    if err.is_timeout() {
        FetchError::Timeout {
            url: url.to_string(),
        }
    } else {
        FetchError::Transport {
            url: url.to_string(),
            message: err.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn screen_rejects_non_http_schemes() {
        for bad in [
            "ftp://example.com/feed",
            "file:///etc/passwd",
            "gopher://example.com/",
            "javascript:alert(1)",
        ] {
            assert!(matches!(
                parse_and_screen(bad),
                Err(FetchError::UnsupportedScheme { .. })
            ));
        }
    }

    #[test]
    fn screen_rejects_userinfo() {
        assert!(matches!(
            parse_and_screen("http://user:pass@example.com/"),
            Err(FetchError::UserinfoForbidden { .. })
        ));
        assert!(matches!(
            parse_and_screen("http://user@example.com/"),
            Err(FetchError::UserinfoForbidden { .. })
        ));
    }

    #[test]
    fn screen_rejects_garbage() {
        assert!(matches!(
            parse_and_screen("not a url"),
            Err(FetchError::InvalidUrl { .. })
        ));
    }

    #[test]
    fn screen_accepts_plain_http_and_https() {
        assert!(parse_and_screen("http://example.com/feed.xml").is_ok());
        assert!(parse_and_screen("https://example.com:8443/feed.xml").is_ok());
    }

    #[test]
    fn default_config_matches_the_contract() {
        let config = FetchConfig::default();
        assert_eq!(config.max_redirects, 5);
        assert_eq!(config.max_body_bytes, 10 * 1024 * 1024);
        assert!(config.user_agent.starts_with("curio/"));
        assert!(config.trusted_addrs.is_empty());
        assert!(config.accept.contains("application/rss+xml"));
        // Two default overrides, most-specific first (first match wins):
        // the authenticated API lane, then the general reddit.com policy.
        assert_eq!(config.host_overrides.len(), 2);
        let oauth = &config.host_overrides[0];
        assert_eq!(oauth.host_suffix, "oauth.reddit.com");
        // ~85/min: under the authenticated 100 QPM free tier.
        assert_eq!(oauth.politeness_delay, Some(Duration::from_millis(700)));
        let reddit = &config.host_overrides[1];
        assert_eq!(reddit.host_suffix, "reddit.com");
        // ~9/min: under Reddit's unauthenticated ~10/min rate limit.
        assert_eq!(reddit.politeness_delay, Some(Duration::from_millis(6500)));
        assert_eq!(reddit.user_agent.as_deref(), Some(REDDIT_USER_AGENT));
        assert!(
            reddit.use_native_tls && oauth.use_native_tls,
            "reddit hosts must use the native TLS stack"
        );
    }

    #[test]
    fn host_override_matches_by_suffix_only() {
        let client = PolicedClient::default();
        let suffix_of = |raw: &str| {
            client
                .override_for(&Url::parse(raw).unwrap())
                .map(|entry| entry.host_suffix.clone())
        };
        // Exact and sub-domain matches hit the reddit.com override…
        assert_eq!(
            suffix_of("https://reddit.com/r/rust/.rss").as_deref(),
            Some("reddit.com")
        );
        assert_eq!(
            suffix_of("https://www.reddit.com/r/rust/.rss").as_deref(),
            Some("reddit.com")
        );
        assert_eq!(
            suffix_of("https://old.reddit.com/r/rust/.rss").as_deref(),
            Some("reddit.com")
        );
        // …but a look-alike or unrelated host does not.
        assert_eq!(suffix_of("https://notreddit.com/feed"), None);
        assert_eq!(suffix_of("https://reddit.com.evil.example/feed"), None);
        assert_eq!(suffix_of("https://example.com/feed"), None);
    }
}
