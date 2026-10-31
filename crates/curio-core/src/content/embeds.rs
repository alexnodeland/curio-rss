//! Embed placeholders, applied to raw HTML *before* the sanitize gate.
//!
//! `<iframe>`s from known media/content providers, or whose URL is
//! embed-shaped (an `embed`/`player`/`widget` path segment or host label —
//! Mastodon's `/@user/1/embed`, a maps `/embed`), plus `<video>` / `<audio>`
//! with an `http(s)` source, are rewritten into a plain
//! `<a href="…">▶ Embedded media (host) ↗</a>` (or `Embedded content`) link,
//! so the reader keeps a way to reach them instead of a silent gap.
//!
//! The rewrite only ever emits an anchor whose `href` is the original
//! `http(s)` source; ammonia still runs afterward and judges that `href`
//! with the same `http`/`https`-only scheme filter as every other link, so a
//! hostile `src` can never survive as a live link — the transform cannot
//! weaken the gate. Invisible frames (`hidden`, 0–1px, `display:none`) and
//! arbitrary non-embed iframes are left untouched, for ammonia to drop.

use std::collections::HashMap;
use std::sync::LazyLock;

use regex::{Captures, Regex};
use scraper::{Html, Selector};
use url::Url;

use super::lazy_images::resolve_http;
use super::sanitize::escape_attr;

/// Media providers whose `<iframe>` embeds become an "Embedded media"
/// placeholder. Matched as a domain suffix, so `www.youtube.com`,
/// `player.vimeo.com` and `w.soundcloud.com` all count.
const MEDIA_EMBED_HOST_SUFFIXES: &[&str] = &[
    "youtube.com",
    "youtube-nocookie.com",
    "youtu.be",
    "vimeo.com",
    "spotify.com",
    "soundcloud.com",
];

/// Content providers whose `<iframe>` embeds become an "Embedded content"
/// placeholder even when the URL is not embed-shaped.
const CONTENT_EMBED_HOST_SUFFIXES: &[&str] = &["codepen.io", "bsky.app"];

/// A path segment or leading host label that marks an iframe URL as an
/// embed from a provider we don't list (Mastodon's `/@user/1/embed`,
/// `maps…/embed`, `embed.example.org`, `…/player/…`).
const EMBED_URL_MARKERS: &[&str] = &["embed", "embeds", "player", "widget", "widgets"];

