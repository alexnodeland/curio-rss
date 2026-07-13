//! BYO Reddit OAuth (D15): the application-only `client_credentials`
//! grant. Users register their own free app at reddit.com/prefs/apps;
//! the heads load the client id + secret (from the OS keychain) into
//! the running core, which trades them for short-lived bearer tokens
//! through the policed client. Authenticated calls go to
//! `oauth.reddit.com` at the free tier's 100 QPM instead of the
//! unauthenticated ~10 QPM.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use url::Url;

use super::EnrichError;
use crate::fetch::PolicedClient;

/// Reddit's real token endpoint; tests inject a loopback one.
const DEFAULT_TOKEN_URL: &str = "https://www.reddit.com/api/v1/access_token";
/// Reddit's authenticated API origin; tests inject a loopback one.
const DEFAULT_API_ORIGIN: &str = "https://oauth.reddit.com";
/// Tokens are refreshed this long before their declared expiry.
const EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// A user's own Reddit API app — the BYO credentials plus the endpoints
/// (overridable so the whole flow is hermetically testable on loopback).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedditApiConfig {
    /// The app's client id (from reddit.com/prefs/apps).
    pub client_id: String,
    /// The app's client secret. Held in memory only; at rest it lives in
    /// the OS keychain (see the core `secrets` module).
    pub client_secret: String,
    /// The OAuth token endpoint.
    pub token_url: String,
    /// The origin authenticated API calls are made against.
    pub api_origin: String,
}

impl RedditApiConfig {
    /// Credentials against Reddit's real endpoints.
    #[must_use]
    pub fn new(client_id: impl Into<String>, client_secret: impl Into<String>) -> Self {
        Self {
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            token_url: DEFAULT_TOKEN_URL.to_owned(),
            api_origin: DEFAULT_API_ORIGIN.to_owned(),
        }
    }
}

#[derive(Debug)]
struct CachedToken {
    bearer: String,
    expires_at: Instant,
}

#[derive(Debug, Default)]
struct TokenState {
    config: Option<RedditApiConfig>,
    token: Option<CachedToken>,
}

/// Owns the runtime credentials and the cached bearer token. Absent
/// credentials mean "unauthenticated" — every caller falls back to the
/// public endpoints at the slow pacing.
#[derive(Debug, Default)]
pub struct TokenManager {
    state: Mutex<TokenState>,
}

impl TokenManager {
    /// Installs (or clears) the credentials; any cached token is dropped.
    pub fn set_config(&self, config: Option<RedditApiConfig>) {
        let mut state = self.lock();
        state.config = config;
        state.token = None;
    }

    /// Whether credentials are installed.
    pub fn configured(&self) -> bool {
        self.lock().config.is_some()
    }

    /// The configured client id (never the secret) — for status surfaces.
    pub fn client_id(&self) -> Option<String> {
        self.lock().config.as_ref().map(|c| c.client_id.clone())
    }

    /// The authenticated API origin, when configured.
    pub fn api_origin(&self) -> Option<String> {
        self.lock().config.as_ref().map(|c| c.api_origin.clone())
    }

    /// A valid bearer token: the cached one when fresh, else a new one
    /// from the token endpoint. `Ok(None)` when no credentials are
    /// installed (the unauthenticated path).
    ///
    /// # Errors
    ///
    /// [`EnrichError`] when the grant fails — including `Http { 429 }`
    /// with the endpoint's `Retry-After` when the token endpoint itself
    /// rate-limits.
    pub async fn bearer(&self, client: &PolicedClient) -> Result<Option<String>, EnrichError> {
        let config = {
            let state = self.lock();
            match (&state.config, &state.token) {
                (None, _) => return Ok(None),
                (Some(_), Some(token)) if token.expires_at > Instant::now() => {
                    return Ok(Some(token.bearer.clone()));
                }
                (Some(config), _) => config.clone(),
            }
        };
        // The grant runs OUTSIDE the lock (it awaits the network).
        // Concurrent callers may race to fetch — harmless: both tokens
        // are valid, the later write wins the cache.
        let response = client
            .post_form(
                &config.token_url,
                (&config.client_id, &config.client_secret),
                &[("grant_type", "client_credentials")],
            )
            .await?;
        if !response.is_success() {
            return Err(EnrichError::Http {
                status: response.status,
                retry_after: response.retry_after,
            });
        }
        let payload: Value = serde_json::from_slice(&response.body)
            .map_err(|err| EnrichError::Malformed(format!("token payload: {err}")))?;
        let bearer = payload
            .get("access_token")
            .and_then(Value::as_str)
            .filter(|token| !token.is_empty())
            .ok_or_else(|| EnrichError::Malformed("token payload: no access_token".to_owned()))?
            .to_owned();
        let expires_in = payload
            .get("expires_in")
            .and_then(Value::as_u64)
            .unwrap_or(3600);
        let expires_at =
            Instant::now() + Duration::from_secs(expires_in).saturating_sub(EXPIRY_MARGIN);
        self.lock().token = Some(CachedToken {
            bearer: bearer.clone(),
            expires_at,
        });
        Ok(Some(bearer))
    }

    /// Drops the cached token — a 401 on an API call means it went stale
    /// (or the credentials were revoked); the next call re-grants.
    pub fn invalidate(&self) {
        self.lock().token = None;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TokenState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// The authenticated JSON endpoint for a reddit post permalink: the
/// permalink path on `api_origin` with `.json?raw_json=1`. `None` when
/// the URL is not a post permalink, or when its host is neither
/// reddit-owned nor the configured origin itself (a reddit-shaped path
/// on some unrelated site must not leak to oauth.reddit.com).
#[must_use]
pub fn authed_json_endpoint(api_origin: &str, source_url: &str) -> Option<String> {
    let url = Url::parse(source_url).ok()?;
    let origin = Url::parse(api_origin).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let is_reddit = host == "reddit.com" || host.ends_with(".reddit.com");
    let same_origin = url.host_str() == origin.host_str()
        && url.port_or_known_default() == origin.port_or_known_default();
    if !is_reddit && !same_origin {
        return None;
    }
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    if segments.len() < 4 || segments[0] != "r" || segments[2] != "comments" {
        return None;
    }
    let path = url.path().trim_end_matches('/');
    Some(format!(
        "{}{path}.json?raw_json=1",
        api_origin.trim_end_matches('/'),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_authed_endpoint_moves_the_permalink_onto_the_api_origin() {
        assert_eq!(
            authed_json_endpoint(
                "https://oauth.reddit.com",
                "https://www.reddit.com/r/rust/comments/abc123/title/",
            )
            .unwrap(),
            "https://oauth.reddit.com/r/rust/comments/abc123/title.json?raw_json=1"
        );
    }

    #[test]
    fn a_reddit_shaped_path_on_an_unrelated_host_never_leaks_to_the_api() {
        assert_eq!(
            authed_json_endpoint(
                "https://oauth.reddit.com",
                "https://blog.example.com/r/rust/comments/abc123/title/",
            ),
            None
        );
        // …but the loopback fixture pattern (source host == api origin)
        // stays testable.
        assert!(
            authed_json_endpoint(
                "http://127.0.0.1:8080",
                "http://127.0.0.1:8080/r/rust/comments/abc123/title/",
            )
            .is_some()
        );
    }

    #[test]
    fn an_unconfigured_manager_reports_unauthenticated() {
        let manager = TokenManager::default();
        assert!(!manager.configured());
        assert!(manager.client_id().is_none());
        assert!(manager.api_origin().is_none());
    }
}
