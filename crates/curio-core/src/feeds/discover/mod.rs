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
//! 3. **Fetch the page once.** If it *is* a feed, that URL is the answer
//!    (its redirect target only when every hop was permanent).
//!    Otherwise its `<link rel="alternate">` feeds (exact type allowlist,
//!    comment feeds dropped beside a primary feed, deduped) — or a
//!    synthesized `YouTube` channel feed — are validated, at most
//!    [`MAX_DECLARED`] of them.
//! 4. **Probe** only when nothing declared validates: same-origin
//!    feed-shaped `<a href>`s, then the common paths (`/feed`, `/feed.xml`,
//!    `/index.xml`, `/rss.xml`, `/atom.xml`, `/rss`) — never more than
//!    [`MAX_PROBES`] HTTP requests in total (redirect hops count), never off
//!    the page's origin (an off-origin redirect is refused unrequested),
//!    first hit wins.
//!
//! The whole run shares one wall-clock budget ([`DEFAULT_DEADLINE`] unless
//! [`DiscoverOptions::deadline`] says otherwise): once it is spent nothing
//! more is fetched and the feeds validated so far are the answer.
//!
//! **Validated** means fetched (2xx), sniffed as a feed root and parsed by
//! [`crate::feeds::parse_fetched_feed`] — no candidate is offered that a refresh
//! would fail to parse. Candidates are deduped by requested and final URL.
//!
//! Favicon precedence: a declared `<link rel="…icon">` (resolved absolute)
//! wins; otherwise the same-origin `/favicon.ico`. No third-party favicon
//! service is ever contacted here — that fallback is a head's opt-in.

mod html;
mod rewrite;
mod sniff;

use std::collections::HashSet;
use std::time::Duration;

use tokio::time::Instant;
use url::Url;

use crate::feeds::parse_fetched_feed;
use crate::fetch::decode::decode_html;
use crate::fetch::{FetchError, FetchRequest, FetchResponse, PolicedClient};

/// Most declared (`<link rel="alternate">`) candidates validated per page —
/// a page listing dozens of category feeds costs a bounded number of
/// requests.
pub const MAX_DECLARED: usize = 8;

/// Most same-origin probe HTTP requests per discovery (anchors + common
/// paths) — redirect hops count against it.
pub const MAX_PROBES: u32 = 6;

/// The wall-clock budget of one discovery when
/// [`DiscoverOptions::deadline`] is unset.
pub const DEFAULT_DEADLINE: Duration = Duration::from_secs(45);

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
    /// Overall wall-clock budget; `None` means [`DEFAULT_DEADLINE`]. Once
    /// it is spent nothing more is fetched: the feeds validated so far are
    /// returned, or [`DiscoverError::TimedOut`] when the page itself had
    /// not answered yet.
    pub deadline: Option<Duration>,
}

/// Why discovery could not run, or why [`resolve`] could not settle on a
/// feed. Only [`discover`] treats finding *no* feed as success (an empty
/// [`Discovery::feeds`]); [`resolve`] reports it as [`DiscoverError::NoFeed`].
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
    /// The time budget ran out before the page answered.
    #[error("gave up on {url}: it did not answer within the discovery time budget")]
    TimedOut {
        /// The page URL.
        url: String,
    },
    /// The page answered with a non-success status.
    #[error("{url} answered with HTTP {status}")]
    Status {
        /// The page URL.
        url: String,
        /// The HTTP status.
        status: u16,
    },
    /// The page answered, but neither it nor anything it declares or the
    /// probes found is a feed — only [`resolve`] reports this ([`discover`]
    /// answers an empty [`Discovery::feeds`] instead).
    #[error("no feed found at {input}")]
    NoFeed {
        /// The input, trimmed.
        input: String,
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

/// What a subscribe resolves typed input to: the feed URL to subscribe
/// plus the other validated candidates discovery offered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The best validated feed URL — the one to subscribe to.
    pub url: String,
    /// The other validated candidates, best first.
    pub alternates: Vec<String>,
}

