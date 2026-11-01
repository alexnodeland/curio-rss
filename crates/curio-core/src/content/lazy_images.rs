//! Lazy-image recovery, applied to raw HTML *before* the sanitize gate.
//!
//! Lazy loaders ship a placeholder `src` (usually a tiny `data:` GIF) and
//! park the real URL in `data-src` / `data-lazy-src` / `srcset`, with a
//! `<noscript><img src=REAL></noscript>` fallback beside it. The gate keeps
//! only `img[src]` with an `http(s)` source and drops `<noscript>` with its
//! content, so without this pass that whole pattern vanishes.
//!
//! The pass only ever *moves a URL into `src`* (or lifts an `<img>` out of a
//! `<noscript>`), rebuilding the tag with `src`/`alt`/`title` and nothing
//! else. Ammonia still runs afterward and judges every promoted `src` with
//! the same `http`/`https`-only scheme filter as any other — a hostile
//! `data-src` cannot survive as a live source, so the pass cannot weaken the
//! gate.

use std::sync::LazyLock;

use regex::{Captures, Regex};
use scraper::{Html, Selector};
use url::Url;

use super::sanitize::escape_attr;

/// Attributes lazy loaders park the real source in, in preference order.
/// When one holds a fetchable URL it wins over `src` (which is then the
/// placeholder, even when it happens to be an `http(s)` spacer image).
const LAZY_SRC_ATTRS: &[&str] = &["data-src", "data-lazy-src", "data-original"];

/// Responsive-candidate attributes, consulted only when no fetchable
/// single source exists.
const SRCSET_ATTRS: &[&str] = &["data-srcset", "data-lazy-srcset", "srcset"];

/// Promotes lazy sources to `src` and unwraps `<noscript>` image fallbacks
/// whose visible sibling has no fetchable source. `base` resolves relative
/// candidates when judging whether they are fetchable; the promoted value
/// itself is left relative for ammonia to resolve.
pub(super) fn recover_lazy_images(html: &str, base: Option<&Url>) -> String {
    let promoted = img_re().map_or_else(
        || html.to_owned(),
        |re| {
            re.replace_all(html, |caps: &Captures<'_>| {
                promote_tag(&caps[0], base).unwrap_or_else(|| caps[0].to_owned())
            })
            .into_owned()
        },
    );
    noscript_re().map_or_else(
        || promoted.clone(),
        |re| {
            re.replace_all(&promoted, |caps: &Captures<'_>| unwrap_noscript(caps, base))
                .into_owned()
        },
    )
}

/// One `<img …>` start tag; quoted attribute values may contain `>`.
fn img_re() -> Option<&'static Regex> {
    static RE: LazyLock<Option<Regex>> =
        LazyLock::new(|| Regex::new(r#"(?is)<img\b(?:[^>"']|"[^"]*"|'[^']*')*>"#).ok());
    RE.as_ref()
}

/// A `<noscript>` block, optionally preceded (across whitespace) by the
/// visible `<img>` it is the fallback for.
fn noscript_re() -> Option<&'static Regex> {
    static RE: LazyLock<Option<Regex>> = LazyLock::new(|| {
        Regex::new(
            r#"(?is)(?P<img><img\b(?:[^>"']|"[^"]*"|'[^']*')*>)?(?P<ws>\s*)<noscript\b[^>]*>(?P<inner>.*?)</noscript\s*>"#,
        )
        .ok()
    });
    RE.as_ref()
}

/// The parsed attributes of an `<img>` tag (or of the first `<img>` in a
/// small fragment).
struct ImgAttrs {
    src: Option<String>,
    alt: Option<String>,
    title: Option<String>,
    lazy: Vec<String>,
    srcsets: Vec<String>,
}

