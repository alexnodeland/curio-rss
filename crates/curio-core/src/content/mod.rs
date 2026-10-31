//! Sanitize-at-ingest content pipeline (D5/D6).
//!
//! ```text
//! raw feed/page HTML → ammonia (strict allowlist)
//!                    → readability extraction (whole pages only)
//!                    → plain text (FTS `content_text`) + word count
//!                    → htmd HTML→CommonMark at export time
//! ```
//!
//! A feed item is usually a bare content fragment, and a fragment is
//! *sanitized only*: everything the allowlist keeps is the article, so
//! nothing is dropped for scoring lower than a sibling. Extraction runs
//! only when the item really is a whole page — it declares `<html>` or
//! `<body>` **and** still carries page chrome (`nav`, or a page-level
//! `header`/`footer`/`aside`) after sanitizing.
//!
//! Raw HTML never reaches storage: [`process`] (feed fragments, sanitize
//! first) and [`process_full_page`] (fetched full pages, readability-extract
//! first, then sanitize) are the only doors — either way ammonia is the last
//! gate before storage. Scripts, iframes, event handlers, inline style,
//! `javascript:`/`data:` URLs and `<base>` are gone by construction —
//! the desktop head's CSP is defense-in-depth, not the primary defense.
//! Markdown conversion is a DOM walk (htmd); the sketch's regex
//! converter and render-time sanitization are banned.

mod extract;
mod markdown;
mod readable;
mod sanitize;

pub use extract::extract_main_content;
pub use markdown::to_markdown;
pub use readable::{PageMeta, extract_full_page, extract_full_page_with_meta};
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
    /// The readability extractor could not parse or score the page.
    #[error("readability: {0}")]
    Readability(#[from] dom_smoothie::ReadabilityError),
}

/// The feed-item pipeline: sanitize → (extract, whole pages only) → text.
/// `base_url` (the article's own URL) resolves relative links and images
/// before the scheme allowlist judges them. A fragment keeps everything
/// the sanitizer keeps; see the module docs for when extraction runs.
#[must_use]
pub fn process(raw_html: &str, base_url: Option<&str>) -> ProcessedContent {
    let clean = sanitize(raw_html, base_url);
    let html = if declares_document(raw_html) && extract::has_page_chrome(&clean) {
        extract_main_content(&clean)
    } else {
        clean
    };
    let text = plain_text(&html);
    let word_count = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
    ProcessedContent {
        html,
        text,
        word_count,
    }
}

/// The full-**page** pipeline for a fetched article: readability extract →
/// ammonia sanitize → text. Distinct from [`process`] (feed fragments, which
/// sanitize first): a full page is *extracted* first to find the article body
/// amid nav/comments/chrome, then the extractor's output passes the sanitize
/// gate. `url` is the article's final (post-redirect) URL — the relative-link
/// base and scheme-allowlist context.
///
/// # Errors
///
/// [`ContentError::Readability`] if the page cannot be parsed or scored.
pub fn process_full_page(raw_html: &str, url: &str) -> Result<ProcessedContent, ContentError> {
    Ok(process_page(raw_html, url)?.content)
}

/// A processed full page plus the metadata the page itself declared —
/// what a single-URL save needs (title, byline, lead image) on top of
/// the body that [`process_full_page`] yields.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProcessedPage {
    /// The sanitized, extracted body.
    pub content: ProcessedContent,
    /// The page's own metadata, best-effort.
    pub meta: PageMeta,
}

/// [`process_full_page`], also surfacing the page's own metadata.
///
/// # Errors
///
/// [`ContentError::Readability`] if the page cannot be parsed or scored.
pub fn process_page(raw_html: &str, url: &str) -> Result<ProcessedPage, ContentError> {
    let (extracted, meta) = extract_full_page_with_meta(raw_html, url)?;
    let html = sanitize(&extracted, Some(url));
    let text = plain_text(&html);
    let word_count = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
    Ok(ProcessedPage {
        content: ProcessedContent {
            html,
            text,
            word_count,
        },
        meta,
    })
}

