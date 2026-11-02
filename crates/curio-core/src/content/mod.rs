//! Sanitize-at-ingest content pipeline (D5/D6).
//!
//! ```text
//! feed item  → ammonia (strict allowlist)
//!            → main-content extraction (whole pages only)
//! full page  → dom_smoothie readability → ammonia → page-chrome strip
//!            → plain text (FTS `content_text`) + word count
//!            → htmd HTML→CommonMark at export time
//! ```
//!
//! A feed item is usually a bare content fragment, and a fragment is
//! *sanitized only*: everything the allowlist keeps is the article, so
//! nothing is dropped for scoring lower than a sibling. Extraction runs
//! only when the item really is a whole page — it declares `<html>` or
//! `<body>` **and** still carries page chrome (`nav`, or a page-level
//! `header`/`footer`/`aside`) after sanitizing.
//!
//! Either door leaves page chrome out of storage: [`process`] through
//! the extractor, [`process_full_page`] by running the same chrome rule
//! over the readability output, which keeps breadcrumbs and page
//! headers that sit inside its chosen container.
//!
//! Raw HTML never reaches storage: [`process`] (feed fragments, sanitize
//! first) and [`process_full_page`] (fetched full pages, readability-extract
//! first, then sanitize) are the only doors — either way ammonia is the last
//! gate before storage. Scripts, iframes, event handlers, inline style,
//! `javascript:`/`data:` URLs and `<base>` are gone by construction —
//! the desktop head's CSP is defense-in-depth, not the primary defense.
//! Markdown conversion is a DOM walk (htmd); the sketch's regex
//! converter and render-time sanitization are banned.

mod code;
mod embeds;
mod extract;
mod footnotes;
mod lazy_images;
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
    /// Sanitized HTML: a feed fragment as the allowlist left it, or the
    /// extracted main content of a whole page.
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
/// ammonia sanitize → page-chrome strip → text. Distinct from [`process`]
/// (feed fragments, which sanitize first): a full page is *extracted* first
/// to find the article body amid nav/comments/chrome, then the extractor's
/// output passes the sanitize gate, then any `nav` or page-level
/// `header`/`footer`/`aside` the readability pass kept inside its container
/// is dropped by the same rule the feed extractor uses. The gate's
/// lazy-image and embed softening also runs *before* extraction, since the
/// extractor would otherwise delete the iframes and `<noscript>` fallbacks
/// it works from. `url` is the article's final (post-redirect) URL — the relative-link
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
    // Soften before extracting: the extractor deletes every iframe outside
    // its short video-host list, and every `<noscript>`, so embed
    // placeholders and lazy-image fallbacks must exist before it runs.
    let base = url::Url::parse(url).ok();
    let softened = sanitize::soften(raw_html, base.as_ref());
    let (extracted, meta) = extract_full_page_with_meta(&softened, url)?;
    let html = extract::strip_page_chrome(&sanitize(&extracted, Some(url)));
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

/// Elements whose content is raw text to the HTML tokenizer: a `<body>`
/// inside them is not a tag.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style", "textarea", "title", "xmp", "noscript"];

/// Whether raw HTML carries an explicit `<html>` or `<body>` start tag
/// (any case) — the mark of a whole document rather than a fragment.
/// Checked on the *raw* input because the sanitizer strips both tags.
/// Comments and raw-text element bodies (`script`, `style`, …) are
/// skipped, so `<!-- <body> -->` does not turn a fragment into a page.
fn declares_document(raw_html: &str) -> bool {
    let bytes = raw_html.as_bytes();
    let mut at = 0;
    while let Some(offset) = raw_html.get(at..).and_then(|rest| rest.find('<')) {
        at += offset;
        let after = at + 1;
        if bytes.get(after..after + 3) == Some(b"!--".as_slice()) {
            // A comment: resume after its end, or stop if it never ends.
            match raw_html.get(after + 3..).and_then(|rest| rest.find("-->")) {
                Some(end) => at = after + 3 + end + 3,
                None => return false,
            }
            continue;
        }
        if ["html", "body"]
            .iter()
            .any(|tag| starts_tag(bytes, after, tag))
        {
            return true;
        }
        if let Some(raw) = RAW_TEXT_ELEMENTS
            .iter()
            .find(|tag| starts_tag(bytes, after, tag))
        {
            // Raw text: resume at its closing tag, or stop if there is none.
            match find_ascii_ci(bytes, after + raw.len(), &format!("</{raw}")) {
                Some(close) => at = close + 2,
                None => return false,
            }
            continue;
        }
        at = after;
    }
    false
}

