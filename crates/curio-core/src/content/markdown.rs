//! HTML → `CommonMark` via htmd's DOM walk (D5).
//!
//! The sketch's regex converter — which destroyed every element spanning
//! lines — is banned; conversion is a real DOM traversal. Input is
//! expected to be sanitized already (this module is called on stored
//! content at export time), but the skip list drops any script/style/
//! embed remnant defensively anyway.
//!
//! `header`, `footer` and `aside` are *not* skipped: whether one of them
//! is page chrome depends on where it sits in the page, and deciding that
//! is the ingest pipeline's job. Both doors ([`super::process`] and
//! [`super::process_full_page`]) drop the page-level ones before storage,
//! so what reaches export is content — a quote's `<footer>` attribution,
//! an article's or section's own header, an in-article callout — and
//! converting it is the point (#128).
//!
//! `nav` is skipped wherever it sits: it is a block of links into the
//! source site (breadcrumbs, a menu, an in-page table of contents whose
//! `#anchors` mean nothing in a note), and articles stored before the
//! full-page door stripped chrome may still carry it.
//!
//! Footnotes become `CommonMark` footnotes (`[^1]` / `[^1]: …`) and other
//! in-page links keep only their text (see `footnotes`). `pre` and
//! `code` have their own handlers (see `code`): code text is written
//! unescaped, so it must land where a renderer reads it as code — a fence
//! longer than any backtick run, or a single-line inline span — and never
//! as live raw HTML (#245). A `pre`/`code` `language-*` class becomes the
//! fence's info string.

use htmd::HtmlToMarkdown;

use super::{ContentError, code, footnotes};