/// Resolves what a user typed to the one feed URL a subscribe should store:
/// [`discover`], then its best validated candidate. This is the validating
/// subscribe path every head shares — a URL that resolves here is a feed a
/// refresh can parse. A head's "subscribe anyway" escape hatch uses
/// [`normalize_input`] instead.
///
/// # Errors
///
/// Every [`discover`] error, plus [`DiscoverError::NoFeed`] when the page
/// answered but no feed validated.
pub async fn resolve(
    client: &PolicedClient,
    input: &str,
    options: DiscoverOptions,
) -> Result<Resolution, DiscoverError> {
    let mut urls = discover(client, input, options)
        .await?
        .feeds
        .into_iter()
        .map(|feed| feed.url);
    let url = urls.next().ok_or_else(|| DiscoverError::NoFeed {
        input: input.trim().to_owned(),
    })?;
    Ok(Resolution {
        url,
        alternates: urls.collect(),
    })
}

/// Normalizes what a user typed into an absolute http(s) URL: trims it,
/// rewrites the legacy `feed:` scheme (`feed://host/x`, `feed:https://…`)
/// and gives a scheme-less input (`example.test`, `//example.test`)
/// `https://`.
///
/// # Errors
///
/// [`DiscoverError::Empty`] for blank input; [`DiscoverError::InvalidUrl`]
/// when the result is not an http(s) URL with a host, or carries userinfo
/// (`user:pass@host`, which the policed client always refuses — and which
/// is what `mailto:a@host` turns into once given a scheme).
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
    let has_userinfo = !url.username().is_empty() || url.password().is_some();
    if matches!(url.scheme(), "http" | "https") && has_host && !has_userinfo {
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
        deadline: Instant::now() + options.deadline.unwrap_or(DEFAULT_DEADLINE),
        expired: false,
    };

    // 2. Fetch-free rewrites.
    let mut feeds = Vec::new();
    for candidate in rewrite::rewrites(&url) {
        let request = validator.request(&candidate);
        if let (Some(feed), _) = validator.validate(&request, None).await {
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
    let response = match validator.fetch(&validator.request(&page)).await {
        None => return Err(DiscoverError::TimedOut { url: page }),
        Some(Err(source)) => return Err(DiscoverError::Fetch { url: page, source }),
        Some(Ok(response)) => response,
    };
    if !response.is_success() {
        return Err(DiscoverError::Status {
            url: page,
            status: response.status,
        });
    }
    validator.seen.insert(page.clone());
    validator.seen.insert(response.final_url.clone());
    let extracted = html::extract(
        &response.body,
        response.content_type.as_deref(),
        &response.final_url,
    );
    if extracted.is_feed {
        if let Ok(parsed) = parse_fetched_feed(&response) {
            return Ok(Discovery {
                feeds: vec![DiscoveredFeed {
                    url: subscribable_url(&page, &response),
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
        let request = validator.request(&candidate.url);
        if let (Some(feed), _) = validator.validate(&request, candidate.title).await {
            feeds.push(feed);
        }
    }

    // 4. Same-origin probing, only when nothing declared validated.
    if feeds.is_empty()
        && let Ok(base) = Url::parse(&response.final_url)
    {
        let common = COMMON_PATHS
            .iter()
            .filter_map(|path| base.join(path).ok().map(String::from));
        let mut probes: Vec<String> = Vec::new();
        for probe in extracted.anchors.into_iter().chain(common) {
            if !probes.contains(&probe) && !validator.seen.contains(&probe) {
                probes.push(probe);
            }
        }
        // The budget is in HTTP requests: a probe may follow only as many
        // (same-origin) redirect hops as the budget has left.
        let mut budget = MAX_PROBES;
        for probe in probes {
            if budget == 0 || validator.expired {
                break;
            }
            let request = FetchRequest {
                same_origin_redirects: true,
                max_redirects: Some(budget - 1),
                ..validator.request(&probe)
            };
            let (feed, spent) = validator.validate(&request, None).await;
            budget = budget.saturating_sub(spent);
            if let Some(feed) = feed {
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

/// The URL to subscribe for a fetch of `requested`: the redirect target
/// only when every hop was permanent (as refresh adopts it) — a temporary
/// redirect (tracking link, signed CDN URL) is never stored.
fn subscribable_url(requested: &str, response: &FetchResponse) -> String {
    if response.permanent_redirect {
        response.final_url.clone()
    } else {
        requested.to_owned()
    }
}

/// Fetch-and-parse validation with cross-candidate dedupe, all under the
/// discovery's deadline.
struct Validator<'a> {
    client: &'a PolicedClient,
    options: DiscoverOptions,
    /// Every URL already requested or landed on.
    seen: HashSet<String>,
    /// When the discovery's time budget runs out.
    deadline: Instant,
    /// The deadline has passed: nothing more is fetched.
    expired: bool,
}

impl Validator<'_> {
    fn request(&self, url: &str) -> FetchRequest {
        FetchRequest {
            allow_private_network: self.options.allow_private_network,
            ..FetchRequest::new(url)
        }
    }

    /// One policed fetch bounded by the deadline — `None` once it has
    /// passed (and for every fetch after).
    async fn fetch(&mut self, request: &FetchRequest) -> Option<Result<FetchResponse, FetchError>> {
        if self.expired {
            return None;
        }
        let result = tokio::time::timeout_at(self.deadline, self.client.fetch(request)).await;
        self.expired = result.is_err();
        result.ok()
    }

    /// An upper bound on the HTTP requests one fetch of `request` sent:
    /// exact on success and on the redirect refusals, otherwise the most
    /// it could have sent (the first request plus every allowed hop).
    fn requests_spent(
        &self,
        request: &FetchRequest,
        result: &Result<FetchResponse, FetchError>,
    ) -> u32 {
        match result {
            Ok(response) => 1 + response.redirects,
            Err(FetchError::CrossOriginRedirect { hops, .. }) => 1 + hops,
            Err(FetchError::TooManyRedirects { limit, .. }) => 1 + limit,
            Err(_) => {
                let cap = self.client.config().max_redirects;
                1 + request.max_redirects.map_or(cap, |own| own.min(cap))
            }
        }
    }

    /// Fetches `request` and keeps it iff it is a parseable feed not
    /// already found under another URL; also returns how many HTTP
    /// requests that cost (an upper bound, see [`Self::requests_spent`]).
    async fn validate(
        &mut self,
        request: &FetchRequest,
        title: Option<String>,
    ) -> (Option<DiscoveredFeed>, u32) {
        let url = request.url.as_str();
        if !self.seen.insert(url.to_owned()) {
            return (None, 0);
        }
        let Some(result) = self.fetch(request).await else {
            return (None, 0);
        };
        let spent = self.requests_spent(request, &result);
        let response = match result {
            Ok(response) if response.is_success() => response,
            Ok(response) => {
                tracing::debug!(
                    url,
                    status = response.status,
                    "discovery candidate rejected"
                );
                return (None, spent);
            }
            Err(error) => {
                tracing::debug!(url, %error, "discovery candidate unreachable");
                return (None, spent);
            }
        };
        if response.final_url != url && !self.seen.insert(response.final_url.clone()) {
            return (None, spent); // a redirect twin of a feed already found
        }
        // Only the sniff window is decoded — never a copy of the whole body —
        // by its BOM and HTTP charset, so a UTF-16 feed still sniffs.
        let head = &response.body[..response.body.len().min(sniff::SNIFF_WINDOW)];
        if !sniff::looks_like_feed(&decode_html(head, response.content_type.as_deref())) {
            return (None, spent);
        }
        let Ok(parsed) = parse_fetched_feed(&response) else {
            return (None, spent);
        };
        let feed = DiscoveredFeed {
            url: subscribable_url(url, &response),
            title: parsed.meta.title.or(title),
        };
        (Some(feed), spent)
    }
}

#[cfg(test)]
mod tests;
