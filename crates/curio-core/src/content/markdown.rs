//! HTML → `CommonMark` via htmd's DOM walk (D5).
//!
//! The sketch's regex converter — which destroyed every element spanning
//! lines — is banned; conversion is a real DOM traversal. Input is
//! expected to be sanitized already (this module is called on stored
//! content at export time), but the skip list drops any script/style/
//! boilerplate defensively anyway.
//!
//! Footnotes become `CommonMark` footnotes (`[^1]` / `[^1]: …`) and other
//! in-page links keep only their text (see `footnotes`); a `pre`/`code`
//! `language-*` class becomes the fence's info string.

use htmd::HtmlToMarkdown;

use super::{ContentError, footnotes};

/// Converts an HTML fragment to `CommonMark`.
///
/// # Errors
///
/// [`ContentError::Markdown`] if the DOM walk fails to serialize
/// (practically unreachable for parseable HTML).
pub fn to_markdown(html: &str) -> Result<String, ContentError> {
    let builder = HtmlToMarkdown::builder().skip_tags(vec![
        "script", "style", "iframe", "object", "embed", "form", "nav", "aside", "header", "footer",
        "noscript", "svg", "canvas",
    ]);
    let converter = footnotes::register(builder, html).build();
    Ok(converter.convert(html)?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn multiline_paragraphs_survive() {
        let md = to_markdown("<p>one\ntwo\nthree</p><p>four</p>").unwrap();
        assert!(md.contains("one"));
        assert!(md.contains("four"));
        assert!(!md.contains('<'));
    }

    #[test]
    fn multiline_code_blocks_become_fences() {
        let md = to_markdown("<pre><code>fn main() {\n    let x = 1;\n}</code></pre>").unwrap();
        assert!(md.contains("```"), "expected a fence: {md}");
        assert!(md.contains("let x = 1;"));
    }

    #[test]
    fn links_and_images_convert() {
        let md = to_markdown(
            "<p><a href=\"https://e.com/a\">link</a> and \
             <img src=\"https://e.com/i.png\" alt=\"pic\"></p>",
        )
        .unwrap();
        assert!(md.contains("[link](https://e.com/a)"));
        assert!(md.contains("![pic](https://e.com/i.png)"));
    }

    #[test]
    fn footnotes_become_commonmark_footnotes() {
        let md = to_markdown(
            "<p>Claim<sup id=\"user-content-fnref1\"><a href=\"#user-content-fn1\">1</a></sup> and \
             more<sup id=\"user-content-fnref2\"><a href=\"#user-content-fn2\">2</a></sup>.</p>\
             <hr><ol>\
             <li id=\"user-content-fn1\"><p>First note. <a href=\"#user-content-fnref1\">↩</a></p>\
             <p>Second paragraph.</p></li>\
             <li id=\"user-content-fn2\"><p>See <a href=\"https://e.com/src\">source</a>. \
             <a href=\"#user-content-fnref2\">↩</a></p></li></ol>",
        )
        .unwrap();
        assert!(md.contains("Claim[^1] and more[^2]."), "{md}");
        assert!(
            md.contains(
                "[^1]: First note.\n\n    Second paragraph.\n[^2]: See [source](https://e.com/src)."
            ),
            "{md}"
        );
        assert!(!md.contains('↩'), "back-reference leaked: {md}");
        assert!(!md.contains("](#"), "dead in-page link: {md}");
    }

    #[test]
    fn other_in_page_links_keep_their_text() {
        let md =
            to_markdown("<p>Jump to <a href=\"#user-content-part-2\">part two</a>.</p>").unwrap();
        assert_eq!(md, "Jump to part two.");
    }

    #[test]
    fn ordinary_ordered_lists_are_untouched_in_a_footnoted_doc() {
        let md = to_markdown(
            "<ol start=\"3\"><li>three</li><li>four</li></ol>\
             <p>x<a href=\"#user-content-fn1\">1</a></p><ol><li id=\"user-content-fn1\">n</li></ol>",
        )
        .unwrap();
        assert!(md.contains("3.  three\n4.  four"), "{md}");
        assert!(md.contains("x[^1]"), "{md}");
        assert!(md.contains("[^1]: n"), "{md}");
    }

    #[test]
    fn deeply_nested_lists_in_a_footnoted_doc_are_walked_once() {
        // Each list used to be walked twice (once to look for definitions,
        // once by htmd), so export cost doubled per nesting level: 64
        // levels would never finish. Walked once, this is instant.
        const DEPTH: usize = 64;
        let html = format!(
            "<p>x<a href=\"#user-content-fn1\">1</a></p>{}deep{}\
             <ol><li id=\"user-content-fn1\">n</li></ol>",
            "<ol><li>".repeat(DEPTH),
            "</li></ol>".repeat(DEPTH),
        );
        let md = to_markdown(&html).unwrap();
        assert!(md.contains("deep"), "{md}");
        assert!(md.contains("x[^1]"), "{md}");
        assert!(md.contains("[^1]: n"), "{md}");
    }

    #[test]
    fn in_page_links_inside_a_definition_keep_their_text() {
        let md = to_markdown(
            "<h2 id=\"user-content-part-2\">Part two</h2>\
             <p>x<sup id=\"user-content-fnref1\"><a href=\"#user-content-fn1\">1</a></sup></p>\
             <ol><li id=\"user-content-fn1\">See <a href=\"#user-content-part-2\">part two</a>. \
             <a href=\"#user-content-fnref1\">↩</a></li></ol>",
        )
        .unwrap();
        assert!(md.contains("[^1]: See part two."), "{md}");
        assert!(!md.contains('↩'), "back-reference leaked: {md}");
    }

    #[test]
    fn code_fence_keeps_its_language() {
        let md = to_markdown("<pre><code class=\"language-rust\">let x = 1;</code></pre>").unwrap();
        assert!(md.starts_with("```rust\n"), "{md}");
    }

    #[test]
    fn headings_and_lists_convert() {
        let md = to_markdown("<h2>title</h2><ul><li>a</li><li>b</li></ul>").unwrap();
        assert!(md.contains("## title"));
        let has_bullet = md
            .lines()
            .any(|line| matches!(line.trim_start().chars().next(), Some('-' | '*' | '+')));
        assert!(has_bullet, "list lost: {md}");
    }
}