/// Converts an HTML fragment to `CommonMark`.
///
/// # Errors
///
/// [`ContentError::Markdown`] if the DOM walk fails to serialize
/// (practically unreachable for parseable HTML).
pub fn to_markdown(html: &str) -> Result<String, ContentError> {
    let builder = HtmlToMarkdown::builder().skip_tags(vec![
        "script", "style", "iframe", "object", "embed", "form", "nav", "noscript", "svg", "canvas",
    ]);
    let converter = code::register(footnotes::register(builder, html)).build();
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

    /// The three #245 rows: escaped markup inside a `<pre>` or a
    /// block-spanning `<code>` stays code, never live raw HTML.
    #[test]
    fn escaped_markup_in_a_bare_pre_is_fenced() {
        assert_eq!(
            to_markdown("<pre>&lt;img src=x onerror=alert(1)&gt;</pre>").unwrap(),
            "```\n<img src=x onerror=alert(1)>\n```"
        );
        assert_eq!(
            to_markdown("<pre>&lt;script&gt;alert(1)&lt;/script&gt;</pre>").unwrap(),
            "```\n<script>alert(1)</script>\n```"
        );
    }

    #[test]
    fn a_block_spanning_code_is_one_inline_span() {
        assert_eq!(
            to_markdown("<code>&lt;script&gt;alert(1)&lt;/script&gt;<p>A</p></code>").unwrap(),
            "`<script>alert(1)</script> A`"
        );
        assert_eq!(
            to_markdown("<p><code>a\n\n&lt;b&gt;\r- &lt;i&gt;</code></p>").unwrap(),
            "`a <b> - <i>`"
        );
    }

    #[test]
    fn the_fence_outgrows_every_backtick_run() {
        let md = to_markdown("<pre>```\n&lt;script&gt;\n````</pre>").unwrap();
        assert_eq!(md, "`````\n```\n<script>\n````\n`````");
        let md = to_markdown("<pre><code>`x`</code></pre>").unwrap();
        assert_eq!(md, "```\n`x`\n```");
    }

    #[test]
    fn a_span_outgrows_every_backtick_run_and_pads_edge_backticks() {
        assert_eq!(to_markdown("<code>a`b</code>").unwrap(), "``a`b``");
        assert_eq!(to_markdown("<code>``x</code>").unwrap(), "``` ``x ```");
        assert_eq!(to_markdown("<code> </code>").unwrap(), "");
    }

    #[test]
    fn adjacent_spans_never_merge_their_delimiters() {
        // Unseparated, `a` + ` ``<x> ` would read as "`a```` ``<x> ```":
        // the merged run leaves every delimiter unpaired and `<x>` live.
        let md = to_markdown("<p><code>a</code><span><code>``&lt;x&gt;</code></span></p>").unwrap();
        assert_eq!(md, "`a` ``` ``<x> ```");
        let md = to_markdown("<p>x <code>a</code> <code>b</code></p>").unwrap();
        assert_eq!(md, "x `a` `b`");
        let md = to_markdown("<p>x<br><code>a</code></p>").unwrap();
        assert!(md.ends_with("\n`a`"), "{md:?}");
    }

    #[test]
    fn a_pre_that_cannot_stand_on_its_own_lines_is_a_span() {
        for (html, expected) in [
            ("<em><pre>&lt;x&gt;\ny</pre></em>", "*`<x> y`*"),
            ("<h2>T <pre>&lt;x&gt;</pre></h2>", "## T `<x>`"),
            (
                "<a href=\"https://e.com/\"><pre>&lt;x&gt;</pre></a>",
                "[`<x>`](https://e.com/)",
            ),
        ] {
            assert_eq!(to_markdown(html).unwrap(), expected, "{html}");
        }
    }

    #[test]
    fn a_pre_in_a_list_or_quote_stays_fenced() {
        let md = to_markdown("<ul><li><pre>&lt;x&gt;\ny</pre></li></ul>").unwrap();
        assert_eq!(md, "*   ```\n    <x>\n    y\n    ```");
        let md = to_markdown("<blockquote><pre>&lt;x&gt;</pre></blockquote>").unwrap();
        assert_eq!(md, "> ```\n> <x>\n> ```");
    }

    #[test]
    fn pre_text_keeps_breaks_and_block_lines() {
        let md = to_markdown("<pre>a<br>b<p>c</p>d</pre>").unwrap();
        assert_eq!(md, "```\na\nb\nc\nd\n```");
        assert_eq!(to_markdown("<pre>\n\n</pre>").unwrap(), "");
        let md = to_markdown("<pre>a&#13;&lt;x&gt;\r\nb</pre>").unwrap();
        assert_eq!(md, "```\na\n<x>\nb\n```");
        // The markdown property's minimal find against htmd's own handler:
        // block children used to turn the `pre` into raw HTML.
        let md = to_markdown("<pre><p>&lt;script&gt;alert(1)&lt;/script&gt;</p></pre>").unwrap();
        assert_eq!(md, "```\n<script>alert(1)</script>\n```");
    }

    #[test]
    fn code_fence_keeps_its_language() {
        let md = to_markdown("<pre><code class=\"language-rust\">let x = 1;</code></pre>").unwrap();
        assert!(md.starts_with("```rust\n"), "{md}");
        // A bare `<pre>` carries its own language; one that could end the
        // info string is dropped.
        let md = to_markdown("<pre class=\"language-c++\">x</pre>").unwrap();
        assert_eq!(md, "```c++\nx\n```");
        let md = to_markdown("<pre class=\"language-a`b\">x</pre>").unwrap();
        assert_eq!(md, "```\nx\n```");
    }

    #[test]
    fn sectioning_elements_left_in_stored_content_convert() {
        let md = to_markdown(
            "<article><header><h1>Title</h1></header><p>Body.</p>\
             <aside><p>Callout.</p></aside>\
             <blockquote><p>Quote.</p><footer>— Author</footer></blockquote></article>",
        )
        .unwrap();
        for kept in ["# Title", "Body.", "Callout.", "> Quote.", "— Author"] {
            assert!(md.contains(kept), "lost {kept:?}: {md}");
        }
    }

    #[test]
    fn nav_is_never_exported() {
        let md = to_markdown(
            "<article><nav><a href=\"https://e.com/blog\">Blog</a> › \
             <a href=\"https://e.com/tech\">Tech</a></nav>\
             <header><h1>Title</h1></header><p>Body.</p></article>",
        )
        .unwrap();
        assert!(!md.contains("Blog"), "breadcrumbs exported: {md}");
        assert!(md.contains("# Title"), "{md}");
        assert!(md.contains("Body."), "{md}");
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
