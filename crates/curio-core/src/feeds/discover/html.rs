//! The fetch-free half of discovery: what one fetched document declares.
//!
//! [`extract`] is fully synchronous — no `scraper::Html` value ever crosses
//! an await (`Html` is not `Send`), so the async orchestration in the parent
//! module stays `Send` for a head's `spawn`.

use scraper::{Html, Selector};
use url::Url;

use super::DiscoveredFeed;
use super::sniff::looks_like_feed;

/// What one fetched document turned up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Extracted {
    /// `true` when the document itself sniffs as a feed; `feeds` is then
    /// exactly the fetched URL.
    pub(super) is_feed: bool,
    /// Candidates the page declares, deduped, in document order: its
    /// `<link rel="alternate">` feeds (comment feeds dropped when a primary
    /// feed exists), else a synthesized `YouTube` channel feed.
    pub(super) feeds: Vec<DiscoveredFeed>,
    /// Same-origin `<a href>` targets shaped like feed URLs — probe
    /// candidates, tried only when nothing declared validates.
    pub(super) anchors: Vec<String>,
    /// A favicon URL to preview (declared icon or `/favicon.ico`).
    pub(super) favicon: Option<String>,
}

/// Pulls feed candidates + favicon out of one fetched document, resolving
/// every relative URL against `final_url`.
pub(super) fn extract(body: &[u8], final_url: &str) -> Extracted {
    let text = String::from_utf8_lossy(body);
    let base = Url::parse(final_url).ok();

    if looks_like_feed(&text) {
        return Extracted {
            is_feed: true,
            feeds: vec![DiscoveredFeed {
                url: final_url.to_owned(),
                title: None,
            }],
            anchors: Vec::new(),
            favicon: base.as_ref().and_then(favicon_ico),
        };
    }

    let document = Html::parse_document(&text);
    let mut feeds = drop_comment_feeds(feed_links(&document, base.as_ref()));
    // A YouTube channel/handle page usually carries no `<link rel="alternate">`
    // feed — synthesize the videos feed from its canonical channel id so a
    // pasted `youtube.com/@handle` still subscribes. Only when nothing else
    // matched, to avoid a duplicate when the alternate link *is* present.
    if feeds.is_empty()
        && let Some(candidate) = youtube_channel_feed(&document, base.as_ref())
    {
        feeds.push(candidate);
    }
    Extracted {
        is_feed: false,
        feeds,
        anchors: feed_anchors(&document, base.as_ref()),
        favicon: favicon(&document, base.as_ref()),
    }
}

