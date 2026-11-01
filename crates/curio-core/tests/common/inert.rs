//! The inertness audit (D6): the one definition of "stored content is
//! clean", shared by the XSS golden, the nasty corpus, the proptests and
//! the `fuzz/` targets.
//!
//! Included per binary with `#[path = "common/inert.rs"] mod inert;` rather
//! than through `common/mod.rs`, so suites that never audit content do not
//! carry it as dead code. Depends on nothing but `scraper` and std, so the
//! out-of-workspace fuzz crate can include it the same way.

/// Elements that must never survive sanitization: script hosts, embedded
/// documents, forms, styling and document-level metadata.
const FORBIDDEN_ELEMENTS: &[&str] = &[
    "script", "svg", "iframe", "frame", "object", "embed", "form", "input", "button", "style",
    "base", "meta", "link", "audio", "video", "math",
];

/// Link/image destination schemes that execute (or smuggle a document)
/// when a markdown renderer follows them.
const SCRIPTABLE_SCHEMES: &[&str] = &["javascript:", "vbscript:", "data:"];

/// Structural audit of stored (sanitized) HTML: no forbidden elements, no
/// event-handler or style attributes, every other attribute allowlisted and
/// value-filtered, and URL attributes that a browser resolves to http(s)
/// (or a prefixed in-page fragment).
///
/// "Resolves to" is judged with WHATWG URL parsing, which is what the
/// desktop webview applies to stored HTML — and what ammonia's own scheme
/// filter applies, through the `url` crate: leading and trailing C0
/// controls and spaces are stripped, tabs and newlines removed, and the
/// scheme is case-insensitive. So `"\u{2}https://x/"` is an https URL to
/// both and passes, while `"\u{1}javascript:…"` is `javascript:` to both
/// and fails. A literal-prefix check would flag the first (a false
/// positive the `content_pipeline` fuzz target found) and say nothing
/// sound about the second.
///
/// Structural on purpose — a substring scan would false-positive on inert
/// anchor *text* like "javascript: link", which sanitization correctly
/// leaves behind as text.
///
/// # Errors
///
/// A description of the first violation found.
pub fn audit_html(html: &str) -> Result<(), String> {
    let stored = scraper::Html::parse_fragment(html);
    let Ok(all) = scraper::Selector::parse("*") else {
        return Err("the universal selector failed to parse".to_owned());
    };
    for element in stored.select(&all) {
        let name = element.value().name();
        if FORBIDDEN_ELEMENTS.contains(&name) {
            return Err(format!("forbidden element <{name}> survived:\n{html}"));
        }
        for (attr, value) in element.value().attrs() {
            if attr.starts_with("on") || attr == "style" || attr == "http-equiv" {
                return Err(format!(
                    "forbidden attribute {attr:?} survived on <{name}>:\n{html}"
                ));
            }
            if !ALLOWED_ATTRIBUTES.contains(&attr) {
                return Err(format!(
                    "unexpected attribute {attr:?} survived on <{name}>:\n{html}"
                ));
            }
            if matches!(attr, "href" | "src") {
                // A prefixed in-page fragment names the reader's own
                // document: inert. (The sanitizer keeps one on `img[src]`
                // too, which loads nothing useful but nothing foreign.)
                let in_page_fragment = value
                    .strip_prefix("#user-content-")
                    .is_some_and(is_plain_token);
                if !matches!(browser_scheme(value).as_deref(), Some("http" | "https"))
                    && !in_page_fragment
                {
                    return Err(format!(
                        "non-http(s) {attr} survived on <{name}>: {value:?}\n{html}"
                    ));
                }
            }
            if !value_is_filtered(name, attr, value) {
                return Err(format!("unfiltered {attr} on <{name}>: {value:?}\n{html}"));
            }
        }
    }
    Ok(())
}

/// The scheme a WHATWG URL parser reads from `value`, lowercased; `None`
/// for a relative reference (no valid scheme before the first `:`).
fn browser_scheme(value: &str) -> Option<String> {
    let cleaned: String = value
        .trim_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let (scheme, _) = cleaned.split_once(':')?;
    let mut chars = scheme.chars();
    let valid = chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

/// Every attribute the sanitize gate may leave on stored HTML.
const ALLOWED_ATTRIBUTES: &[&str] = &[
    "href", "rel", "title", "src", "alt", "datetime", "colspan", "rowspan", "id", "dir", "lang",
    "start", "class", "width", "height",
];

/// The id/fragment token shape: ASCII alphanumerics and `-_.:`.
fn is_plain_token(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}

/// Whether a value-filtered attribute carries only what the sanitizer's
/// filter lets through (attributes without a filter always pass here).
fn value_is_filtered(element: &str, attr: &str, value: &str) -> bool {
    match attr {
        "id" => value
            .strip_prefix("user-content-")
            .is_some_and(is_plain_token),
        "dir" => matches!(value, "ltr" | "rtl" | "auto"),
        "lang" => {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        }
        "start" => element == "ol" && value.parse::<i32>().is_ok(),
        // Plain, canonical pixel integers on images only (`1..=20000`).
        "width" | "height" => {
            element == "img"
                && value.parse::<u32>().is_ok_and(|pixels| {
                    (1..=20_000).contains(&pixels) && pixels.to_string() == value
                })
        }
        "class" => {
            matches!(element, "pre" | "code")
                && value.strip_prefix("language-").is_some_and(|lang| {
                    !lang.is_empty()
                        && lang
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"-_+#.".contains(&b))
                })
        }
        _ => true,
    }
}

