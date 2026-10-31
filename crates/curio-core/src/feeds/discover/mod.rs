//! Feed autodiscovery: from whatever the user typed to validated feed URLs.
//!
//! [`discover`] runs, through the policed client only:
//!
//! 1. **Normalize** the input ([`normalize_input`]): trim, scheme-less
//!    `example.com` → `https://example.com/`, legacy `feed:` → `https:`.
//! 2. **Rewrite** fetch-free: reddit `/r/x`, GitHub repos, `YouTube`
//!    playlists/channels, Mastodon `@user` and Substack publications map to
//!    their feed URL without scraping the page. A rewrite that validates
//!    ends discovery.
//! 3. **Fetch the page once.** If it *is* a feed, that URL is the answer.
//!    Otherwise its `<link rel="alternate">` feeds (exact type allowlist,
//!    comment feeds dropped beside a primary feed, deduped) — or a
//!    synthesized `YouTube` channel feed — are validated, at most
//!    [`MAX_DECLARED`] of them.
//! 4. **Probe** only when nothing declared validates: same-origin
//!    feed-shaped `<a href>`s, then the common paths (`/feed`, `/feed.xml`,
//!    `/index.xml`, `/rss.xml`, `/atom.xml`, `/rss`) — never more than
//!    [`MAX_PROBES`] requests, never off the page's origin, first hit wins.
//!
//! **Validated** means fetched (2xx), sniffed as a feed root and parsed by
//! [`crate::feeds::parse_feed`] — no candidate is offered that a refresh
//! would fail to parse. Candidates are deduped by requested and final URL.
//!
//! Favicon precedence: a declared `<link rel="…icon">` (resolved absolute)
//! wins; otherwise the same-origin `/favicon.ico`. No third-party favicon
//! service is ever contacted here — that fallback is a head's opt-in.

mod html;
mod rewrite;
mod sniff;

use std::collections::HashSet;

use url::{Origin, Url};

use crate::feeds::parse_feed;
use crate::fetch::{FetchError, FetchRequest, PolicedClient};

/// Most declared (`<link rel="alternate">`) candidates validated per page —
/// a page listing dozens of category feeds costs a bounded number of
/// requests.
pub const MAX_DECLARED: usize = 8;

/// Most same-origin probe requests per discovery (anchors + common paths).
pub const MAX_PROBES: usize = 6;

/// Common feed paths, most popular generator first (`WordPress`, Jekyll,
/// Hugo, then the generic names).
const COMMON_PATHS: [&str; 6] = [
    "/feed",
    "/feed.xml",
    "/index.xml",
    "/rss.xml",
    "/atom.xml",
    "/rss",
];

/// One validated feed candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredFeed {
    /// Absolute feed URL — the one to subscribe to.
    pub url: String,
    /// The feed's own title (from parsing it), else the title the page gave
    /// its `<link>`.
    pub title: Option<String>,
}

/// What one discovery turned up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    /// Validated feed candidates, best first (empty: nothing found).
    pub feeds: Vec<DiscoveredFeed>,
    /// A favicon URL to preview (declared icon or same-origin
    /// `/favicon.ico`).
    pub favicon: Option<String>,
}

/// Per-discovery policy.
#[derive(Debug, Clone, Copy, Default)]
pub struct DiscoverOptions {
    /// Contract W1: exempt these fetches from the private-network guard —
    /// set only from explicit configuration (a CLI flag), never content.
    pub allow_private_network: bool,
}

/// Why discovery could not run (finding *no* feed is not an error — it is
/// an empty [`Discovery::feeds`]).
#[derive(Debug, thiserror::Error)]
pub enum DiscoverError {
    /// The input was blank.
    #[error("enter a URL to look up")]
    Empty,
    /// The input is not an http(s) URL, even after normalization.
    #[error("not a fetchable http(s) URL: {input}")]
    InvalidUrl {
        /// The rejected input, trimmed.
        input: String,
    },
    /// The page could not be fetched (network, policy, size cap).
    #[error("could not reach {url}: {source}")]
    Fetch {
        /// The page URL.
        url: String,
        /// The policed-client failure.
        #[source]
        source: FetchError,
    },
    /// The page answered with a non-success status.
    #[error("{url} answered with HTTP {status}")]
    Status {
        /// The page URL.
        url: String,
        /// The HTTP status.
        status: u16,
    },
}

impl DiscoverError {
    /// Whether the input itself was at fault (blank / not a URL), as
    /// opposed to the network.
    #[must_use]
    pub fn is_input_error(&self) -> bool {
        matches!(self, Self::Empty | Self::InvalidUrl { .. })
    }
}

/// Normalizes what a user typed into an absolute http(s) URL: trims it,
/// rewrites the legacy `feed:` scheme (`feed://host/x`, `feed:https://…`)
/// and gives a scheme-less input (`example.test`, `//example.test`)
/// `https://`.
///
/// # Errors
///
/// [`DiscoverError::Empty`] for blank input; [`DiscoverError::InvalidUrl`]
/// when the result is not an http(s) URL with a host.
pub fn normalize_input(input: &str) -> Result<Url, DiscoverError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(DiscoverError::Empty);
    }
    let invalid = || DiscoverError::InvalidUrl {
        input: trimmed.to_owned(),
    };
    // `feed://host/x` and `feed:https://host/x` both mean the URL after it.
    let unfeed = match trimmed.get(..5) {
        Some(prefix) if prefix.eq_ignore_ascii_case("feed:") => &trimmed[5..],
        _ => trimmed,
    };
    let absolute = if has_scheme(unfeed) {
        unfeed.to_owned()
    } else {
        format!("https://{}", unfeed.trim_start_matches('/'))
    };
    let url = Url::parse(&absolute).map_err(|_| invalid())?;
    let has_host = url.host_str().is_some_and(|host| !host.is_empty());
    if matches!(url.scheme(), "http" | "https") && has_host {
        Ok(url)
    } else {
        Err(invalid())
    }
}

