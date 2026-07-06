//! Feed autodiscovery + favicon resolution — head-local platform policy
//! (network + HTML scan), fetched through curio-core's policed client so a
//! bare reqwest client never appears in the tree.
//!
//! Given a URL the user typed, [`Discovery::discover`] fetches it once and:
//!
//! - if the bytes are already a feed (`<rss`/`<feed`/`<rdf`/JSON-feed
//!   marker), returns that URL as the sole candidate;
//! - otherwise parses the HTML for `<link rel="alternate" …>` feed links
//!   (RSS/Atom/JSON) and resolves each `href` against the final URL.
//!
//! Favicon precedence: a declared `<link rel="…icon">` (resolved absolute)
//! wins; otherwise the same-origin `/favicon.ico`. The Google favicon
//! service is **never** contacted here — that fallback is opt-in and lives
//! in the frontend, so nothing leaves the machine toward a third party
//! unless the user turns it on (PRIVACY.md).

use curio_core::fetch::{FetchRequest, PolicedClient};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::error::{CommandError, ErrorCode};

/// One discovered feed candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiscoveredFeedDto {
    /// Absolute feed URL (resolved against the fetched page).
    pub url: String,
    /// The `title` the page gave the `<link>`, if any.
    pub title: Option<String>,
}

/// What one autodiscovery fetch turned up: zero or more feed candidates
/// plus a best-guess favicon URL (same-origin only — the Google fallback
/// is the frontend's opt-in concern).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiscoveryDto {
    /// Feed candidates, in document order (the page URL itself if it *is* a
    /// feed).
    pub feeds: Vec<DiscoveredFeedDto>,
    /// A favicon URL to preview (declared icon or `/favicon.ico`), if the
    /// final URL was absolute enough to resolve one.
    pub favicon: Option<String>,
}

/// The autodiscovery client, managed as Tauri state. Holds its own policed
/// client (same policy config as the image cache); construction mirrors
/// [`crate::image_cache::ImageCache`].
#[derive(Debug)]
pub struct Discovery {
    client: PolicedClient,
}

impl Discovery {
    /// A discovery service fetching through `client`.
    #[must_use]
    pub fn new(client: PolicedClient) -> Self {
        Self { client }
    }

    /// Fetches `url` once and extracts feed candidates + a favicon.
    ///
    /// # Errors
    ///
    /// User-tier network errors from the policed fetch (SSRF-guarded,
    /// size-capped); an invalid/empty URL is a user input error.
    pub async fn discover(&self, url: &str) -> Result<DiscoveryDto, CommandError> {
        let trimmed = url.trim();
        if trimmed.is_empty() {
            return Err(CommandError::invalid_input("enter a URL to look up"));
        }
        let response = self
            .client
            .fetch(&FetchRequest::new(trimmed))
            .await
            .map_err(|error| {
                CommandError::user(
                    ErrorCode::Network,
                    format!("could not reach {trimmed}: {error}"),
                    true,
                )
            })?;
        if !response.is_success() {
            return Err(CommandError::user(
                ErrorCode::Network,
                format!("{trimmed} answered with HTTP {}", response.status),
                true,
            ));
        }
        // Extraction is fully synchronous (no `Html` value crosses an await
        // — `scraper::Html` is not `Send`), so the returned future stays
        // `Send` for `spawn`.
        Ok(extract(&response.body, &response.final_url))
    }
}

/// Pulls feed candidates + favicon out of one fetched document, resolving
/// every relative URL against `final_url`.
fn extract(body: &[u8], final_url: &str) -> DiscoveryDto {
    let text = String::from_utf8_lossy(body);
    let base = Url::parse(final_url).ok();

    if looks_like_feed(&text) {
        return DiscoveryDto {
            feeds: vec![DiscoveredFeedDto {
                url: final_url.to_owned(),
                title: None,
            }],
            favicon: base.as_ref().and_then(favicon_ico),
        };
    }

    let document = Html::parse_document(&text);
    DiscoveryDto {
        feeds: feed_links(&document, base.as_ref()),
        favicon: favicon(&document, base.as_ref()),
    }
}

/// A quick structural sniff: does the head of the body look like a feed
/// rather than an HTML page? (Content-Type is not exposed by the policed
/// response, so we inspect the bytes.)
fn looks_like_feed(text: &str) -> bool {
    // Look only at the leading window — feeds declare their root element
    // early; scanning the whole body would misfire on HTML that quotes a
    // feed snippet far down the page.
    let head = &text[..text.len().min(2048)].to_ascii_lowercase();
    let trimmed = head.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    // XML prolog, then a feed root; or a bare feed root; or a JSON feed.
    trimmed.contains("<rss")
        || trimmed.contains("<feed")
        || trimmed.contains("<rdf:rdf")
        || trimmed.contains("\"version\":\"https://jsonfeed.org")
        || trimmed.contains("\"version\": \"https://jsonfeed.org")
}

/// `<link rel="alternate" type="…rss/atom/xml/json…" href="…">` candidates.
fn feed_links(document: &Html, base: Option<&Url>) -> Vec<DiscoveredFeedDto> {
    // `rel~=alternate` = whitespace-separated token match (handles both
    // `rel="alternate"` and `rel="alternate home"`).
    let Ok(selector) = Selector::parse("link[rel~=alternate][href]") else {
        return Vec::new();
    };
    let mut feeds = Vec::new();
    for element in document.select(&selector) {
        let value = element.value();
        let feed_type = value.attr("type").unwrap_or_default().to_ascii_lowercase();
        if !is_feed_type(&feed_type) {
            continue;
        }
        let Some(href) = value.attr("href") else {
            continue;
        };
        if let Some(resolved) = resolve(base, href) {
            feeds.push(DiscoveredFeedDto {
                url: resolved,
                title: value.attr("title").map(str::to_owned),
            });
        }
    }
    feeds
}

