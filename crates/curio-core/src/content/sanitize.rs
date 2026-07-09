//! The ammonia gate: a strict allowlist, applied at ingest (D6).
//!
//! Keeps semantic HTML (headings, paragraphs, lists, tables, quotes,
//! code, figures, and the sectioning elements the extractor needs to see
//! to strip boilerplate) and `img` with http(s) sources. Everything
//! else — scripts, iframes, event-handler attributes, inline style,
//! `javascript:`/`data:` URLs, `<base>` — is gone before storage.
//!
//! One softening, applied *before* ammonia: recognized media embeds
//! (`<iframe>` from a small allowlist of video hosts, plus `<video>` /
//! `<audio>` with an `http(s)` source) are rewritten into a plain
//! `<a href="…">▶ Embedded media (host) ↗</a>` link, so the reader keeps
//! a way to reach the video instead of a silent gap. The rewrite only
//! ever emits an anchor whose `href` is the original source; ammonia
//! still runs afterward and judges that `href` with the same
//! `http`/`https`-only scheme filter as every other link, so a hostile
//! `src` can never survive as a live link — the transform cannot weaken
//! the gate. Unrecognized iframes stay fully dropped.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use ammonia::{Builder, UrlRelative};
use regex::{Captures, Regex};
use scraper::{Html, Selector};
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

/// Host suffixes whose `<iframe>` embeds are surfaced as a clickable
/// placeholder instead of being dropped. Everything else stays dropped.
/// Matched as a domain suffix, so `www.youtube.com` and
/// `player.vimeo.com` both count.
const EMBED_IFRAME_HOST_SUFFIXES: &[&str] = &[
    "youtube.com",
    "youtube-nocookie.com",
    "youtu.be",
    "vimeo.com",
];

/// Sanitizes raw HTML through the strict allowlist. `base_url` resolves
/// relative `href`/`src` before the http(s)-only scheme check; without
/// one, relative URLs are dropped. Recognized media embeds are rewritten
/// to placeholder links first (see the module docs), then judged by the
/// same gate as everything else.
#[must_use]
pub fn sanitize(raw_html: &str, base_url: Option<&str>) -> String {
    let rewritten = rewrite_embeds(raw_html, base_url);
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
    builder.clean(&rewritten).to_string()
}

/// Rewrites recognized `<iframe>`/`<video>`/`<audio>` embeds into
/// placeholder `<a>` links, in place, before ammonia runs. Anything not
/// rewritten is left untouched for ammonia to drop as before.
fn rewrite_embeds(html: &str, base_url: Option<&str>) -> String {
    let after_iframe = iframe_re().map_or_else(
        || html.to_owned(),
        |re| {
            re.replace_all(html, |caps: &Captures<'_>| {
                iframe_placeholder(&caps[0], base_url)
            })
            .into_owned()
        },
    );
    let after_video = media_re("video").map_or_else(
        || after_iframe.clone(),
        |re| {
            re.replace_all(&after_iframe, |caps: &Captures<'_>| {
                media_placeholder(&caps[0], base_url)
            })
            .into_owned()
        },
    );
    media_re("audio").map_or_else(
        || after_video.clone(),
        |re| {
            re.replace_all(&after_video, |caps: &Captures<'_>| {
                media_placeholder(&caps[0], base_url)
            })
            .into_owned()
        },
    )
}

/// The `<iframe>…</iframe>` matcher (case-insensitive, dot-matches-newline,
/// non-greedy). Compiled once; `None` only if the literal pattern fails to
/// compile, in which case the rewrite is skipped and ammonia drops embeds
/// as before.
fn iframe_re() -> Option<&'static Regex> {
    static RE: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?is)<iframe\b[^>]*>.*?</iframe\s*>").ok());
    RE.as_ref()
}

/// The `<video>`/`<audio>` element matcher for a given tag name.
fn media_re(tag: &str) -> Option<&'static Regex> {
    static VIDEO: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?is)<video\b[^>]*>.*?</video\s*>").ok());
    static AUDIO: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r"(?is)<audio\b[^>]*>.*?</audio\s*>").ok());
    match tag {
        "video" => VIDEO.as_ref(),
        "audio" => AUDIO.as_ref(),
        _ => None,
    }
}

/// Turns a matched `<iframe>` block into a placeholder link when its host
/// is allowlisted; otherwise returns the block unchanged (ammonia drops it).
/// Returns an owned `String` because a `replace_all` callback may not borrow
/// from its `Captures` argument.
fn iframe_placeholder(block: &str, base_url: Option<&str>) -> String {
    let Some(src) = first_attr(block, "iframe[src]", "src") else {
        return block.to_owned();
    };
    match embed_host(&src, base_url) {
        Some(host) if is_allowlisted_embed_host(&host) => placeholder_html(&src, &host),
        _ => block.to_owned(),
    }
}

/// Turns a matched `<video>`/`<audio>` block into a placeholder link when a
/// resolvable `http(s)` source is present (on the element or a child
/// `<source>`); otherwise returns the block unchanged.
fn media_placeholder(block: &str, base_url: Option<&str>) -> String {
    let src = first_attr(block, "video[src], audio[src]", "src")
        .or_else(|| first_attr(block, "source[src]", "src"));
    let Some(src) = src else {
        return block.to_owned();
    };
    match embed_host(&src, base_url) {
        Some(host) => placeholder_html(&src, &host),
        None => block.to_owned(),
    }
}