/// Whether `bytes[name_at..]` starts with the tag name `tag` (any case),
/// followed by the end of input, `>`, `/` or whitespace.
fn starts_tag(bytes: &[u8], name_at: usize, tag: &str) -> bool {
    let name_end = name_at + tag.len();
    bytes
        .get(name_at..name_end)
        .is_some_and(|name| name.eq_ignore_ascii_case(tag.as_bytes()))
        && bytes
            .get(name_end)
            .is_none_or(|&next| next == b'>' || next == b'/' || next.is_ascii_whitespace())
}

/// The first index at or after `from` where `needle` (ASCII) occurs,
/// ignoring ASCII case.
fn find_ascii_ci(bytes: &[u8], from: usize, needle: &str) -> Option<usize> {
    let needle = needle.as_bytes();
    bytes
        .get(from..)?
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|position| from + position)
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
    fn process_full_page_keeps_the_code_fence_language() {
        // The extractor strips classes by default; the language class has
        // to survive it to reach the sanitizer (which keeps only it).
        let page = r#"<html><body><article>
              <h1>Query planning</h1>
              <p>This is the first substantial paragraph of the genuine article body,
                 written with enough real sentences that a readability scorer prefers
                 this region over anything else on the page around it.</p>
              <pre class="highlight"><code class="hljs language-sql">SELECT 1;</code></pre>
              <p>A second full paragraph continues the argument with more meaningful
                 prose, cementing this container as the densest block of text.</p>
            </article></body></html>"#;
        let processed = process_full_page(page, "https://example.com/post").unwrap();
        assert!(
            processed
                .html
                .contains("<code class=\"language-sql\">SELECT 1;</code>"),
            "{}",
            processed.html
        );
        assert!(!processed.html.contains("hljs"), "{}", processed.html);
        assert!(!processed.html.contains("highlight"), "{}", processed.html);
    }

    #[test]
    fn process_full_page_keeps_embeds_and_lazy_images() {
        // The extractor deletes iframes outside its video-host list and
        // every <noscript>; the softening pass has to run before it.
        let page = r#"<html><body><article>
              <h1>Listen and look</h1>
              <p>This is the first substantial paragraph of the genuine article body,
                 written with enough real sentences that a readability scorer prefers
                 this region over anything else on the page around it.</p>
              <iframe src="https://open.spotify.com/embed/episode/abc" width="100%" height="232"></iframe>
              <p>A second full paragraph continues the argument with more meaningful
                 prose, cementing this container as the densest block of text.</p>
              <iframe src="https://mastodon.social/@Mastodon/1093/embed" width="400"></iframe>
              <p>A third paragraph introduces the picture below, which a lazy loader
                 only reveals once scripts run, with a noscript fallback beside it.</p>
              <img src="data:image/gif;base64,R0lGOD" alt="lazy" class="lazyload">
              <noscript><img src="/uploads/real.jpg" alt="lazy"></noscript>
              <p>A closing paragraph wraps up the article with a few more sentences.</p>
            </article></body></html>"#;
        let processed = process_full_page(page, "https://example.com/post").unwrap();
        let markdown = to_markdown(&processed.html).unwrap();
        for needle in [
            "[▶ Embedded media (open.spotify.com) ↗](https://open.spotify.com/embed/episode/abc)",
            "[Embedded content (mastodon.social) ↗](https://mastodon.social/@Mastodon/1093/embed)",
            "![lazy](https://example.com/uploads/real.jpg)",
        ] {
            assert!(
                markdown.contains(needle),
                "{needle} missing from:\n{markdown}"
            );
        }
        assert!(!processed.html.contains("iframe"), "{}", processed.html);
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
            <p>A short intro.</p><div><p>The longer body paragraph of the post.</p>\
            <blockquote><p>Quoted words.</p><footer>— Someone</footer></blockquote>\
            </div><footer><p>© the site</p></footer></Body></HTML>";
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
    fn a_wrapped_post_of_headed_sections_keeps_its_headings() {
        let wrapped = "<html><body>\
            <section><header><h2>Part one title</h2></header><p>Part one body.</p></section>\
            <section><header><h2>Part two title</h2></header><p>Part two body.</p></section>\
            </body></html>";
        let processed = process(wrapped, None);
        assert_eq!(processed.html, sanitize(wrapped, None), "not a page");
        for kept in ["Part one title", "Part two title"] {
            assert!(processed.text.contains(kept), "lost {kept:?}");
        }
    }

    #[test]
    fn a_whole_page_keeps_the_title_block_of_its_main() {
        let page = "<html><body><nav><a href=\"https://e.com/\">Home</a></nav><main>\
            <header><h1>Post title</h1></header>\
            <p>Body text one, long enough to score decisively here.</p>\
            <p>Body text two, also long enough to add to the score.</p>\
            </main></body></html>";
        let processed = process(page, None);
        assert!(
            processed.text.starts_with("Post title"),
            "{}",
            processed.text
        );
        assert!(!processed.text.contains("Home"), "{}", processed.text);
    }

    #[test]
    fn a_body_tag_in_a_comment_or_script_is_not_a_document() {
        let fragment = "<!-- <body> --><nav><p>An in-post table of contents.</p></nav>\
            <script>document.write('<body>')</script><p>The body.</p>";
        assert!(!declares_document(fragment));
        let processed = process(fragment, None);
        assert!(
            processed.text.contains("table of contents"),
            "{}",
            processed.text
        );
    }

    #[test]
    fn full_page_output_loses_its_breadcrumbs_but_keeps_the_byline() {
        let page = r#"<html><head><title>Title</title></head><body>
            <header><p>Example Blog</p></header>
            <article>
              <nav class="breadcrumbs"><a href="/blog">Blog</a> › <a href="/tech">Tech</a></nav>
              <header><h1>Title</h1><p>By someone</p></header>
              <p>This is the first substantial paragraph of the genuine article body,
                 written with enough real sentences that a readability scorer prefers
                 this region over the surrounding navigation and footer chrome.</p>
              <p>A second full paragraph continues the argument with more meaningful
                 prose, cementing this container as the densest block of text on the
                 page and therefore the main content the reader actually wants.</p>
            </article>
            </body></html>"#;
        let processed = process_full_page(page, "https://example.com/post").unwrap();
        assert!(!processed.html.contains("<nav"), "{}", processed.html);
        assert!(
            !processed.text.contains("Tech"),
            "breadcrumbs stored: {}",
            processed.text
        );
        let markdown = to_markdown(&processed.html).unwrap();
        assert!(
            !markdown.contains("Blog"),
            "breadcrumbs exported: {markdown}"
        );
        assert!(
            markdown.contains("By someone"),
            "the article's own header: {markdown}"
        );
        assert!(
            markdown.contains("first substantial paragraph"),
            "{markdown}"
        );
    }

    #[test]
    fn declares_document_skips_comments_and_raw_text() {
        assert!(!declares_document("<!-- <body> --><p>x</p>"));
        assert!(!declares_document("<!-- <html> never closed"));
        assert!(!declares_document(
            "<script>var s = '<body>';</script><p>x</p>"
        ));
        assert!(!declares_document("<STYLE>/* <html> */</Style><p>x</p>"));
        assert!(!declares_document("<textarea><body></textarea>"));
        assert!(!declares_document("<script>never closed <body>"));
        assert!(declares_document("<!-- note --><body><p>x</p></body>"));
        assert!(declares_document("<script>x</script><html>"));
        assert!(
            declares_document("<p>é</p><body>"),
            "multi-byte text before the tag"
        );
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