/// Audit of exported markdown, judged the way a `CommonMark` renderer
/// reads it: no scriptable link/image destination, and no *live* raw HTML
/// tag that is forbidden or carries a handler / scriptable URL.
///
/// Code (fenced blocks, inline spans) and backslash-escaped `\<` are text
/// to a renderer, so `\<script>` — how htmd writes literal angle brackets
/// that were plain text in the source — is inert and passes.
///
/// # Errors
///
/// A description of the first violation found.
pub fn audit_markdown(markdown: &str) -> Result<(), String> {
    let prose = strip_code(markdown).to_lowercase();
    for (at, _) in prose.match_indices("](") {
        // The destination, read like the scheme of an `href` (see
        // `browser_scheme`), so `]( java\tscript:…)` cannot slip past.
        let rest = prose[at + 2..].trim_start_matches(|c: char| c <= ' ' || c == '<');
        let end = rest.find([')', '>', '\n']).unwrap_or(rest.len());
        if let Some(scheme) = browser_scheme(&rest[..end]).and_then(|scheme| {
            SCRIPTABLE_SCHEMES
                .iter()
                .find(|listed| listed.strip_suffix(':') == Some(scheme.as_str()))
        }) {
            return Err(format!(
                "markdown links to a {scheme} destination:\n{markdown}"
            ));
        }
    }
    let bytes = prose.as_bytes();
    for (at, _) in prose.match_indices('<') {
        let backslashes = bytes[..at]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count();
        if backslashes % 2 == 1 {
            continue; // `\<` is a literal bracket, not a tag
        }
        let rest = &prose[at + 1..];
        let tag_end = rest.find(['>', '\n']).unwrap_or(rest.len());
        let tag = &rest[..tag_end];
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(char::is_ascii_alphanumeric)
            .collect();
        if name.is_empty() {
            continue; // `a < b`, `<3`: not a tag
        }
        if FORBIDDEN_ELEMENTS.contains(&name.as_str()) {
            return Err(format!("markdown carries a live <{name}> tag:\n{markdown}"));
        }
        let handler = tag.split_whitespace().skip(1).any(|attr| {
            attr.strip_prefix("on")
                .is_some_and(|rest| rest.contains('=') && !rest.starts_with('='))
        });
        if handler || SCRIPTABLE_SCHEMES.iter().any(|scheme| tag.contains(scheme)) {
            return Err(format!(
                "markdown carries a live tag with a handler or scriptable URL <{tag}>:\n{markdown}"
            ));
        }
    }
    Ok(())
}

/// `markdown` with fenced code blocks and inline code spans blanked out —
/// exactly what a `CommonMark` renderer reads as code, and nothing more, so
/// live HTML can never hide behind something that only *looks* like code:
///
/// - a fence opens only at 0–3 spaces of indentation (four or more is an
///   indented code *line*, audited here, never a fence that would hide
///   every line after it), with three or more of one marker, and closes on
///   the same marker at least as long, followed by nothing but spaces;
/// - an inline span opens on a backtick run and closes on the next run of
///   the *same length*; an unmatched run is literal text, and a
///   backslash-escaped backtick opens nothing.
///
/// Spans are matched within a line, never across lines: a span a renderer
/// would continue onto the next line is audited as prose instead — a false
/// positive at worst, never a hiding place.
fn strip_code(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fence: Option<(char, usize)> = None;
    for line in markdown.lines() {
        match fence {
            Some((marker, len)) => {
                if fence_run(line).is_some_and(|(close, run, rest)| {
                    close == marker && run >= len && rest.trim_matches(' ').is_empty()
                }) {
                    fence = None;
                }
            }
            None => match fence_run(line) {
                // A backtick fence's info string may not hold a backtick
                // (such a line is inline spans instead).
                Some((marker, run, info)) if marker == '~' || !info.contains('`') => {
                    fence = Some((marker, run));
                }
                _ => out.push_str(&strip_spans(line)),
            },
        }
        out.push('\n');
    }
    out
}

/// A fence run opening `line`: its marker, its length (three or more) and
/// the rest of the line. `None` past three spaces of indentation.
fn fence_run(line: &str) -> Option<(char, usize, &str)> {
    let body = line.trim_start_matches(' ');
    if line.len() - body.len() > 3 {
        return None;
    }
    let marker = body.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let run = body.len() - body.trim_start_matches(marker).len();
    (run >= 3).then(|| (marker, run, &body[run..]))
}

/// One line with its inline code spans removed (see [`strip_code`]).
fn strip_spans(line: &str) -> String {
    let bytes = line.as_bytes();
    let run_at = |from: usize| bytes[from..].iter().take_while(|&&b| b == b'`').count();
    let mut kept = String::with_capacity(line.len());
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'\\' && bytes.get(at + 1).is_some_and(u8::is_ascii_punctuation) {
            // An escaped punctuation character is literal — an escaped
            // backtick opens no span. The escape is kept so the tag check
            // still reads `\<` as a literal bracket.
            kept.push_str(&line[at..at + 2]);
            at += 2;
        } else if byte == b'`' {
            let run = run_at(at);
            let mut from = at + run;
            let mut close = None;
            while let Some(offset) = bytes[from..].iter().position(|&b| b == b'`') {
                let start = from + offset;
                let len = run_at(start);
                if len == run {
                    close = Some(start);
                    break;
                }
                from = start + len;
            }
            if let Some(start) = close {
                at = start + run; // the span, delimiters included, is code
            } else {
                kept.push_str(&line[at..at + run]); // unmatched: literal
                at += run;
            }
        } else {
            let Some(ch) = line[at..].chars().next() else {
                break;
            };
            kept.push(ch);
            at += ch.len_utf8();
        }
    }
    kept
}