/// Parses a small HTML block and returns the first matching element's
/// attribute value, if any.
fn first_attr(html: &str, selector: &str, attr: &str) -> Option<String> {
    let doc = Html::parse_fragment(html);
    let selector = Selector::parse(selector).ok()?;
    doc.select(&selector)
        .next()?
        .value()
        .attr(attr)
        .map(str::to_owned)
}

/// The lowercase host of a source URL, resolving relative sources against
/// `base_url`. `None` when there is no host (e.g. a `javascript:` source,
/// or a relative source with no base) — the caller then emits no link.
fn embed_host(src: &str, base_url: Option<&str>) -> Option<String> {
    if let Ok(url) = Url::parse(src) {
        return url.host_str().map(str::to_ascii_lowercase);
    }
    let base = Url::parse(base_url?).ok()?;
    base.join(src).ok()?.host_str().map(str::to_ascii_lowercase)
}

/// Whether `host` matches (as a domain suffix) one of the allowlisted
/// embed hosts.
fn is_allowlisted_embed_host(host: &str) -> bool {
    EMBED_IFRAME_HOST_SUFFIXES.iter().any(|&suffix| {
        host == suffix
            || host
                .strip_suffix(suffix)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// The placeholder anchor. `href` is the raw source (ammonia validates its
/// scheme and resolves it against the base); `host` is display-only.
fn placeholder_html(src: &str, host: &str) -> String {
    format!(
        "<p><a href=\"{href}\">▶ Embedded media ({host}) ↗</a></p>",
        href = escape_attr(src),
        host = escape_text(host),
    )
}

/// Minimal HTML-attribute escaping for a value placed inside `"…"`.
fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Minimal HTML text escaping for character data.
fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
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

    #[test]
    fn youtube_iframe_becomes_a_clickable_placeholder() {
        let out = sanitize(
            "<p>Watch:</p><iframe src=\"https://www.youtube-nocookie.com/embed/abc123\" \
             width=\"560\" height=\"315\"></iframe>",
            None,
        );
        assert!(!out.contains("iframe"), "iframe tag survived: {out}");
        assert!(
            out.contains("href=\"https://www.youtube-nocookie.com/embed/abc123\""),
            "placeholder href missing: {out}"
        );
        assert!(
            out.contains("Embedded media"),
            "placeholder text missing: {out}"
        );
        assert!(
            out.contains("www.youtube-nocookie.com"),
            "host label missing: {out}"
        );
        assert!(
            out.contains("<p>Watch:</p>"),
            "surrounding content dropped: {out}"
        );
    }

    #[test]
    fn vimeo_iframe_becomes_a_placeholder() {
        let out = sanitize(
            "<iframe src=\"https://player.vimeo.com/video/12345\"></iframe>",
            None,
        );
        assert!(
            out.contains("href=\"https://player.vimeo.com/video/12345\""),
            "{out}"
        );
        assert!(out.contains("player.vimeo.com"), "{out}");
    }

    #[test]
    fn unknown_iframe_host_is_still_dropped() {
        // The allowlist is what keeps the hostile-corpus gate green: an
        // iframe to an arbitrary host must leave no trace, no link.
        let out = sanitize(
            "<iframe src=\"https://ads.evil.example/track\"></iframe><p>ok</p>",
            None,
        );
        assert!(!out.contains("iframe"), "{out}");
        assert!(
            !out.contains("ads.evil.example"),
            "hostile host leaked: {out}"
        );
        assert!(
            !out.contains("Embedded media"),
            "unexpected placeholder: {out}"
        );
        assert!(out.contains("<p>ok</p>"), "{out}");
    }

    #[test]
    fn video_element_becomes_a_placeholder() {
        let out = sanitize("<video src=\"https://cdn.example/v.mp4\"></video>", None);
        assert!(!out.contains("<video"), "{out}");
        assert!(out.contains("href=\"https://cdn.example/v.mp4\""), "{out}");
        assert!(out.contains("cdn.example"), "{out}");
    }

    #[test]
    fn video_with_a_source_child_becomes_a_placeholder() {
        let out = sanitize(
            "<video controls><source src=\"https://cdn.example/v.mp4\" type=\"video/mp4\"></video>",
            None,
        );
        assert!(out.contains("href=\"https://cdn.example/v.mp4\""), "{out}");
        assert!(out.contains("Embedded media"), "{out}");
    }

    #[test]
    fn audio_element_becomes_a_placeholder() {
        let out = sanitize("<audio src=\"https://cdn.example/a.mp3\"></audio>", None);
        assert!(!out.contains("<audio"), "{out}");
        assert!(out.contains("href=\"https://cdn.example/a.mp3\""), "{out}");
    }

    #[test]
    fn a_non_http_media_source_yields_no_live_link() {
        // `javascript:` has no host → no placeholder is emitted, and even
        // if one were, ammonia's scheme filter would strip the href.
        let out = sanitize("<video src=\"javascript:alert(1)\"></video>ok", None);
        assert!(!out.contains("javascript"), "{out}");
        assert!(!out.contains("Embedded media"), "{out}");
        assert!(out.contains("ok"), "{out}");
    }

    #[test]
    fn a_relative_embed_src_resolves_against_the_base() {
        let out = sanitize(
            "<iframe src=\"/embed/xyz\"></iframe>",
            Some("https://www.youtube.com/watch?v=xyz"),
        );
        assert!(out.contains("https://www.youtube.com/embed/xyz"), "{out}");
        assert!(out.contains("Embedded media"), "{out}");
    }
}
