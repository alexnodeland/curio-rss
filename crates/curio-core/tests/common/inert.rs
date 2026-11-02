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
/// Code (fenced and indented blocks, inline spans) and backslash-escaped
/// `\<` are text to a renderer, so `\<script>` — how htmd writes literal
/// angle brackets that were plain text in the source — is inert and
/// passes. What counts as code, and where a link's destination sits, is
/// decided by a real `CommonMark` parser (see [`parse`]); everything else
/// is scanned as prose, conservatively.
///
/// # Errors
///
/// A description of the first violation found.
pub fn audit_markdown(markdown: &str) -> Result<(), String> {
    // Plain `CommonMark`, and the GFM-style extensions note apps and
    // static-site generators turn on (a table splits its row into cells
    // first, so a span the plain parse pairs may not pair there).
    let gfm = pulldown_cmark::Options::ENABLE_TABLES
        | pulldown_cmark::Options::ENABLE_FOOTNOTES
        | pulldown_cmark::Options::ENABLE_STRIKETHROUGH
        | pulldown_cmark::Options::ENABLE_TASKLISTS;
    for options in [pulldown_cmark::Options::empty(), gfm] {
        let parsed = parse(markdown, options);
        if let Some(destination) = parsed.scriptable_destination {
            return Err(format!(
                "markdown links to a scriptable destination {destination:?}:\n{markdown}"
            ));
        }
        audit_link_prose(&parsed.prose.to_lowercase(), markdown)?;
        audit_tag_prose(&parsed.tag_prose.to_lowercase(), markdown)?;
    }
    Ok(())
}

/// Every `](` destination in `prose`, read like the scheme of an `href`
/// (see `browser_scheme`), so `]( java\tscript:…)` cannot slip past —
/// whether or not the parser saw a link there.
fn audit_link_prose(prose: &str, markdown: &str) -> Result<(), String> {
    for (at, _) in prose.match_indices("](") {
        let rest = prose[at + 2..].trim_start_matches(|c: char| c <= ' ' || c == '<');
        let end = rest.find([')', '>', '\n']).unwrap_or(rest.len());
        if let Some(scheme) = browser_scheme(&rest[..end]).and_then(|scheme| scriptable(&scheme)) {
            return Err(format!(
                "markdown links to a {scheme} destination:\n{markdown}"
            ));
        }
    }
    Ok(())
}

/// Every `<` in `prose` that is not escaped and opens something tag-shaped:
/// a forbidden element, or a tag carrying a handler or a scriptable URL.
fn audit_tag_prose(prose: &str, markdown: &str) -> Result<(), String> {
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

/// The listed scriptable scheme `scheme` names, if any.
fn scriptable(scheme: &str) -> Option<&'static str> {
    SCRIPTABLE_SCHEMES
        .iter()
        .find(|listed| listed.strip_suffix(':') == Some(scheme))
        .copied()
}

/// What one `CommonMark` parse of exported markdown says.
struct Parsed {
    /// The markdown with every byte the parser reads as code — fenced and
    /// indented code blocks and inline code spans, delimiters included —
    /// blanked to a space (line breaks kept, so the line-based scans see
    /// the same lines).
    prose: String,
    /// [`Self::prose`] with the destinations of inline links and images
    /// blanked too: `[x](<https://a.example/ b>)` is a link, not a tag.
    tag_prose: String,
    /// The first link or image destination (any kind: inline, reference,
    /// autolink) whose scheme is scriptable.
    scriptable_destination: Option<String>,
}

/// Parses `markdown` once with `options`.
///
/// A real parser, because what is code depends on the containers around
/// it: a fence inside a block quote opens after its `> ` marker, one inside
/// a list item after the item's indentation, and a span pairs only within
/// one paragraph — and on a run of the *same length*, never after an
/// escaped backtick (#245). A line scanner either reads container code as
/// prose, or guesses at containers and hides prose when it guesses wrong.
/// Anything the parser does not read as code stays in the prose, so live
/// HTML can never hide behind something that only *looks* like code.
fn parse(markdown: &str, options: pulldown_cmark::Options) -> Parsed {
    use pulldown_cmark::{Event, LinkType, Parser, Tag};

    let mut prose = markdown.as_bytes().to_vec();
    let mut destinations = Vec::new();
    let mut scriptable_destination = None;
    for (event, range) in Parser::new_ext(markdown, options).into_offset_iter() {
        match event {
            Event::Code(_) | Event::Start(Tag::CodeBlock(_)) => blank(&mut prose, range),
            Event::Start(
                Tag::Link {
                    link_type,
                    dest_url,
                    ..
                }
                | Tag::Image {
                    link_type,
                    dest_url,
                    ..
                },
            ) => {
                if scriptable_destination.is_none()
                    && browser_scheme(&dest_url).is_some_and(|s| scriptable(&s).is_some())
                {
                    scriptable_destination = Some(dest_url.to_string());
                }
                // An inline link's destination (and title) follows its
                // last `](`: everything before it is the link text, which
                // stays in the prose.
                if link_type == LinkType::Inline
                    && let Some(at) = markdown[range.clone()].rfind("](")
                {
                    destinations.push(range.start + at + 2..range.end);
                }
            }
            _ => {}
        }
    }
    let mut tag_prose = prose.clone();
    for range in destinations {
        blank(&mut tag_prose, range);
    }
    // Ranges fall on character boundaries and only ASCII spaces are
    // written, so these are still UTF-8 (lossy only to keep a panic path
    // out).
    Parsed {
        prose: String::from_utf8_lossy(&prose).into_owned(),
        tag_prose: String::from_utf8_lossy(&tag_prose).into_owned(),
        scriptable_destination,
    }
}

/// Blanks `range` of `bytes` to spaces, keeping line breaks.
fn blank(bytes: &mut [u8], range: std::ops::Range<usize>) {
    for byte in &mut bytes[range] {
        if *byte != b'\n' {
            *byte = b' ';
        }
    }
}