fn is_feed_type(feed_type: &str) -> bool {
    feed_type.contains("rss")
        || feed_type.contains("atom")
        || (feed_type.contains("xml") && !feed_type.contains("xhtml"))
        || feed_type.contains("json")
}

/// The best favicon URL: a declared `<link rel="…icon">` if present,
/// else the same-origin `/favicon.ico`.
fn favicon(document: &Html, base: Option<&Url>) -> Option<String> {
    if let Ok(selector) = Selector::parse("link[href]") {
        for element in document.select(&selector) {
            let value = element.value();
            let rel = value.attr("rel").unwrap_or_default().to_ascii_lowercase();
            // Matches "icon", "shortcut icon", "apple-touch-icon".
            if rel.contains("icon")
                && let Some(href) = value.attr("href")
                && let Some(resolved) = resolve(base, href)
            {
                return Some(resolved);
            }
        }
    }
    base.and_then(favicon_ico)
}

fn favicon_ico(base: &Url) -> Option<String> {
    base.join("/favicon.ico").ok().map(String::from)
}

/// Resolves `href` (possibly relative or protocol-relative) against the
/// page's final URL; drops non-http(s) results (the frontend only ever
/// loads these through the policed image cache, which is http(s)-only).
fn resolve(base: Option<&Url>, href: &str) -> Option<String> {
    let joined = match base {
        Some(base) => base.join(href).ok()?,
        None => Url::parse(href).ok()?,
    };
    matches!(joined.scheme(), "http" | "https").then(|| joined.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use curio_core::fetch::FetchConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn discovery() -> Discovery {
        Discovery::new(PolicedClient::new(FetchConfig {
            politeness_delay: Duration::ZERO,
            trusted_addrs: HashSet::from([IpAddr::V4(Ipv4Addr::LOCALHOST)]),
            ..FetchConfig::default()
        }))
    }

    #[test]
    fn extracts_alternate_feed_links_and_resolves_relative_hrefs() {
        let html = r#"<html><head>
            <link rel="alternate" type="application/rss+xml" title="RSS" href="/feed.xml">
            <link rel="alternate" type="application/atom+xml" href="https://cdn.test/atom">
            <link rel="stylesheet" href="/style.css">
            <link rel="icon" href="/static/favicon.png">
        </head><body>hi</body></html>"#;
        let out = extract(html.as_bytes(), "https://blog.test/index.html");
        assert_eq!(
            out.feeds,
            vec![
                DiscoveredFeedDto {
                    url: "https://blog.test/feed.xml".into(),
                    title: Some("RSS".into()),
                },
                DiscoveredFeedDto {
                    url: "https://cdn.test/atom".into(),
                    title: None,
                },
            ]
        );
        assert_eq!(
            out.favicon.as_deref(),
            Some("https://blog.test/static/favicon.png")
        );
    }

    #[test]
    fn a_url_that_is_already_a_feed_returns_itself_with_ico_favicon() {
        let feed = r#"<?xml version="1.0"?><rss version="2.0"><channel></channel></rss>"#;
        let out = extract(feed.as_bytes(), "https://blog.test/feed.xml");
        assert_eq!(out.feeds.len(), 1);
        assert_eq!(out.feeds[0].url, "https://blog.test/feed.xml");
        assert_eq!(out.feeds[0].title, None);
        assert_eq!(
            out.favicon.as_deref(),
            Some("https://blog.test/favicon.ico")
        );
    }

    #[test]
    fn a_plain_page_with_no_links_falls_back_to_the_origin_favicon() {
        let html = "<html><head><title>Nothing here</title></head><body></body></html>";
        let out = extract(html.as_bytes(), "https://blog.test/some/deep/page");
        assert!(out.feeds.is_empty());
        assert_eq!(
            out.favicon.as_deref(),
            Some("https://blog.test/favicon.ico")
        );
    }

    #[test]
    fn non_http_feed_hrefs_are_dropped() {
        let html = r#"<link rel="alternate" type="application/rss+xml" href="javascript:alert(1)">
                      <link rel="alternate" type="application/rss+xml" href="feed://blog.test/x">"#;
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert!(out.feeds.is_empty(), "only http(s) feed URLs survive");
    }

    #[test]
    fn xhtml_alternates_are_not_mistaken_for_feeds() {
        let html = r#"<link rel="alternate" type="application/xhtml+xml" href="/amp">"#;
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert!(out.feeds.is_empty());
    }

    #[tokio::test]
    async fn discover_fetches_through_the_policed_client() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><head>
                   <link rel="alternate" type="application/rss+xml" href="/feed.xml">
                   </head></html>"#,
            ))
            .mount(&server)
            .await;

        let out = discovery()
            .discover(&format!("{}/", server.uri()))
            .await
            .unwrap();
        assert_eq!(out.feeds.len(), 1);
        assert!(out.feeds[0].url.ends_with("/feed.xml"));
    }

    #[tokio::test]
    async fn an_empty_url_is_a_user_error() {
        let error = discovery().discover("   ").await.unwrap_err();
        assert_eq!(error.kind, crate::error::ErrorKind::User);
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn a_non_success_status_surfaces_as_a_network_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let error = discovery()
            .discover(&format!("{}/", server.uri()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Network);
    }
}