/// Rewrites recognized `<iframe>`/`<video>`/`<audio>` embeds into
/// placeholder `<a>` links, in place, before ammonia runs. Anything not
/// rewritten is left untouched for ammonia to drop as before.
pub(super) fn rewrite_embeds(html: &str, base: Option<&Url>) -> String {
    let after_iframe = iframe_re().map_or_else(
        || html.to_owned(),
        |re| {
            re.replace_all(html, |caps: &Captures<'_>| {
                iframe_placeholder(&caps[0], base)
            })
            .into_owned()
        },
    );
    let after_video = media_re("video").map_or_else(
        || after_iframe.clone(),
        |re| {
            re.replace_all(&after_iframe, |caps: &Captures<'_>| {
                media_placeholder(&caps[0], base)
            })
            .into_owned()
        },
    );
    media_re("audio").map_or_else(
        || after_video.clone(),
        |re| {
            re.replace_all(&after_video, |caps: &Captures<'_>| {
                media_placeholder(&caps[0], base)
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

/// What kind of placeholder an embed becomes (its label).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmbedKind {
    /// Video/audio: "▶ Embedded media (host) ↗".
    Media,
    /// Anything else: "Embedded content (host) ↗".
    Content,
}

/// Turns a matched `<iframe>` block into a placeholder link when it is a
/// visible embed from a known provider or with an embed-shaped URL;
/// otherwise returns the block unchanged (ammonia drops it). Returns an
/// owned `String` because a `replace_all` callback may not borrow from its
/// `Captures` argument.
fn iframe_placeholder(block: &str, base: Option<&Url>) -> String {
    let Some(attrs) = first_attrs(block, "iframe") else {
        return block.to_owned();
    };
    if is_invisible_frame(&attrs) {
        return block.to_owned();
    }
    let Some(src) = attrs.get("src") else {
        return block.to_owned();
    };
    let Some(url) = resolve_http(src, base) else {
        return block.to_owned();
    };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return block.to_owned();
    };
    let kind = if host_matches(&host, MEDIA_EMBED_HOST_SUFFIXES) {
        EmbedKind::Media
    } else if host_matches(&host, CONTENT_EMBED_HOST_SUFFIXES) || is_embed_shaped(&url) {
        EmbedKind::Content
    } else {
        return block.to_owned();
    };
    placeholder_html(src, &host, kind)
}

/// Turns a matched `<video>`/`<audio>` block into a placeholder link when a
/// resolvable `http(s)` source is present (on the element or a child
/// `<source>`); otherwise returns the block unchanged.
fn media_placeholder(block: &str, base: Option<&Url>) -> String {
    let src = first_attrs(block, "video[src], audio[src]")
        .or_else(|| first_attrs(block, "source[src]"))
        .and_then(|attrs| attrs.get("src").cloned());
    let Some(src) = src else {
        return block.to_owned();
    };
    match resolve_http(&src, base).and_then(|url| url.host_str().map(str::to_ascii_lowercase)) {
        Some(host) => placeholder_html(&src, &host, EmbedKind::Media),
        None => block.to_owned(),
    }
}

/// Parses a small HTML block and returns the first matching element's
/// attributes (names lowercase, as the parser normalizes them).
fn first_attrs(html: &str, selector: &str) -> Option<HashMap<String, String>> {
    let doc = Html::parse_fragment(html);
    let selector = Selector::parse(selector).ok()?;
    let element = doc.select(&selector).next()?;
    Some(
        element
            .value()
            .attrs()
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
            .collect(),
    )
}

/// A frame nobody can see — a tracker, not content: `hidden`, a 0/1px
/// dimension, or an inline `display:none` / `visibility:hidden`.
fn is_invisible_frame(attrs: &HashMap<String, String>) -> bool {
    let tiny = |name: &str| {
        attrs.get(name).is_some_and(|value| {
            let digits: String = value
                .trim()
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse::<u32>().is_ok_and(|px| px <= 1)
        })
    };
    let hidden_style = attrs.get("style").is_some_and(|style| {
        let style: String = style
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>()
            .to_ascii_lowercase();
        style.contains("display:none") || style.contains("visibility:hidden")
    });
    attrs.contains_key("hidden") || tiny("width") || tiny("height") || hidden_style
}

/// Whether an iframe URL looks like an embed: an `embed`/`player`/`widget`
/// path segment or leading host label.
fn is_embed_shaped(url: &Url) -> bool {
    let is_marker = |part: &str| EMBED_URL_MARKERS.contains(&part.to_ascii_lowercase().as_str());
    url.path_segments()
        .is_some_and(|mut segments| segments.any(is_marker))
        || url
            .host_str()
            .and_then(|host| host.split('.').next())
            .is_some_and(is_marker)
}

/// Whether `host` matches (as a domain suffix) one of `suffixes`.
fn host_matches(host: &str, suffixes: &[&str]) -> bool {
    suffixes.iter().any(|&suffix| {
        host == suffix
            || host
                .strip_suffix(suffix)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

/// The placeholder anchor. `href` is the raw source (ammonia validates its
/// scheme and resolves it against the base); `host` is display-only.
fn placeholder_html(src: &str, host: &str, kind: EmbedKind) -> String {
    let label = match kind {
        EmbedKind::Media => "▶ Embedded media",
        EmbedKind::Content => "Embedded content",
    };
    format!(
        "<p><a href=\"{href}\">{label} ({host}) ↗</a></p>",
        href = escape_attr(src),
        host = escape_text(host),
    )
}

/// Minimal HTML text escaping for character data.
fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use crate::content::sanitize;

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
        // The allowlist is what keeps the hostile-corpus gate green: a
        // non-embed iframe to an arbitrary host must leave no trace, no link.
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

    #[test]
    fn spotify_and_soundcloud_iframes_are_media_placeholders() {
        for src in [
            "https://open.spotify.com/embed/track/4uLU6hMCjMI75M1A2tKUQC",
            "https://w.soundcloud.com/player/?url=https%3A//api.soundcloud.com/tracks/1",
        ] {
            let out = sanitize(&format!("<iframe src=\"{src}\"></iframe>"), None);
            assert!(out.contains("▶ Embedded media"), "{out}");
            assert!(!out.contains("<iframe"), "{out}");
        }
    }

    #[test]
    fn codepen_bluesky_and_mastodon_iframes_are_content_placeholders() {
        for (src, host) in [
            ("https://codepen.io/team/pen/abc", "codepen.io"),
            (
                "https://embed.bsky.app/embed/did:plc:x/app.bsky.feed.post/3k",
                "embed.bsky.app",
            ),
            (
                "https://mastodon.social/@user/1234/embed",
                "mastodon.social",
            ),
        ] {
            let out = sanitize(&format!("<iframe src=\"{src}\"></iframe>"), None);
            assert!(
                out.contains(&format!("Embedded content ({host}) ↗")),
                "{src}: {out}"
            );
            assert!(out.contains(&format!("href=\"{src}\"")), "{src}: {out}");
        }
    }

    #[test]
    fn an_embed_shaped_iframe_from_an_unknown_host_becomes_a_link() {
        let out = sanitize(
            "<iframe src=\"https://maps.example.org/embed?pb=1\" width=\"600\"></iframe>",
            None,
        );
        assert!(
            out.contains("<a href=\"https://maps.example.org/embed?pb=1\""),
            "{out}"
        );
        assert!(out.contains("Embedded content (maps.example.org)"), "{out}");
    }

    #[test]
    fn invisible_or_non_http_embed_frames_stay_dropped() {
        for block in [
            "<iframe src=\"https://www.youtube.com/embed/x\" width=\"1\" height=\"1\"></iframe>",
            "<iframe src=\"https://t.example/embed/px\" hidden></iframe>",
            "<iframe src=\"https://t.example/embed/px\" style=\"display: none\"></iframe>",
            "<iframe src=\"ftp://files.example/embed/x\"></iframe>",
            "<iframe src=\"javascript:alert(1)//embed\"></iframe>",
            "<iframe></iframe>",
        ] {
            let out = sanitize(&format!("{block}<p>ok</p>"), None);
            assert_eq!(out, "<p>ok</p>", "{block}");
        }
    }
}
