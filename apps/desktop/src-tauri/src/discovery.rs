//! Feed autodiscovery + favicon resolution — head-local platform policy
//! (network + HTML scan), fetched through curio-core's policed client so a
//! bare reqwest client never appears in the tree.
//!
//! Given a URL the user typed, [`Discovery::discover`] fetches it once and:
//!
//! - if the bytes are already a feed (an `rss`/`feed`/`rdf:RDF` root
//!   element, or a JSON Feed `version`), returns that URL as the sole
//!   candidate;
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
    let mut feeds = feed_links(&document, base.as_ref());
    // A YouTube channel/handle page usually carries no `<link rel="alternate">`
    // feed — synthesize the videos feed from its canonical channel id so a
    // pasted `youtube.com/@handle` still subscribes. Only when nothing else
    // matched, to avoid a duplicate when the alternate link *is* present.
    if feeds.is_empty()
        && let Some(candidate) = youtube_channel_feed(&document, base.as_ref())
    {
        feeds.push(candidate);
    }
    DiscoveryDto {
        feeds,
        favicon: favicon(&document, base.as_ref()),
    }
}

/// If the page is a `YouTube` channel/handle page, builds its
/// `feeds/videos.xml?channel_id=UC…` feed from the canonical `/channel/UC…`
/// URL the page declares (`<link rel="canonical">` or `<meta og:url>`). No new
/// fetch: the page was already retrieved through the policed client.
fn youtube_channel_feed(document: &Html, base: Option<&Url>) -> Option<DiscoveredFeedDto> {
    let host = base?.host_str()?.to_ascii_lowercase();
    if host != "youtube.com" && !host.ends_with(".youtube.com") {
        return None;
    }
    let channel_id = canonical_youtube_channel_id(document)?;
    Some(DiscoveredFeedDto {
        url: format!("https://www.youtube.com/feeds/videos.xml?channel_id={channel_id}"),
        title: og_title(document),
    })
}