fn parse_imgs(html: &str) -> Vec<ImgAttrs> {
    let Ok(selector) = Selector::parse("img") else {
        return Vec::new();
    };
    let doc = Html::parse_fragment(html);
    doc.select(&selector)
        .map(|img| {
            let el = img.value();
            let owned = |name: &str| el.attr(name).map(|v| v.trim().to_owned());
            ImgAttrs {
                src: owned("src"),
                alt: el.attr("alt").map(str::to_owned),
                title: el.attr("title").map(str::to_owned),
                lazy: LAZY_SRC_ATTRS.iter().filter_map(|a| owned(a)).collect(),
                srcsets: SRCSET_ATTRS.iter().filter_map(|a| owned(a)).collect(),
            }
        })
        .collect()
}

impl ImgAttrs {
    /// The source this image should carry: a fetchable lazy source, else a
    /// fetchable `src`, else the best fetchable `srcset` candidate.
    fn best_src(&self, base: Option<&Url>) -> Option<String> {
        if let Some(lazy) = self.lazy.iter().find(|v| is_fetchable(v, base)) {
            return Some(lazy.clone());
        }
        if let Some(src) = self.src.as_ref().filter(|v| is_fetchable(v, base)) {
            return Some(src.clone());
        }
        self.srcsets
            .iter()
            .find_map(|set| best_srcset_candidate(set, base))
    }

    /// The rebuilt tag carrying `src` (and `alt`/`title`, the only other
    /// attributes the gate keeps on `img`).
    fn tag_with_src(&self, src: &str) -> String {
        let optional = |name: &str, value: Option<&String>| {
            value.map_or_else(String::new, |v| format!(" {name}=\"{}\"", escape_attr(v)))
        };
        format!(
            "<img src=\"{}\"{}{}>",
            escape_attr(src),
            optional("alt", self.alt.as_ref()),
            optional("title", self.title.as_ref()),
        )
    }
}

/// The rewritten tag when a better source than `src` exists; `None` leaves
/// the original tag untouched.
fn promote_tag(tag: &str, base: Option<&Url>) -> Option<String> {
    // Fast path: a tag with no lazy or responsive attribute has nothing to
    // promote, so the common plain `<img src>` is never parsed.
    let lower = tag.to_ascii_lowercase();
    if !lower.contains("data-") && !lower.contains("srcset") {
        return None;
    }
    let img = parse_imgs(tag).into_iter().next()?;
    let best = img.best_src(base)?;
    (img.src.as_deref() != Some(best.as_str())).then(|| img.tag_with_src(&best))
}

/// A `<noscript>` (plus its optional visible sibling) → the sibling when it
/// already shows a fetchable image (the fallback is a duplicate), otherwise
/// the fallback's own fetchable `<img>`s, lifted out. Anything else inside
/// `<noscript>` stays dropped, exactly as the gate would drop it.
fn unwrap_noscript(caps: &Captures<'_>, base: Option<&Url>) -> String {
    let sibling = caps.name("img").map(|m| m.as_str());
    let ws = caps.name("ws").map_or("", |m| m.as_str());
    if let Some(tag) = sibling {
        let shows_image = parse_imgs(tag)
            .first()
            .and_then(|img| img.src.as_ref())
            .is_some_and(|src| is_fetchable(src, base));
        if shows_image {
            return format!("{tag}{ws}");
        }
    }
    let fallbacks: Vec<String> = parse_imgs(caps.name("inner").map_or("", |m| m.as_str()))
        .iter()
        .filter_map(|img| img.best_src(base).map(|src| img.tag_with_src(&src)))
        .collect();
    if fallbacks.is_empty() {
        // Nothing to recover: keep the sibling (if any) for the gate to judge.
        return sibling.map_or_else(String::new, |tag| format!("{tag}{ws}"));
    }
    fallbacks.concat()
}

/// Whether `value` resolves (against `base` when relative) to an `http(s)`
/// URL — i.e. the image cache could fetch it.
pub(super) fn is_fetchable(value: &str, base: Option<&Url>) -> bool {
    resolve_http(value, base).is_some()
}

