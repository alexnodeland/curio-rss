//! Sanitize-at-ingest content pipeline (D5/D6).
//!
//! ```text
//! raw feed/page HTML → ammonia (strict allowlist)
//!                    → readability extraction (main content)
//!                    → plain text (FTS `content_text`) + word count
//!                    → htmd HTML→CommonMark at export time
//! ```
//!
//! Raw HTML never reaches storage: [`process`] is the only door, and it
//! sanitizes first. Scripts, iframes, event handlers, inline style,
//! `javascript:`/`data:` URLs and `<base>` are gone by construction —
//! the desktop head's CSP is defense-in-depth, not the primary defense.
//! Markdown conversion is a DOM walk (htmd); the sketch's regex
//! converter and render-time sanitization are banned.

mod extract;
mod markdown;
mod sanitize;

pub use extract::extract_main_content;
pub use markdown::to_markdown;
pub use sanitize::sanitize;

/// Storage-ready content: sanitized/extracted HTML plus the extracted
/// plain text that makes full-text search real (D4).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProcessedContent {
    /// Sanitized, readability-extracted HTML.
    pub html: String,
    /// Whitespace-normalized plain text (the FTS `content_text` column).
    pub text: String,
    /// Word count of the extracted text.
    pub word_count: u32,
}

/// Content-pipeline failures.
#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    /// htmd failed to serialize the DOM walk.
    #[error("markdown conversion: {0}")]
    Markdown(#[from] std::io::Error),
}

/// The full ingest pipeline: sanitize → extract → text. `base_url`
/// (the article's own URL) resolves relative links and images before
/// the scheme allowlist judges them.
#[must_use]
pub fn process(raw_html: &str, base_url: Option<&str>) -> ProcessedContent {
    let clean = sanitize(raw_html, base_url);
    let html = extract_main_content(&clean);
    let text = plain_text(&html);
    let word_count = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
    ProcessedContent {
        html,
        text,
        word_count,
    }
}

/// Elements whose boundaries do *not* split words: `<b>old</b>er` reads
/// "older", but `<p>a</p><p>b</p>` reads "a b".
const INLINE_ELEMENTS: &[&str] = &[
    "a", "abbr", "b", "cite", "code", "del", "dfn", "em", "i", "ins", "kbd", "mark", "q", "s",
    "samp", "small", "span", "strong", "sub", "sup", "time", "u", "var", "wbr",
];

/// Whitespace-normalized plain text of an HTML fragment. Block-element
/// boundaries become word boundaries; inline elements do not.
#[must_use]
pub fn plain_text(html: &str) -> String {
    use ego_tree::iter::Edge;

    let document = scraper::Html::parse_fragment(html);
    let mut raw = String::new();
    for edge in document.root_element().traverse() {
        let node = match edge {
            Edge::Open(node) | Edge::Close(node) => node,
        };
        match node.value() {
            scraper::Node::Text(text) => {
                if matches!(edge, Edge::Open(_)) {
                    raw.push_str(text);
                }
            }
            scraper::Node::Element(element) if !INLINE_ELEMENTS.contains(&element.name()) => {
                raw.push(' ');
            }
            _ => {}
        }
    }
    let mut out = String::with_capacity(raw.len());
    for word in raw.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

/// The first `http(s)` `<img>` source in a fragment, in document order —
/// the inline-image fallback for an article's lead image when the feed
/// declared none in its metadata. Runs over the *sanitized* body, so the
/// `src` is already base-resolved to an absolute URL and scheme-allowlisted;
/// `data:` and other non-fetchable schemes are skipped (the lead image is a
/// URL to load through the policed cache, not inline bytes).
#[must_use]
pub fn first_image(html: &str) -> Option<String> {
    // A fragment-local selector parse can't fail; unwrap is unreachable.
    let selector = scraper::Selector::parse("img[src]").ok()?;
    scraper::Html::parse_fragment(html)
        .select(&selector)
        .filter_map(|img| img.value().attr("src"))
        .map(str::trim)
        .find(|src| src.starts_with("http://") || src.starts_with("https://"))
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_populates_text_and_word_count() {
        let processed = process("<p>Hello <b>world</b>, again.</p>", None);
        assert_eq!(processed.text, "Hello world, again.");
        assert_eq!(processed.word_count, 3);
        assert!(processed.html.contains("world"));
    }

    #[test]
    fn plain_text_collapses_whitespace() {
        assert_eq!(
            plain_text("<p>a\n   b</p>\n\n<p>c</p>"),
            "a b c",
            "newlines and runs of spaces collapse to single spaces"
        );
    }

    #[test]
    fn empty_input_is_empty_output() {
        let processed = process("", None);
        assert_eq!(processed.text, "");
        assert_eq!(processed.word_count, 0);
    }

    #[test]
    fn first_image_takes_the_leading_http_img() {
        let html = "<p>intro</p><img src=\"https://cdn.example.com/a.jpg\"/>\
                    <img src=\"https://cdn.example.com/b.jpg\"/>";
        assert_eq!(
            first_image(html).as_deref(),
            Some("https://cdn.example.com/a.jpg"),
            "document order wins"
        );
    }

    #[test]
    fn first_image_skips_non_http_sources() {
        let html = "<img src=\"data:image/png;base64,AAAA\"/>\
                    <img src=\"/relative.png\"/>\
                    <img src=\"https://cdn.example.com/real.png\"/>";
        assert_eq!(
            first_image(html).as_deref(),
            Some("https://cdn.example.com/real.png"),
            "data: and relative srcs can't be fetched by the cache"
        );
    }

    #[test]
    fn first_image_is_none_without_a_usable_img() {
        assert_eq!(first_image("<p>just words</p>"), None);
        assert_eq!(first_image("<img src=\"/only-relative.png\"/>"), None);
    }
}