/// The `UC…` channel id from the page's canonical URL / `og:url`, if either
/// points at `/channel/UC…`.
fn canonical_youtube_channel_id(document: &Html) -> Option<String> {
    let sources = [
        ("link[rel~=canonical][href]", "href"),
        (r#"meta[property="og:url"][content]"#, "content"),
    ];
    for (selector, attr) in sources {
        let Ok(parsed) = Selector::parse(selector) else {
            continue;
        };
        for element in document.select(&parsed) {
            if let Some(href) = element.value().attr(attr)
                && let Some(id) = channel_id_from_url(href)
            {
                return Some(id);
            }
        }
    }
    None
}

/// Extracts a `UC…` id from a `.../channel/UC…` URL (24 chars, `UC` + 22).
fn channel_id_from_url(url: &str) -> Option<String> {
    let start = url.find("/channel/")? + "/channel/".len();
    let id: String = url[start..]
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    (id.len() == 24 && id.starts_with("UC")).then_some(id)
}

/// The page's `og:title`, trimmed — a friendlier preview name than the raw
/// page `<title>`. The feed fills in its real title on the first fetch.
fn og_title(document: &Html) -> Option<String> {
    let selector = Selector::parse(r#"meta[property="og:title"][content]"#).ok()?;
    document
        .select(&selector)
        .next()
        .and_then(|element| element.value().attr("content"))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
}

/// How much of the body the feed sniff looks at. Feeds declare their root
/// element (or JSON Feed `version`) early; scanning the whole body would
/// misfire on HTML that quotes a feed snippet far down the page.
const SNIFF_WINDOW: usize = 2048;

/// A quick structural sniff: does the body look like a feed rather than an
/// HTML page? (Content-Type is not exposed by the policed response, so we
/// inspect the bytes.)
///
/// XML: past any BOM, whitespace, XML prolog, processing instructions,
/// comments and `<!DOCTYPE …>`, the *root element* must be `rss`, `feed` or
/// `rdf:RDF` (any namespace prefix, ASCII case-insensitive) — a `<feed`
/// substring elsewhere in an HTML page does not count. JSON: the body is an
/// object declaring a `jsonfeed.org` `version`.
fn looks_like_feed(text: &str) -> bool {
    // Cut on a char boundary: `text` is lossily decoded, so byte
    // `SNIFF_WINDOW` can fall inside a multi-byte char (CJK, emoji, U+FFFD).
    let head = &text[..text.floor_char_boundary(SNIFF_WINDOW)];
    let head = head.trim_start_matches(|c: char| c.is_whitespace() || c == '\u{feff}');
    if head.starts_with('{') {
        return declares_json_feed_version(head);
    }
    root_element_name(head).is_some_and(|name| {
        // `rsplit` always yields at least one piece; the fallback is inert.
        let local = name.rsplit(':').next().unwrap_or(name);
        ["rss", "feed", "rdf"]
            .iter()
            .any(|root| local.eq_ignore_ascii_case(root))
    })
}

/// The name of the first element in `xml`, skipping whitespace, processing
/// instructions (incl. the `<?xml …?>` prolog), comments and a doctype
/// (with or without an internal subset). `None` when the window ends before
/// a root element starts, or the head is not markup at all.
fn root_element_name(xml: &str) -> Option<&str> {
    let mut rest = xml;
    loop {
        rest = rest.trim_start();
        if let Some(after) = rest.strip_prefix("<?") {
            rest = &after[after.find("?>")? + "?>".len()..];
        } else if let Some(after) = rest.strip_prefix("<!--") {
            rest = &after[after.find("-->")? + "-->".len()..];
        } else if let Some(after) = rest.strip_prefix("<!") {
            rest = skip_declaration(after)?;
        } else {
            let after = rest.strip_prefix('<')?;
            let end = after
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(after.len());
            let name = &after[..end];
            return (!name.is_empty()).then_some(name);
        }
    }
}

/// Skips the rest of a `<!…>` declaration (`decl` starts just past the
/// `<!`), honouring a bracketed internal subset (`<!DOCTYPE rss [ … ]>`).
fn skip_declaration(decl: &str) -> Option<&str> {
    let close = decl.find('>')?;
    let close = match decl.find('[') {
        Some(open) if open < close => {
            let subset_end = open + decl[open..].find(']')?;
            subset_end + decl[subset_end..].find('>')?
        }
        _ => close,
    };
    Some(&decl[close + 1..])
}

/// Whether a JSON object's head carries `"version": "http(s)://jsonfeed.org…"`
/// (any whitespace around the colon).
fn declares_json_feed_version(json: &str) -> bool {
    json.match_indices("\"version\"").any(|(at, key)| {
        json[at + key.len()..]
            .trim_start()
            .strip_prefix(':')
            .map(str::trim_start)
            .is_some_and(|value| {
                value.starts_with("\"https://jsonfeed.org")
                    || value.starts_with("\"http://jsonfeed.org")
            })
    })
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
        assert_eq!(out.feeds.len(), 0);
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
    fn a_youtube_channel_page_synthesizes_the_videos_feed_from_its_canonical_id() {
        // A channel/handle page carries no <link rel="alternate"> feed, only a
        // canonical /channel/UC… URL — we build the videos feed from it.
        let html = r#"<html><head>
            <link rel="canonical" href="https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw">
            <meta property="og:title" content="Linus Tech Tips">
        </head><body></body></html>"#;
        let out = extract(html.as_bytes(), "https://www.youtube.com/@linustechtips");
        assert_eq!(
            out.feeds,
            vec![DiscoveredFeedDto {
                url: "https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw"
                    .into(),
                title: Some("Linus Tech Tips".into()),
            }]
        );
    }

    #[test]
    fn a_youtube_page_with_a_real_alternate_link_does_not_duplicate() {
        // When YouTube *does* declare the feed, use it — no synthesized twin.
        let html = r#"<html><head>
            <link rel="alternate" type="application/rss+xml" href="https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw">
            <link rel="canonical" href="https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw">
        </head><body></body></html>"#;
        let out = extract(html.as_bytes(), "https://www.youtube.com/@linustechtips");
        assert_eq!(out.feeds.len(), 1, "the declared feed is not duplicated");
    }

    #[test]
    fn a_non_youtube_canonical_channel_url_is_ignored() {
        // The /channel/ synthesis is YouTube-host-scoped.
        let html =
            r#"<link rel="canonical" href="https://example.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw">"#;
        let out = extract(html.as_bytes(), "https://example.com/some/page");
        assert_eq!(out.feeds.len(), 0);
    }

    #[test]
    fn xhtml_alternates_are_not_mistaken_for_feeds() {
        let html = r#"<link rel="alternate" type="application/xhtml+xml" href="/amp">"#;
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert_eq!(out.feeds.len(), 0);
    }

    /// An HTML page whose head declares one RSS alternate and is padded so a
    /// 3-byte char (`日`) starts at byte `SNIFF_WINDOW - 1`, straddling the
    /// sniff cut.
    fn page_straddling_the_sniff_window() -> String {
        let prefix = r#"<html><head><link rel="alternate" type="application/rss+xml" href="/feed.xml"><title>"#;
        let mut html = String::from(prefix);
        html.push_str(&"a".repeat(SNIFF_WINDOW - 1 - prefix.len()));
        html.push('日');
        html.push_str("本語</title></head><body>本文</body></html>");
        html
    }

    #[test]
    fn a_multibyte_char_straddling_the_sniff_window_does_not_panic() {
        let html = page_straddling_the_sniff_window();
        assert!(
            !html.is_char_boundary(SNIFF_WINDOW),
            "fixture precondition: the cut falls inside a char"
        );
        assert!(!looks_like_feed(&html));
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert_eq!(
            out.feeds,
            vec![DiscoveredFeedDto {
                url: "https://blog.test/feed.xml".into(),
                title: None,
            }]
        );
    }

    #[test]
    fn a_lossy_replacement_char_at_the_sniff_window_does_not_panic() {
        // Each invalid byte decodes to its own U+FFFD (3 bytes); the first
        // starts at byte SNIFF_WINDOW - 1, so it straddles the cut.
        let mut body = b"<html><head><title>".to_vec();
        body.resize(SNIFF_WINDOW - 1, b'x');
        body.extend_from_slice(&[0xff, 0xfe, 0xfd]);
        body.extend_from_slice(b"</title></head></html>");
        let out = extract(&body, "https://blog.test/");
        assert!(out.feeds.is_empty(), "the replacement chars are plain text");
    }

    #[test]
    fn a_4kb_japanese_and_emoji_page_is_scanned_for_its_links() {
        let mut html = String::from(
            r#"<!DOCTYPE html><html lang="ja"><head><meta charset="utf-8">
            <title>日本語のブログ 🎌</title>
            <link rel="alternate" type="application/atom+xml" title="フィード 📰" href="/atom.xml">
            </head><body>"#,
        );
        let paragraph = "<p>こんにちは世界、今日はいい天気ですね。🍣🍜🗻✨👍🏽👨‍👩‍👧</p>";
        while html.len() + paragraph.len() < SNIFF_WINDOW - 8 {
            html.push_str(paragraph);
        }
        // Pad so a 4-byte emoji starts at byte SNIFF_WINDOW - 1, straddling
        // the sniff cut, then keep filling past 4 KB.
        html.push_str("<p>");
        html.push_str(&"a".repeat(SNIFF_WINDOW - 1 - html.len()));
        html.push_str("🎌</p>");
        while html.len() < 4 * 1024 + 512 {
            html.push_str(paragraph);
        }
        html.push_str("</body></html>");
        assert!(
            !html.is_char_boundary(SNIFF_WINDOW),
            "fixture precondition: the cut falls inside a char"
        );
        assert!(!looks_like_feed(&html));
        let out = extract(html.as_bytes(), "https://blog.test/ja/");
        assert_eq!(
            out.feeds,
            vec![DiscoveredFeedDto {
                url: "https://blog.test/atom.xml".into(),
                title: Some("フィード 📰".into()),
            }]
        );
    }

    #[test]
    fn a_4kb_japanese_and_emoji_feed_is_recognised_as_a_feed() {
        let mut feed = String::from(
            r#"<?xml version="1.0" encoding="utf-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>日本語 🎌</title>"#,
        );
        while feed.len() < 4 * 1024 + 512 {
            feed.push_str("<entry><title>記事のタイトル 🍣👍🏽</title></entry>");
        }
        feed.push_str("</feed>");
        assert!(
            !feed.is_char_boundary(SNIFF_WINDOW),
            "fixture precondition: the cut falls inside a char"
        );
        assert!(looks_like_feed(&feed));
        let out = extract(feed.as_bytes(), "https://blog.test/atom.xml");
        assert_eq!(out.feeds.len(), 1);
        assert_eq!(out.feeds[0].url, "https://blog.test/atom.xml");
    }

    #[test]
    fn feed_roots_are_recognised_past_prolog_comments_and_doctype() {
        let feeds = [
            r#"<rss version="2.0"><channel/></rss>"#,
            "\u{feff}  \n<?xml version=\"1.0\"?>\n<RSS version=\"0.91\"/>",
            r#"<?xml version="1.0"?><?xml-stylesheet href="s.xsl"?><!-- hi --><feed xmlns="http://www.w3.org/2005/Atom"/>"#,
            r#"<?xml version="1.0"?><!DOCTYPE rss PUBLIC "-//Netscape//DTD RSS 0.91//EN" "x.dtd"><rss/>"#,
            r#"<!DOCTYPE rss [ <!ENTITY nbsp "&#160;"> ]><rss version="2.0"/>"#,
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"></rdf:RDF>"#,
            r#"<atom:feed xmlns:atom="http://www.w3.org/2005/Atom"></atom:feed>"#,
            "<feed>",
            r#"{"version":"https://jsonfeed.org/version/1.1","title":"x"}"#,
            "{\n  \"title\": \"x\",\n  \"version\" :\t\"https://jsonfeed.org/version/1\"\n}",
        ];
        for feed in feeds {
            assert!(looks_like_feed(feed), "expected a feed: {feed:?}");
        }
    }

    #[test]
    fn html_mentioning_a_feed_tag_is_not_a_feed() {
        // The old substring sniff misfired on any `<feed`/`<rss` in the head.
        let pages = [
            "<html><head><title>How to write a <feed> element</title></head></html>",
            "<!DOCTYPE html><!-- <rss> --><html><body><rss></rss></body></html>",
            r#"<?xml version="1.0"?><html xmlns="http://www.w3.org/1999/xhtml"><feed/></html>"#,
            "<feedback>not a feed</feedback>",
            r#"{"title":"not a feed","note":"\"version\":\"https://jsonfeed.org\" is quoted"}"#,
            r#"{"version":"1.0"}"#,
            "<!-- unterminated comment <rss>",
            "<?xml version=\"1.0\"",
            "<!DOCTYPE rss [ <!ENTITY x \"y\">",
            "plain text <rss>",
            "",
            "<",
        ];
        for page in pages {
            assert!(!looks_like_feed(page), "expected not a feed: {page:?}");
        }
        let out = extract(pages[0].as_bytes(), "https://blog.test/");
        assert!(out.feeds.is_empty(), "the HTML path ran, finding no links");
    }

    proptest::proptest! {
        #[test]
        fn extract_never_panics_on_arbitrary_bytes(
            body in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..4096),
        ) {
            let _ = extract(&body, "https://blog.test/");
        }

        #[test]
        fn extract_never_panics_on_arbitrary_text(body in "\\PC{0,3000}") {
            let _ = extract(body.as_bytes(), "https://blog.test/");
        }

        #[test]
        fn the_sniff_never_panics_on_markup_shaped_text(
            body in "[ \u{feff}]{0,3}(<\\?[^>]{0,40}\\??>?|<!--.{0,40}(-->)?|<!DOCTYPE[^>]{0,30}(\\[.{0,30}\\]?)?>?|<[a-zA-Z:日本🎌]{0,10}|\\{.{0,40}|[日本語🎌👍🏽a]{0,700}){0,6}",
        ) {
            let _ = looks_like_feed(&body);
            let _ = extract(body.as_bytes(), "https://blog.test/");
        }
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