/// If the page is a `YouTube` channel/handle page, builds its
/// `feeds/videos.xml?channel_id=UC…` feed from the canonical `/channel/UC…`
/// URL the page declares (`<link rel="canonical">` or `<meta og:url>`). No new
/// fetch: the page was already retrieved through the policed client.
fn youtube_channel_feed(document: &Html, base: Option<&Url>) -> Option<DiscoveredFeed> {
    if !super::rewrite::is_youtube_host(base?.host_str()?) {
        return None;
    }
    let channel_id = canonical_youtube_channel_id(document)?;
    Some(DiscoveredFeed {
        url: super::rewrite::youtube_channel_feed_url(&channel_id),
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
                && let Some(id) = super::rewrite::channel_id_from_url(href)
            {
                return Some(id);
            }
        }
    }
    None
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

/// `<link rel="alternate" type="…" href="…">` feed candidates whose type is
/// on the allowlist ([`is_feed_link`]), deduped by resolved URL.
fn feed_links(document: &Html, base: Option<&Url>) -> Vec<DiscoveredFeed> {
    // `rel~=alternate` = whitespace-separated token match (handles both
    // `rel="alternate"` and `rel="alternate home"`).
    let Ok(selector) = Selector::parse("link[rel~=alternate][href]") else {
        return Vec::new();
    };
    let mut feeds: Vec<DiscoveredFeed> = Vec::new();
    for element in document.select(&selector) {
        let value = element.value();
        let Some(href) = value.attr("href") else {
            continue;
        };
        let title = value
            .attr("title")
            .map(str::trim)
            .filter(|title| !title.is_empty());
        if !is_feed_link(value.attr("type").unwrap_or_default(), href, title) {
            continue;
        }
        if let Some(resolved) = resolve(base, href)
            && !feeds.iter().any(|feed| feed.url == resolved)
        {
            feeds.push(DiscoveredFeed {
                url: resolved,
                title: title.map(str::to_owned),
            });
        }
    }
    feeds
}

/// The exact feed-type allowlist. `application/rss+xml`, `application/atom+xml`
/// and `application/feed+json` (any top-level type, parameters ignored)
/// always qualify; bare `application/json` only with a feed-like signal (the
/// title or href says "feed"). Everything else — `application/json+oembed`,
/// `text/xml+oembed`, `application/xhtml+xml`, a `WordPress` REST-API
/// `application/json` alternate — is not a feed.
fn is_feed_link(feed_type: &str, href: &str, title: Option<&str>) -> bool {
    let essence = feed_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let subtype = essence.split_once('/').map_or("", |(_, subtype)| subtype);
    match subtype {
        "rss+xml" | "atom+xml" | "feed+json" => true,
        "json" if essence == "application/json" => {
            let says_feed = |text: &str| text.to_ascii_lowercase().contains("feed");
            says_feed(href) || title.is_some_and(says_feed)
        }
        _ => false,
    }
}

/// Drops comment feeds (`WordPress`'s site-wide `/comments/feed/` and per-post
/// "… Comments Feed") when the page also declares a primary feed — a post
/// page lists all three, and only the site feed is what a reader means.
/// A page that declares nothing *but* comment feeds keeps them.
fn drop_comment_feeds(feeds: Vec<DiscoveredFeed>) -> Vec<DiscoveredFeed> {
    let (comments, primary): (Vec<_>, Vec<_>) = feeds.into_iter().partition(is_comment_feed);
    if primary.is_empty() {
        comments
    } else {
        primary
    }
}

fn is_comment_feed(feed: &DiscoveredFeed) -> bool {
    let title = feed
        .title
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let path = Url::parse(&feed.url)
        .map(|url| url.path().to_ascii_lowercase())
        .unwrap_or_default();
    title.contains("comments feed")
        || title.contains("comment feed")
        || path.contains("/comments/feed")
}

/// Path endings that make an `<a href>` worth probing as a feed.
const FEED_SHAPED_SUFFIXES: [&str; 14] = [
    ".rss",
    ".atom",
    ".rdf",
    "rss.xml",
    "atom.xml",
    "feed.xml",
    "index.xml",
    "feed.json",
    "/feed",
    "/feed/",
    "/rss",
    "/rss/",
    "/atom",
    "/atom/",
];

/// Same-origin `<a href>` targets whose path looks like a feed URL, deduped,
/// in document order. Cross-origin anchors are never probe candidates.
fn feed_anchors(document: &Html, base: Option<&Url>) -> Vec<String> {
    let (Some(base), Ok(selector)) = (base, Selector::parse("a[href]")) else {
        return Vec::new();
    };
    let mut anchors: Vec<String> = Vec::new();
    for element in document.select(&selector) {
        let Some(href) = element.value().attr("href") else {
            continue;
        };
        let Ok(mut url) = base.join(href) else {
            continue;
        };
        url.set_fragment(None);
        let path = url.path().to_ascii_lowercase();
        let feed_shaped = FEED_SHAPED_SUFFIXES
            .iter()
            .any(|suffix| path.ends_with(suffix));
        let resolved = String::from(url.clone());
        if feed_shaped
            && url.origin() == base.origin()
            && resolved != base.as_str()
            && !anchors.contains(&resolved)
        {
            anchors.push(resolved);
        }
    }
    anchors
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

/// The same-origin `/favicon.ico` of `base`.
pub(super) fn favicon_ico(base: &Url) -> Option<String> {
    base.join("/favicon.ico").ok().map(String::from)
}

/// Resolves `href` (possibly relative or protocol-relative) against the
/// page's final URL; drops non-http(s) results (heads only ever load these
/// through the policed client, which is http(s)-only).
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

    use super::super::sniff::SNIFF_WINDOW;
    use super::*;

    fn feed(url: &str, title: Option<&str>) -> DiscoveredFeed {
        DiscoveredFeed {
            url: url.to_owned(),
            title: title.map(str::to_owned),
        }
    }

    /// A fixture under `fixtures/discovery/`, read at run time (never
    /// through a compile-time path — a test binary reused from another
    /// worktree would carry it stale, issue #183).
    fn fixture(name: &str) -> Vec<u8> {
        let crate_dir = std::env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("CARGO_MANIFEST_DIR unset: run the tests through cargo"));
        std::fs::read(
            std::path::Path::new(&crate_dir)
                .join("../../fixtures/discovery")
                .join(name),
        )
        .unwrap()
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
        assert!(!out.is_feed);
        assert_eq!(
            out.feeds,
            vec![
                feed("https://blog.test/feed.xml", Some("RSS")),
                feed("https://cdn.test/atom", None),
            ]
        );
        assert_eq!(
            out.favicon.as_deref(),
            Some("https://blog.test/static/favicon.png")
        );
    }

    #[test]
    fn a_url_that_is_already_a_feed_returns_itself_with_ico_favicon() {
        let body = r#"<?xml version="1.0"?><rss version="2.0"><channel></channel></rss>"#;
        let out = extract(body.as_bytes(), "https://blog.test/feed.xml");
        assert!(out.is_feed);
        assert_eq!(out.feeds, vec![feed("https://blog.test/feed.xml", None)]);
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
            vec![feed(
                "https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw",
                Some("Linus Tech Tips"),
            )]
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

    #[test]
    fn a_wordpress_post_yields_only_the_real_feed() {
        // A real-shaped `WordPress` post head: the site feed, the site-wide and
        // per-post comment feeds, a REST-API `application/json` alternate and
        // two oEmbed alternates. Only the site feed is a subscription.
        let out = extract(
            &fixture("wordpress-post.html"),
            "https://blog.example/2026/10/hello-world/",
        );
        assert_eq!(
            out.feeds,
            vec![feed(
                "https://blog.example/feed/",
                Some("Example Blog » Feed")
            )]
        );
    }

    #[test]
    fn the_type_allowlist_is_exact() {
        let cases = [
            ("application/rss+xml", "/x", None, true),
            ("application/atom+xml", "/x", None, true),
            ("application/feed+json", "/x", None, true),
            ("APPLICATION/RSS+XML; charset=utf-8", "/x", None, true),
            ("text/rss+xml", "/x", None, true),
            ("application/json", "/feed.json", None, true),
            ("application/json", "/x", Some("JSON Feed"), true),
            (
                "application/json",
                "/wp-json/wp/v2/posts/7",
                Some("JSON"),
                false,
            ),
            (
                "application/json+oembed",
                "/oembed?feed=1",
                Some("feed"),
                false,
            ),
            ("text/xml+oembed", "/oembed", None, false),
            ("text/json", "/feed.json", None, false),
            ("application/xhtml+xml", "/feed", None, false),
            ("text/xml", "/feed", None, false),
            ("text/html", "/feed", None, false),
            ("", "/feed.xml", None, false),
        ];
        for (feed_type, href, title, expected) in cases {
            assert_eq!(
                is_feed_link(feed_type, href, title),
                expected,
                "{feed_type:?} {href:?} {title:?}"
            );
        }
    }

    #[test]
    fn a_page_declaring_only_comment_feeds_keeps_them() {
        let html = r#"<link rel="alternate" type="application/rss+xml" title="Comments Feed" href="/comments/feed/">"#;
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert_eq!(
            out.feeds.len(),
            1,
            "no primary feed: the comments feed stays"
        );
    }

    #[test]
    fn duplicate_alternates_collapse_to_one_candidate() {
        let html = r#"<link rel="alternate" type="application/rss+xml" title="A" href="/feed.xml">
                      <link rel="alternate" type="application/rss+xml" title="B" href="https://blog.test/feed.xml">"#;
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert_eq!(
            out.feeds,
            vec![feed("https://blog.test/feed.xml", Some("A"))]
        );
    }

    #[test]
    fn feed_shaped_anchors_are_same_origin_and_deduped() {
        let html = r#"<body>
            <a href="/rss.xml">RSS</a>
            <a href="/rss.xml#top">RSS again</a>
            <a href="https://other.test/feed.xml">elsewhere</a>
            <a href="http://blog.test/atom.xml">other scheme = other origin</a>
            <a href="/about">About</a>
            <a href="/posts/feed/">Posts feed</a>
            <a href="/">home</a>
        </body>"#;
        let out = extract(html.as_bytes(), "https://blog.test/");
        assert_eq!(
            out.anchors,
            vec!["https://blog.test/rss.xml", "https://blog.test/posts/feed/"]
        );
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
        assert_eq!(out.feeds, vec![feed("https://blog.test/feed.xml", None)]);
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
            vec![feed("https://blog.test/atom.xml", Some("フィード 📰"))]
        );
    }

    #[test]
    fn a_4kb_japanese_and_emoji_feed_is_recognised_as_a_feed() {
        let mut body = String::from(
            r#"<?xml version="1.0" encoding="utf-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>日本語 🎌</title>"#,
        );
        while body.len() < 4 * 1024 + 512 {
            body.push_str("<entry><title>記事のタイトル 🍣👍🏽</title></entry>");
        }
        body.push_str("</feed>");
        assert!(
            !body.is_char_boundary(SNIFF_WINDOW),
            "fixture precondition: the cut falls inside a char"
        );
        assert!(looks_like_feed(&body));
        let out = extract(body.as_bytes(), "https://blog.test/atom.xml");
        assert_eq!(out.feeds, vec![feed("https://blog.test/atom.xml", None)]);
    }

    #[test]
    fn html_mentioning_a_feed_tag_runs_the_html_path() {
        let page = "<html><head><title>How to write a <feed> element</title></head></html>";
        let out = extract(page.as_bytes(), "https://blog.test/");
        assert!(!out.is_feed);
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
    }
}