/// `value` as an absolute `http(s)` URL, resolving a relative reference
/// against `base`. `None` for any other scheme or an unresolvable value.
pub(super) fn resolve_http(value: &str, base: Option<&Url>) -> Option<Url> {
    let url = match Url::parse(value.trim()) {
        Ok(url) => url,
        Err(url::ParseError::RelativeUrlWithoutBase) => base?.join(value.trim()).ok()?,
        Err(_) => return None,
    };
    matches!(url.scheme(), "http" | "https").then_some(url)
}

/// The largest fetchable candidate of a `srcset`: the widest `w`
/// descriptor if any candidate has one, else the highest density (`x`,
/// default `1x`). `None` when no candidate is fetchable.
fn best_srcset_candidate(srcset: &str, base: Option<&Url>) -> Option<String> {
    parse_srcset(srcset)
        .into_iter()
        .filter(|(url, _)| is_fetchable(url, base))
        .max_by_key(|(_, descriptor)| descriptor_rank(descriptor))
        .map(|(url, _)| url.to_owned())
}

/// Ranks a candidate descriptor: any width (`640w`) outranks any density,
/// then larger wins. Unparseable descriptors rank as `1x`.
fn descriptor_rank(descriptor: &str) -> (u8, u64) {
    let descriptor = descriptor.trim().to_ascii_lowercase();
    if let Some(width) = descriptor
        .strip_suffix('w')
        .and_then(|w| w.parse::<u64>().ok())
    {
        return (1, width);
    }
    let density = descriptor
        .strip_suffix('x')
        .and_then(|x| x.parse::<f64>().ok())
        .filter(|x| x.is_finite() && *x > 0.0)
        .unwrap_or(1.0);
    // Hundredths of a density unit; the cast saturates, and densities are
    // small positive numbers, so the ordering is preserved.
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "density is finite and positive (filtered above); `as` saturates"
    )]
    let rank = (density * 100.0).round() as u64;
    (0, rank)
}

