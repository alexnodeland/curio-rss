//! The ammonia gate: a strict allowlist, applied at ingest (D6).
//!
//! Keeps semantic HTML (headings, paragraphs, lists, tables, quotes,
//! code, figures, and the sectioning elements the extractor needs to see
//! to strip boilerplate) and `img` with http(s) sources. Everything
//! else — scripts, iframes, event-handler attributes, inline style,
//! `javascript:`/`data:` URLs, `<base>` — is gone before storage.
//!
//! The few attributes beyond links and image sources are each value-filtered
//! (see [`filter_attribute`]), never passed through verbatim:
//!
//! - `id` on any element, restricted to a plain token and prefixed with
//!   [`ID_PREFIX`] so author ids can never clobber the reader's own DOM;
//!   fragment-only `href`s (`#fn1`) are kept, rewritten to the same prefix,
//!   so footnotes and in-page links stay in-page instead of being
//!   absolutized into a link back to the source site;
//! - `dir` (`ltr`/`rtl`/`auto`) and `lang` (a language tag) for RTL and
//!   mixed-language feeds; `ol[start]` (an integer);
//! - `class` on `pre`/`code`, reduced to its single `language-*` token —
//!   the fence language htmd writes into exported markdown.
//!
//! Two softenings run *before* ammonia, and neither can weaken it:
//!
//! - **Lazy images** (see `lazy_images`): a lazy loader's real URL
//!   (`data-src`, `data-lazy-src`, the best `srcset` candidate) is promoted
//!   to `src`, and a `<noscript>` image fallback is lifted out when its
//!   visible sibling has no fetchable source. Every promoted `src` is still
//!   judged by ammonia's `http`/`https`-only scheme filter.
//! - **Embeds** (see `embeds`): visible `<iframe>`s from known providers or
//!   with an embed-shaped URL, and `<video>`/`<audio>` with an `http(s)`
//!   source, become a plain placeholder link to the original source, which
//!   ammonia then judges like every other link. Arbitrary iframes stay
//!   fully dropped.

use std::borrow::Cow;
use std::collections::{HashMap, HashSet};

use ammonia::{Builder, UrlRelative, UrlRelativeEvaluate};
use url::Url;

use super::embeds::rewrite_embeds;
use super::lazy_images::{recover_lazy_images, resolve_http};

/// The prefix every surviving `id` (and every fragment-only `href`) carries.
/// Author ids live in their own namespace, so `id="main-content"` or
/// `id="location"` in a feed can never collide with — or clobber — the
/// reader's own elements and globals. The namespace is deliberately *not*
/// `curio-`: the desktop app names its own DOM ids that way
/// (`curio-custom-themes`, …), so the app must never use an id starting
/// with this prefix — every such id belongs to feed content.
pub(crate) const ID_PREFIX: &str = "user-content-";

/// Longest `id` / fragment token kept (real footnote ids are short).
const MAX_ID_LEN: usize = 128;

/// Tags whose entire content is dropped (not just the tag).
const CLEAN_CONTENT_TAGS: &[&str] = &[
    "script", "style", "iframe", "frame", "frameset", "object", "embed", "applet", "form", "input",
    "button", "select", "textarea", "noscript", "canvas", "audio", "video", "template", "svg",
    "math", "title", "head",
];

/// The semantic allowlist. Sectioning elements (`nav`, `header`,
/// `footer`, `aside`, …) are deliberately *kept* here so the
/// pipeline after it still sees the document's structure: it tells
/// page chrome (a `nav`, a page-level `header`) from the content's own
/// sectioning (a quote's `<footer>`, a section's `<header>`) by position,
/// and it can only judge what it can still recognize.
const ALLOWED_TAGS: &[&str] = &[
    "a",
    "abbr",
    "article",
    "aside",
    "b",
    "bdi",
    "bdo",
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

/// Attributes allowed on every element — each one value-filtered by
/// [`filter_attribute`].
const GENERIC_ATTRIBUTES: &[&str] = &["id", "dir", "lang"];

/// Sanitizes raw HTML through the strict allowlist. `base_url` resolves
/// relative `href`/`src` before the http(s)-only scheme check; without
/// one, relative URLs are dropped (fragment-only `href`s are kept either
/// way). Lazy images are recovered and recognized embeds rewritten to
/// placeholder links first (see the module docs), then judged by the same
/// gate as everything else.
#[must_use]
pub fn sanitize(raw_html: &str, base_url: Option<&str>) -> String {
    let base = base_url.and_then(|base| Url::parse(base).ok());
    let rewritten = soften(raw_html, base.as_ref());
    let mut builder = Builder::default();
    builder
        .tags(ALLOWED_TAGS.iter().copied().collect::<HashSet<_>>())
        .clean_content_tags(CLEAN_CONTENT_TAGS.iter().copied().collect::<HashSet<_>>())
        .tag_attributes(tag_attributes())
        .generic_attributes(GENERIC_ATTRIBUTES.iter().copied().collect::<HashSet<_>>())
        .attribute_filter(filter_attribute)
        .url_schemes(["http", "https"].into_iter().collect::<HashSet<_>>())
        .url_relative(UrlRelative::Custom(Box::new(RelativeUrls { base })))
        .strip_comments(true);
    builder.clean(&rewritten).to_string()
}

/// The two pre-gate softenings (see the module docs): lazy images get
/// their real `src`, recognized embeds become placeholder links. Both only
/// ever hand ammonia an `http(s)` candidate to judge. Idempotent, so the
/// full-page path can run it before readability extraction (which drops
/// most iframes and every `<noscript>`) and [`sanitize`] again after.
pub(super) fn soften(raw_html: &str, base: Option<&Url>) -> String {
    let recovered = recover_lazy_images(raw_html, base);
    rewrite_embeds(&recovered, base)
}

/// The relative-URL policy: a fragment-only reference stays in-page
/// (rewritten into the [`ID_PREFIX`] namespace, like the `id` it targets);
/// anything else resolves against the article base and must come out
/// `http(s)`. Without a base, non-fragment relative URLs are dropped.
struct RelativeUrls {
    base: Option<Url>,
}

impl UrlRelativeEvaluate<'_> for RelativeUrls {
    fn evaluate<'url>(&self, url: &'url str) -> Option<Cow<'url, str>> {
        if let Some(fragment) = url.trim().strip_prefix('#') {
            return prefixed_id(fragment).map(|id| Cow::Owned(format!("#{id}")));
        }
        resolve_http(url, self.base.as_ref()).map(|resolved| Cow::Owned(resolved.into()))
    }
}

