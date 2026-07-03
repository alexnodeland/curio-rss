//! The ammonia gate: a strict allowlist, applied at ingest (D6).
//!
//! Keeps semantic HTML (headings, paragraphs, lists, tables, quotes,
//! code, figures, and the sectioning elements the extractor needs to see
//! to strip boilerplate) and `img` with http(s) sources. Everything
//! else — scripts, iframes, event-handler attributes, inline style,
//! `javascript:`/`data:` URLs, `<base>` — is gone before storage.

use std::collections::{HashMap, HashSet};

use ammonia::{Builder, UrlRelative};
use url::Url;

/// Tags whose entire content is dropped (not just the tag).
const CLEAN_CONTENT_TAGS: &[&str] = &[
    "script", "style", "iframe", "frame", "frameset", "object", "embed", "applet", "form", "input",
    "button", "select", "textarea", "noscript", "canvas", "audio", "video", "template", "svg",
    "math", "title", "head",
];

/// The semantic allowlist. Sectioning elements (`nav`, `header`,
/// `footer`, `aside`, …) are deliberately *kept* here so the
/// readability extractor still sees the document's structure — it is
/// the extractor's job to drop boilerplate, and it can only drop what
/// it can still recognize.
const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "article",
    "aside",
    "b",
    "blockquote",
    "br",
    "caption",
    "cite",
    "code",
    "dd",
    "del",
    "details",
    "dfn",
    "div",
    "dl",
    "dt",
    "em",
    "figcaption",
    "figure",
    "footer",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "header",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "main",
    "mark",
    "nav",
    "ol",
    "p",
    "pre",
    "q",
    "s",
    "samp",
    "section",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "time",
    "tr",
    "u",
    "ul",
    "var",
    "wbr",
];

/// Sanitizes raw HTML through the strict allowlist. `base_url` resolves
/// relative `href`/`src` before the http(s)-only scheme check; without
/// one, relative URLs are dropped.
#[must_use]
pub fn sanitize(raw_html: &str, base_url: Option<&str>) -> String {
    let mut builder = Builder::default();
    builder
        .tags(ALLOWED_TAGS.iter().copied().collect::<HashSet<_>>())
        .clean_content_tags(CLEAN_CONTENT_TAGS.iter().copied().collect::<HashSet<_>>())
        .tag_attributes(tag_attributes())
        .generic_attributes(HashSet::new())
        .url_schemes(["http", "https"].into_iter().collect::<HashSet<_>>())
        .strip_comments(true);
    match base_url.and_then(|base| Url::parse(base).ok()) {
        Some(base) => builder.url_relative(UrlRelative::RewriteWithBase(base)),
        None => builder.url_relative(UrlRelative::Deny),
    };
    builder.clean(raw_html).to_string()
}

fn tag_attributes() -> HashMap<&'static str, HashSet<&'static str>> {
    let mut attrs: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    attrs.insert("a", ["href", "title"].into_iter().collect());
    attrs.insert("img", ["src", "alt", "title"].into_iter().collect());
    attrs.insert("time", ["datetime"].into_iter().collect());
    attrs.insert("td", ["colspan", "rowspan"].into_iter().collect());
    attrs.insert("th", ["colspan", "rowspan"].into_iter().collect());
    attrs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_removed_with_their_content() {
        let out = sanitize("<p>ok</p><script>alert(1)</script>", None);
        assert!(!out.contains("script"));
        assert!(!out.contains("alert"));
        assert!(out.contains("<p>ok</p>"));
    }

    #[test]
    fn event_handler_attributes_are_stripped() {
        let out = sanitize(
            "<p onmouseover=\"alert(1)\">x</p><img src=\"https://e.com/i.png\" onerror=\"alert(1)\">",
            None,
        );
        assert!(!out.contains("onmouseover"));
        assert!(!out.contains("onerror"));
        assert!(out.contains("<img src=\"https://e.com/i.png\">"));
    }

    #[test]
    fn javascript_and_data_urls_are_dropped() {
        let out = sanitize(
            "<a href=\"javascript:alert(1)\">a</a>\
             <a href=\"data:text/html,x\">b</a>\
             <img src=\"data:image/png;base64,xxxx\">",
            None,
        );
        assert!(!out.contains("javascript:"));
        assert!(!out.contains("data:"));
    }

    #[test]
    fn inline_style_and_style_elements_are_stripped() {
        let out = sanitize(
            "<style>p{color:red}</style><p style=\"color:red\">x</p>",
            None,
        );
        assert!(!out.contains("style"));
        assert!(!out.contains("color"));
    }

    #[test]
    fn base_iframe_svg_and_forms_are_gone() {
        let out = sanitize(
            "<base href=\"https://evil.example/\">\
             <iframe src=\"https://evil.example/\"></iframe>\
             <svg onload=\"alert(1)\"><circle r=\"1\"/></svg>\
             <form action=\"javascript:alert(1)\"><input value=\"x\"></form>ok",
            None,
        );
        assert!(!out.contains("base"));
        assert!(!out.contains("iframe"));
        assert!(!out.contains("svg"));
        assert!(!out.contains("form"));
        assert!(!out.contains("onload"));
        assert!(out.contains("ok"));
    }

    #[test]
    fn relative_urls_resolve_against_the_article_base() {
        let out = sanitize(
            "<img src=\"/images/x.png\"><a href=\"other\">o</a>",
            Some("https://example.com/posts/one"),
        );
        assert!(out.contains("https://example.com/images/x.png"));
        assert!(out.contains("https://example.com/posts/other"));
    }

    #[test]
    fn relative_urls_without_a_base_are_dropped() {
        let out = sanitize("<img src=\"/images/x.png\" alt=\"kept\">", None);
        assert!(!out.contains("src"));
        assert!(out.contains("alt=\"kept\""));
    }

    #[test]
    fn semantic_structure_survives() {
        let input = "<article><h2>t</h2><p>a <em>b</em> <code>c</code></p>\
                     <pre><code>x = 1\ny = 2</code></pre>\
                     <table><tr><td colspan=\"2\">cell</td></tr></table></article>";
        let out = sanitize(input, None);
        for needle in ["<article>", "<h2>", "<em>", "<pre>", "colspan=\"2\""] {
            assert!(out.contains(needle), "{needle} missing from {out}");
        }
    }
}