/// Splits a `srcset` into `(url, descriptor)` candidates, following the
/// HTML parsing rule that a URL runs to the next whitespace (so commas
/// *inside* a URL, as in `w_300,h_200` CDN paths, survive) and a URL ending
/// in `,` has no descriptor.
fn parse_srcset(srcset: &str) -> Vec<(&str, &str)> {
    let mut out = Vec::new();
    let mut rest = srcset;
    loop {
        rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == ',');
        if rest.is_empty() {
            break;
        }
        let url_end = rest
            .find(|c: char| c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let (raw_url, after) = rest.split_at(url_end);
        let (url, descriptor, remainder) = if raw_url.ends_with(',') {
            (raw_url.trim_end_matches(','), "", after)
        } else {
            let end = after.find(',').unwrap_or(after.len());
            let (descriptor, remainder) = after.split_at(end);
            (raw_url, descriptor.trim(), remainder)
        };
        if !url.is_empty() {
            out.push((url, descriptor));
        }
        rest = remainder;
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn base() -> Url {
        Url::parse("https://example.com/posts/one").unwrap()
    }

    #[test]
    fn data_src_replaces_a_data_uri_placeholder() {
        let out = recover_lazy_images(
            "<img src=\"data:image/gif;base64,R0lGOD\" data-src=\"https://cdn.example/real.jpg\" alt=\"a\">",
            None,
        );
        assert_eq!(out, "<img src=\"https://cdn.example/real.jpg\" alt=\"a\">");
    }

    #[test]
    fn data_lazy_src_wins_over_an_http_spacer() {
        let out = recover_lazy_images(
            "<img src=\"https://example.com/spacer.gif\" data-lazy-src=\"/real.png\">",
            Some(&base()),
        );
        assert_eq!(out, "<img src=\"/real.png\">");
    }

    #[test]
    fn srcset_best_candidate_fills_a_missing_src() {
        let out = recover_lazy_images(
            "<img srcset=\"https://c.example/w_300,h_200/a.jpg 300w, https://c.example/w_900,h_600/a.jpg 900w\" alt=\"x\">",
            None,
        );
        assert!(
            out.contains("src=\"https://c.example/w_900,h_600/a.jpg\""),
            "{out}"
        );
    }

    #[test]
    fn a_fetchable_src_is_left_alone_even_with_a_srcset() {
        let tag = "<img src=\"https://e.example/a.jpg\" srcset=\"https://e.example/b.jpg 2x\">";
        assert_eq!(recover_lazy_images(tag, None), tag);
    }

    #[test]
    fn unfetchable_lazy_sources_are_not_promoted() {
        let tag = "<img src=\"data:image/gif;base64,R0\" data-src=\"javascript:alert(1)\" srcset=\"javascript:alert(2) 2x\">";
        assert_eq!(recover_lazy_images(tag, None), tag);
    }

    #[test]
    fn noscript_fallback_is_lifted_when_the_sibling_is_a_placeholder() {
        let out = recover_lazy_images(
            "<p>a</p><img src=\"data:image/gif;base64,R0\" alt=\"ph\">\n\
             <noscript><img src=\"https://cdn.example/real.jpg\" alt=\"real\"></noscript><p>b</p>",
            None,
        );
        assert_eq!(
            out,
            "<p>a</p><img src=\"https://cdn.example/real.jpg\" alt=\"real\"><p>b</p>"
        );
    }

    #[test]
    fn noscript_duplicate_is_dropped_once_the_sibling_was_promoted() {
        let out = recover_lazy_images(
            "<img src=\"data:image/gif;base64,R0\" data-src=\"https://cdn.example/real.jpg\">\
             <noscript><img src=\"https://cdn.example/real.jpg\"></noscript>",
            None,
        );
        assert_eq!(out, "<img src=\"https://cdn.example/real.jpg\">");
    }

    #[test]
    fn a_lone_noscript_keeps_only_its_images() {
        let out = recover_lazy_images(
            "<noscript><p>Enable JS</p><img src=\"https://cdn.example/x.png\" onerror=\"alert(1)\"><script>alert(1)</script></noscript>",
            None,
        );
        assert_eq!(out, "<img src=\"https://cdn.example/x.png\">");
    }

    #[test]
    fn a_noscript_without_images_keeps_its_sibling() {
        let out = recover_lazy_images(
            "<img src=\"data:image/gif;base64,R0\"> <noscript>nothing</noscript>",
            None,
        );
        assert_eq!(out, "<img src=\"data:image/gif;base64,R0\"> ");
    }

    #[test]
    fn attribute_values_are_re_escaped() {
        let out = recover_lazy_images(
            "<img data-src=\"https://e.example/a.jpg?x=1&amp;y=2\" alt=\"say &quot;hi&quot; &lt;3\">",
            None,
        );
        assert_eq!(
            out,
            "<img src=\"https://e.example/a.jpg?x=1&amp;y=2\" alt=\"say &quot;hi&quot; &lt;3\">"
        );
    }

    #[test]
    fn srcset_parsing_follows_the_html_rules() {
        assert_eq!(
            parse_srcset(" a.jpg 1x , b.jpg, c,d.jpg 3x,,e.jpg"),
            vec![
                ("a.jpg", "1x"),
                ("b.jpg", ""),
                ("c,d.jpg", "3x"),
                ("e.jpg", "")
            ]
        );
    }

    #[test]
    fn descriptors_rank_width_over_density() {
        assert!(descriptor_rank("100w") > descriptor_rank("3x"));
        assert!(descriptor_rank("2x") > descriptor_rank(""));
        assert!(descriptor_rank("1.5x") > descriptor_rank("1x"));
        assert_eq!(descriptor_rank("garbage"), descriptor_rank("1x"));
    }

    #[test]
    fn resolve_http_rejects_other_schemes() {
        assert!(resolve_http("ftp://e.example/x", None).is_none());
        assert!(resolve_http("/x.png", None).is_none());
        assert!(resolve_http("http://[::1", None).is_none());
        assert_eq!(
            resolve_http("/x.png", Some(&base())).map(String::from),
            Some("https://example.com/x.png".to_owned())
        );
    }
}