/// Value filter for the non-URL attributes the allowlist admits. Returning
/// `None` drops the attribute. (`href`/`src` are judged by the scheme
/// filter and [`RelativeUrls`]; they pass through here unchanged.)
fn filter_attribute<'a>(element: &str, attribute: &str, value: &'a str) -> Option<Cow<'a, str>> {
    match attribute {
        "id" => prefixed_id(value).map(Cow::Owned),
        "dir" => {
            let dir = value.trim().to_ascii_lowercase();
            matches!(dir.as_str(), "ltr" | "rtl" | "auto").then_some(Cow::Owned(dir))
        }
        "lang" => is_language_tag(value.trim()).then(|| Cow::Owned(value.trim().to_owned())),
        "start" if element == "ol" => value
            .trim()
            .parse::<i32>()
            .ok()
            .map(|start| Cow::Owned(start.to_string())),
        "class" if matches!(element, "pre" | "code") => language_class(value).map(Cow::Owned),
        "class" | "start" => None,
        _ => Some(Cow::Borrowed(value)),
    }
}

/// `raw` as a prefixed id: a non-empty token of ASCII letters, digits and
/// `-_.:` (what footnote generators emit: `fn1`, `fn:1`, `fnref-2`), with
/// [`ID_PREFIX`] prepended unless already present (re-sanitizing is
/// idempotent). Anything else — whitespace, quotes, markup — is dropped.
fn prefixed_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let is_token = !raw.is_empty()
        && raw.len() <= MAX_ID_LEN
        && raw
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'));
    if !is_token {
        return None;
    }
    Some(if raw.starts_with(ID_PREFIX) {
        raw.to_owned()
    } else {
        format!("{ID_PREFIX}{raw}")
    })
}

/// A BCP 47-shaped language tag: alphanumeric subtags of 1–8 characters
/// joined by `-`, starting with a letter (`en`, `ar-EG`, `zh-Hant-TW`).
fn is_language_tag(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 35
        && value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value.split('-').all(|sub| {
            (1..=8).contains(&sub.len()) && sub.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

/// The fence language of a `pre`/`code` class list, normalized to
/// `language-<name>` (Prism's `lang-<name>` alias included). Every other
/// class token is dropped; `None` when no language token is present.
fn language_class(value: &str) -> Option<String> {
    value.split_ascii_whitespace().find_map(|token| {
        let name = token
            .strip_prefix("language-")
            .or_else(|| token.strip_prefix("lang-"))?;
        let valid = !name.is_empty()
            && name.len() <= 32
            && name.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'+' | b'#' | b'.')
            });
        valid.then(|| format!("language-{name}"))
    })
}

/// Minimal HTML-attribute escaping for a value placed inside `"…"`.
pub(super) fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