/// Whether `input` starts with an RFC 3986 scheme followed by `://`
/// (`https://…`, `ftp://…`) — not merely contains one (`host/?u=http://x`).
fn has_scheme(input: &str) -> bool {
    input.split_once("://").is_some_and(|(scheme, _)| {
        let mut chars = scheme.chars();
        chars.next().is_some_and(|c| c.is_ascii_alphabetic())
            && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
    })
}

/// Discovers the feeds behind `input` (see the module docs for the order).
///
/// # Errors
///
/// [`DiscoverError`] when the input is not a URL, or when no rewrite
/// validated and the page itself could not be fetched. Finding no feed is
/// `Ok` with empty `feeds`.
pub async fn discover(
    client: &PolicedClient,
    input: &str,
    options: DiscoverOptions,
) -> Result<Discovery, DiscoverError> {
    let url = normalize_input(input)?;
    let mut validator = Validator {
        client,
        options,
        seen: HashSet::new(),
    };

    // 2. Fetch-free rewrites.
    let mut feeds = Vec::new();
    for candidate in rewrite::rewrites(&url) {
        if let Some(feed) = validator.validate(&candidate, None, None).await {
            feeds.push(feed);
        }
    }
    if !feeds.is_empty() {
        return Ok(Discovery {
            feeds,
            favicon: html::favicon_ico(&url),
        });
    }

    // 3. The page itself.
    let page = String::from(url);
    let response = client
        .fetch(&validator.request(&page))
        .await
        .map_err(|source| DiscoverError::Fetch {
            url: page.clone(),
            source,
        })?;
    if !response.is_success() {
        return Err(DiscoverError::Status {
            url: page,
            status: response.status,
        });
    }
    validator.seen.insert(page);
    validator.seen.insert(response.final_url.clone());
    let extracted = html::extract(&response.body, &response.final_url);
    if extracted.is_feed {
        if let Ok(parsed) = parse_feed(&response.body, Some(&response.final_url)) {
            return Ok(Discovery {
                feeds: vec![DiscoveredFeed {
                    url: response.final_url,
                    title: parsed.meta.title,
                }],
                favicon: extracted.favicon,
            });
        }
        // Sniffed like a feed but does not parse: nothing to offer.
        return Ok(Discovery {
            feeds: Vec::new(),
            favicon: extracted.favicon,
        });
    }
    for candidate in extracted.feeds.into_iter().take(MAX_DECLARED) {
        if let Some(feed) = validator
            .validate(&candidate.url, candidate.title, None)
            .await
        {
            feeds.push(feed);
        }
    }

    // 4. Same-origin probing, only when nothing declared validated.
    if feeds.is_empty()
        && let Ok(base) = Url::parse(&response.final_url)
    {
        let origin = base.origin();
        let common = COMMON_PATHS
            .iter()
            .filter_map(|path| base.join(path).ok().map(String::from));
        let mut probes: Vec<String> = Vec::new();
        for probe in extracted.anchors.into_iter().chain(common) {
            if !probes.contains(&probe) && !validator.seen.contains(&probe) {
                probes.push(probe);
            }
        }
        for probe in probes.into_iter().take(MAX_PROBES) {
            if let Some(feed) = validator.validate(&probe, None, Some(&origin)).await {
                feeds.push(feed);
                break;
            }
        }
    }
    Ok(Discovery {
        feeds,
        favicon: extracted.favicon,
    })
}

/// Fetch-and-parse validation with cross-candidate dedupe.
struct Validator<'a> {
    client: &'a PolicedClient,
    options: DiscoverOptions,
    /// Every URL already requested or landed on.
    seen: HashSet<String>,
}

impl Validator<'_> {
    fn request(&self, url: &str) -> FetchRequest {
        FetchRequest {
            allow_private_network: self.options.allow_private_network,
            ..FetchRequest::new(url)
        }
    }

    /// Fetches `url` and keeps it iff it is a parseable feed not already
    /// found under another URL. With `origin`, a response that redirected
    /// off that origin is discarded (probes stay same-origin).
    async fn validate(
        &mut self,
        url: &str,
        title: Option<String>,
        origin: Option<&Origin>,
    ) -> Option<DiscoveredFeed> {
        if !self.seen.insert(url.to_owned()) {
            return None;
        }
        let response = match self.client.fetch(&self.request(url)).await {
            Ok(response) if response.is_success() => response,
            Ok(response) => {
                tracing::debug!(
                    url,
                    status = response.status,
                    "discovery candidate rejected"
                );
                return None;
            }
            Err(error) => {
                tracing::debug!(url, %error, "discovery candidate unreachable");
                return None;
            }
        };
        if let Some(origin) = origin
            && Url::parse(&response.final_url)
                .map(|landed| landed.origin())
                .ok()
                != Some(origin.clone())
        {
            return None;
        }
        if response.final_url != url && !self.seen.insert(response.final_url.clone()) {
            return None; // a redirect twin of a feed already found
        }
        if !sniff::looks_like_feed(&String::from_utf8_lossy(&response.body)) {
            return None;
        }
        let parsed = parse_feed(&response.body, Some(&response.final_url)).ok()?;
        let url = if response.permanent_redirect {
            response.final_url
        } else {
            url.to_owned()
        };
        Some(DiscoveredFeed {
            url,
            title: parsed.meta.title.or(title),
        })
    }
}

#[cfg(test)]
mod tests;
