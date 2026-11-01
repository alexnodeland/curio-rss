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
/// event-handler or style attributes, http(s)-only URL attributes.
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
                let in_page_fragment = attr == "href"
                    && value
                        .strip_prefix("#user-content-")
                        .is_some_and(is_plain_token);
                if !(value.starts_with("http://") || value.starts_with("https://"))
                    && !in_page_fragment
                {
                    return Err(format!(
                        "non-http(s) {attr} survived on <{name}>: {value:?}\n{html}"
                    ));
                }
            }
            if !value_is_filtered(name, attr, value) {
                return Err(format!(
                    "unfiltered {attr} on <{name}>: {value:?}\n{html}"
                ));
            }
        }
    }
    Ok(())
}

/// Every attribute the sanitize gate may leave on stored HTML.
const ALLOWED_ATTRIBUTES: &[&str] = &[
    "href", "rel", "title", "src", "alt", "datetime", "colspan", "rowspan", "id", "dir", "lang",
    "start", "class",
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
        let destination = prose[at + 2..].trim_start_matches([' ', '<']);
        if let Some(scheme) = SCRIPTABLE_SCHEMES
            .iter()
            .find(|scheme| destination.starts_with(**scheme))
        {
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

/// `markdown` with fenced code blocks and inline code spans blanked out.
fn strip_code(markdown: &str) -> String {
    let mut out = String::with_capacity(markdown.len());
    let mut fence: Option<&str> = None;
    for line in markdown.lines() {
        let trimmed = line.trim_start();
        let marker = ["```", "~~~"]
            .into_iter()
            .find(|marker| trimmed.starts_with(marker));
        match (fence, marker) {
            (None, Some(open)) => fence = Some(open),
            (Some(open), Some(close)) if open == close => fence = None,
            (None, None) => {
                // Inline spans: drop everything between paired backticks.
                // An unpaired backtick opens no span, so that line is
                // audited whole (never a hiding place).
                let mut in_span = false;
                let mut kept = String::with_capacity(line.len());
                for ch in line.chars() {
                    if ch == '`' {
                        in_span = !in_span;
                    } else if !in_span {
                        kept.push(ch);
                    }
                }
                out.push_str(if in_span { line } else { &kept });
            }
            _ => {}
        }
        out.push('\n');
    }
    out
}