/// Whether raw HTML carries an explicit `<html>` or `<body>` start tag
/// (any case) — the mark of a whole document rather than a fragment.
/// Checked on the *raw* input because the sanitizer strips both tags.
fn declares_document(raw_html: &str) -> bool {
    let bytes = raw_html.as_bytes();
    raw_html.match_indices('<').any(|(at, _)| {
        ["html", "body"].iter().any(|tag| {
            let name_end = at + 1 + tag.len();
            bytes
                .get(at + 1..name_end)
                .is_some_and(|name| name.eq_ignore_ascii_case(tag.as_bytes()))
                && bytes
                    .get(name_end)
                    .is_none_or(|&next| next == b'>' || next == b'/' || next.is_ascii_whitespace())
        })
    })
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
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn process_populates_text_and_word_count() {
        let processed = process("<p>Hello <b>world</b>, again.</p>", None);
        assert_eq!(processed.text, "Hello world, again.");
        assert_eq!(processed.word_count, 3);
        assert!(processed.html.contains("world"));
    }

    #[test]
    fn process_full_page_extracts_the_body_and_sanitizes() {
        let page = r"<html><head><title>Ignored</title></head><body>
            <header><nav>Home · About · Contact · Subscribe</nav></header>
            <article>
              <h1>The Real Headline</h1>
              <p>This is the first substantial paragraph of the genuine article body,
                 written with enough real sentences that a readability scorer prefers
                 this region over the surrounding navigation and footer chrome.</p>
              <p>A second full paragraph continues the argument with more meaningful
                 prose, cementing this container as the densest block of text on the
                 page and therefore the main content the reader actually wants.</p>
              <script>alert('xss')</script>
            </article>
            <footer>Copyright, related links, more navigation, cookie notice.</footer>
            </body></html>";
        let processed = process_full_page(page, "https://example.com/post").unwrap();
        assert!(
            processed.text.contains("first substantial paragraph"),
            "extracted the article body: {}",
            processed.text
        );
        assert!(
            !processed.html.contains("<script"),
            "output passed the sanitize gate"
        );
        assert!(processed.word_count > 20);
    }

    #[test]
    fn fragments_are_sanitized_only() {
        let fragment = "<p>A short intro.</p>\
            <div><p>The longer body paragraph that would outscore the intro.</p></div>\
            <header><p>A header the feed put in the item.</p></header>\
            <aside><p>An aside callout.</p></aside>\
            <footer><p>A closing footer note.</p></footer>";
        let processed = process(fragment, None);
        assert_eq!(
            processed.html,
            sanitize(fragment, None),
            "fragment was rewritten"
        );
        for kept in [
            "A short intro.",
            "longer body",
            "header the feed",
            "aside callout",
            "closing footer",
        ] {
            assert!(
                processed.text.contains(kept),
                "lost {kept:?}: {}",
                processed.text
            );
        }
    }

    #[test]
    fn a_wrapped_fragment_without_chrome_is_sanitized_only() {
        let wrapped = "<html><body><p>A short intro.</p>\
            <div><p>The longer body paragraph that would outscore the intro.</p></div>\
            </body></html>";
        let processed = process(wrapped, None);
        assert_eq!(processed.html, sanitize(wrapped, None));
        assert!(processed.text.contains("A short intro."));
    }

    #[test]
    fn chrome_without_a_document_wrapper_is_not_extracted() {
        let fragment = "<nav><p>An in-post table of contents.</p></nav><p>The body.</p>";
        let processed = process(fragment, None);
        assert!(
            processed.text.contains("table of contents"),
            "{}",
            processed.text
        );
    }

    #[test]
    fn a_whole_page_item_is_extracted() {
        let page = "<HTML><Body class=\"x\"><nav><a href=\"https://e.com/\">Home</a></nav>\
            <p>A short intro.</p><article><p>The longer body paragraph of the post.</p>\
            <blockquote><p>Quoted words.</p><footer>— Someone</footer></blockquote>\
            </article><footer><p>© the site</p></footer></Body></HTML>";
        let processed = process(page, None);
        assert!(
            !processed.text.contains("Home"),
            "nav leaked: {}",
            processed.text
        );
        assert!(
            !processed.text.contains("© the site"),
            "footer leaked: {}",
            processed.text
        );
        for kept in [
            "A short intro.",
            "longer body",
            "Quoted words.",
            "— Someone",
        ] {
            assert!(
                processed.text.contains(kept),
                "lost {kept:?}: {}",
                processed.text
            );
        }
    }

    #[test]
    fn declares_document_matches_real_tags_only() {
        assert!(declares_document("<html><p>x</p></html>"));
        assert!(declares_document("<!DOCTYPE html>\n<HTML lang=\"en\">"));
        assert!(declares_document("<body>"));
        assert!(declares_document("<body\nclass=\"post\">"));
        assert!(
            declares_document("x<html"),
            "a tag cut off at the end still counts"
        );
        assert!(!declares_document("<p>&lt;html&gt; is escaped</p>"));
        assert!(!declares_document("<htmlish>not it</htmlish><bodyguard>"));
        assert!(!declares_document("<p>plain</p>"));
        assert!(!declares_document(""));
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