fn tag_attributes() -> HashMap<&'static str, HashSet<&'static str>> {
    let mut attrs: HashMap<&'static str, HashSet<&'static str>> = HashMap::new();
    attrs.insert("a", ["href", "title"].into_iter().collect());
    attrs.insert("img", ["src", "alt", "title"].into_iter().collect());
    attrs.insert("time", ["datetime"].into_iter().collect());
    attrs.insert("td", ["colspan", "rowspan"].into_iter().collect());
    attrs.insert("th", ["colspan", "rowspan"].into_iter().collect());
    attrs.insert("ol", ["start"].into_iter().collect());
    attrs.insert("pre", ["class"].into_iter().collect());
    attrs.insert("code", ["class"].into_iter().collect());
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

    #[test]
    fn fragment_links_and_ids_share_the_prefixed_namespace() {
        let out = sanitize(
            "<p>x<sup id=\"fnref:1\"><a href=\"#fn:1\">1</a></sup></p>\
             <ol><li id=\"fn:1\">note <a href=\"#fnref:1\">↩</a></li></ol>",
            Some("https://example.com/post"),
        );
        assert!(out.contains("<sup id=\"user-content-fnref:1\">"), "{out}");
        assert!(out.contains("href=\"#user-content-fn:1\""), "{out}");
        assert!(out.contains("<li id=\"user-content-fn:1\">"), "{out}");
        assert!(out.contains("href=\"#user-content-fnref:1\""), "{out}");
        assert!(
            !out.contains("example.com/post#"),
            "fragment was absolutized: {out}"
        );
    }

    #[test]
    fn fragment_links_survive_without_a_base() {
        let out = sanitize("<a href=\"#fn1\">1</a><a href=\"/rel\">r</a>", None);
        assert!(out.contains("href=\"#user-content-fn1\""), "{out}");
        assert!(!out.contains("/rel"), "{out}");
    }

    #[test]
    fn hostile_ids_and_fragments_are_dropped() {
        let out = sanitize(
            "<p id=\"a b\">1</p><p id=\"x&quot;onclick=&quot;y\">2</p><p id=\"\">3</p>\
             <a href=\"#\">4</a><a href=\"#a(1)\">5</a><a href=\"#x y\">6</a>",
            None,
        );
        assert!(!out.contains("id="), "{out}");
        assert!(!out.contains("href="), "{out}");
    }

    #[test]
    fn ids_are_prefixed_once() {
        let once = sanitize("<p id=\"location\">x</p>", None);
        assert_eq!(once, "<p id=\"user-content-location\">x</p>");
        assert_eq!(sanitize(&once, None), once, "re-sanitizing is idempotent");
    }

    #[test]
    fn direction_and_language_survive_filtered() {
        let out = sanitize(
            "<p dir=\"RTL\" lang=\"ar-EG\">مرحبا</p><bdo dir=\"ltr\">x</bdo>\
             <p dir=\"sideways\" lang=\"en&quot;x\">y</p><p lang=\"1en\">z</p>",
            None,
        );
        assert!(
            out.contains("<p dir=\"rtl\" lang=\"ar-EG\">مرحبا</p>"),
            "{out}"
        );
        assert!(out.contains("<bdo dir=\"ltr\">x</bdo>"), "{out}");
        assert!(out.contains("<p>y</p>"), "{out}");
        assert!(out.contains("<p>z</p>"), "{out}");
    }

    #[test]
    fn ordered_list_start_survives_as_an_integer() {
        assert_eq!(
            sanitize("<ol start=\" 4 \"><li>d</li></ol>", None),
            "<ol start=\"4\"><li>d</li></ol>"
        );
        assert_eq!(
            sanitize("<ol start=\"4 onclick=x\"><li>d</li></ol>", None),
            "<ol><li>d</li></ol>"
        );
        assert_eq!(
            sanitize("<ul start=\"4\"><li>d</li></ul>", None),
            "<ul><li>d</li></ul>"
        );
    }

    #[test]
    fn code_class_is_reduced_to_its_language() {
        assert_eq!(
            sanitize(
                "<pre class=\"highlight\"><code class=\"hljs language-rust evil\">x</code></pre>",
                None
            ),
            "<pre><code class=\"language-rust\">x</code></pre>"
        );
        assert_eq!(
            sanitize("<pre class=\"lang-c++\"><code>x</code></pre>", None),
            "<pre class=\"language-c++\"><code>x</code></pre>"
        );
        assert_eq!(
            sanitize("<code class=\"language-a&quot;b\">x</code>", None),
            "<code>x</code>"
        );
        assert_eq!(
            sanitize("<p class=\"language-rust\">x</p>", None),
            "<p>x</p>"
        );
    }

    #[test]
    fn lazy_images_reach_the_gate_with_their_real_source() {
        let out = sanitize(
            "<img src=\"data:image/gif;base64,R0lGOD\" data-src=\"/real.jpg\" alt=\"a\">\
             <noscript><img src=\"/real.jpg\" alt=\"a\"></noscript>",
            Some("https://example.com/post"),
        );
        assert_eq!(out, "<img src=\"https://example.com/real.jpg\" alt=\"a\">");
    }

    #[test]
    fn a_hostile_lazy_source_is_still_judged_by_the_gate() {
        let out = sanitize(
            "<img data-src=\"javascript:alert(1)\" srcset=\"data:text/html,x 2x\" alt=\"a\">",
            None,
        );
        assert_eq!(out, "<img alt=\"a\">");
    }
}
