//! Turning fetched bytes into text: character-encoding detection for HTML
//! pages and feeds, plus the media-type checks that decide whether a body
//! is a page worth extracting at all.
//!
//! The precedence chain is the same for both kinds of document:
//!
//! 1. a byte-order mark (UTF-8, UTF-16LE/BE) — always authoritative;
//! 2. the HTTP `Content-Type` `charset` parameter;
//! 3. the document's own declaration — `<meta charset>` /
//!    `<meta http-equiv="Content-Type">` for HTML (the WHATWG prescan, over
//!    the first [`PRESCAN_WINDOW`] bytes), the `<?xml … encoding="…"?>`
//!    declaration for feeds;
//! 4. UTF-8, lossily (invalid sequences become U+FFFD).
//!
//! One pragmatic exception to step 2: a header claiming UTF-8 for bytes
//! that are *not* valid UTF-8 yields to the document's own declaration.
//! That is the classic server-wide `AddDefaultCharset UTF-8` misconfiguration
//! in front of a legacy-encoded page, where the document knows better.
//!
//! Labels resolve through the WHATWG Encoding Standard ([`encoding_rs`]), so
//! `latin1`, `iso-8859-1` and `us-ascii` all decode as windows-1252, exactly
//! as a browser does.

use std::borrow::Cow;

use encoding_rs::{Encoding, UTF_8, WINDOWS_1252};

/// How far into an HTML body the `<meta charset>` prescan looks. The HTML
/// standard prescans 1024 bytes; real pages sometimes push the declaration
/// a little later (a long comment banner, inline critical CSS), so the window
/// is more generous. Past it, the declaration is ignored as browsers do.
pub const PRESCAN_WINDOW: usize = 4096;

/// The lowercase `type/subtype` of a `Content-Type` header value, without
/// parameters — `None` for an empty or malformed value.
#[must_use]
pub fn mime_essence(content_type: &str) -> Option<String> {
    let essence = content_type.split(';').next()?.trim();
    (essence.contains('/') && !essence.starts_with('/') && !essence.ends_with('/'))
        .then(|| essence.to_ascii_lowercase())
}

/// The encoding a `Content-Type` value's `charset` parameter names, if it
/// names one the Encoding Standard knows.
#[must_use]
pub fn charset_param(content_type: &str) -> Option<&'static Encoding> {
    content_type.split(';').skip(1).find_map(|param| {
        let (name, value) = param.split_once('=')?;
        if !name.trim().eq_ignore_ascii_case("charset") {
            return None;
        }
        let label = value.trim().trim_matches(|c| c == '"' || c == '\'');
        Encoding::for_label(label.as_bytes())
    })
}

/// Decodes an HTML page to text: BOM → HTTP charset → `<meta charset>` →
/// UTF-8 (lossy). Borrows when the body is already valid UTF-8 and nothing
/// says otherwise.
#[must_use]
pub fn decode_html<'a>(body: &'a [u8], content_type: Option<&str>) -> Cow<'a, str> {
    if let Some((encoding, bom_len)) = Encoding::for_bom(body) {
        return encoding.decode_without_bom_handling(&body[bom_len..]).0;
    }
    let declared = || meta_charset(body);
    let encoding = content_type
        .and_then(charset_param)
        .and_then(|header| trust_header(header, body, declared))
        .or_else(declared)
        .unwrap_or(UTF_8);
    encoding.decode_without_bom_handling(body).0
}

/// Re-encodes feed bytes as UTF-8 for the XML/JSON parser: BOM → HTTP
/// charset → XML declaration → UTF-8. Transcoded output loses anything
/// ahead of the document (the BOM, stray whitespace before `<?xml`, which
/// XML forbids and real servers emit) and has its declaration rewritten to
/// name UTF-8, so the parser never re-decodes it with the stale label.
///
/// Borrows the input untouched when it is UTF-8 already and its
/// declaration (if any) agrees — the common case costs no copy. The parser
/// skips a UTF-8 BOM and leading whitespace on its own.
#[must_use]
pub fn feed_to_utf8<'a>(body: &'a [u8], content_type: Option<&str>) -> Cow<'a, [u8]> {
    let (bom, bom_len) = Encoding::for_bom(body).map_or((None, 0), |(enc, len)| (Some(enc), len));
    let rest = &body[bom_len..];
    // The declaration is only readable in place for ASCII-compatible bytes;
    // a UTF-16 document is decoded by its BOM, which outranks it anyway.
    let ascii_compatible = bom.is_none_or(Encoding::is_ascii_compatible);
    let declared = || {
        ascii_compatible
            .then(|| xml_decl_encoding(trim_xml_space(rest)))
            .flatten()
    };
    let encoding = bom
        .or_else(|| {
            content_type
                .and_then(charset_param)
                .and_then(|header| trust_header(header, rest, declared))
        })
        .or_else(declared)
        .unwrap_or(UTF_8);

    if encoding == UTF_8 && declared().is_none_or(|label| label == UTF_8) {
        return Cow::Borrowed(body);
    }
    let text = encoding.decode_without_bom_handling(rest).0;
    let text = text.trim_start_matches(is_xml_space);
    Cow::Owned(relabel_xml_decl(text).into_bytes())
}

/// Whether a body is an HTML page worth running extraction on: an HTML or
/// XHTML media type, or — when the server named no usable type (or the
/// generic `application/octet-stream`) — bytes that do not sniff as a PDF,
/// an image or other binary data. Everything else (PDFs, images, plain
/// text, JSON, feeds) is not a page.
#[must_use]
pub fn is_html_page(content_type: Option<&str>, body: &[u8]) -> bool {
    match content_type.and_then(mime_essence).as_deref() {
        Some("text/html" | "application/xhtml+xml") => true,
        None | Some("application/octet-stream") => !sniffs_binary(body),
        Some(_) => false,
    }
}

/// A header that claims UTF-8 for bytes that are not UTF-8 loses to a
/// declaration in the document; any other header is taken at its word.
fn trust_header(
    header: &'static Encoding,
    body: &[u8],
    declared: impl FnOnce() -> Option<&'static Encoding>,
) -> Option<&'static Encoding> {
    if header == UTF_8 && std::str::from_utf8(body).is_err() {
        return declared().or(Some(header));
    }
    Some(header)
}

/// Binary signatures (PDF, PNG, JPEG, GIF, WebP) or a NUL byte early on —
/// text formats never carry one.
fn sniffs_binary(body: &[u8]) -> bool {
    const SIGNATURES: [&[u8]; 6] = [
        b"%PDF-",
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff",
        b"GIF87a",
        b"GIF89a",
        b"\x00\x00\x01\x00", // ICO
    ];
    let is_webp = body.len() >= 12 && &body[..4] == b"RIFF" && &body[8..12] == b"WEBP";
    is_webp
        || SIGNATURES.iter().any(|sig| body.starts_with(sig))
        || body[..body.len().min(1024)].contains(&0)
}

/// The WHATWG prescan, simplified: the first `<meta>` within
/// [`PRESCAN_WINDOW`] that carries a `charset` — as its own attribute or
/// inside an `http-equiv` `content` value — and names a known encoding.
/// Per the HTML standard a UTF-16 label here means UTF-8 (the bytes were
/// readable as ASCII, so they are not UTF-16), and `x-user-defined` means
/// windows-1252.
fn meta_charset(body: &[u8]) -> Option<&'static Encoding> {
    let window = &body[..body.len().min(PRESCAN_WINDOW)];
    let lower = window.to_ascii_lowercase();
    let mut from = 0;
    loop {
        let rest = &lower[from..];
        let offset = match (find(rest, b"<!--"), find(rest, b"<meta")) {
            (Some(comment), Some(meta)) => comment.min(meta),
            (Some(found), None) | (None, Some(found)) => found,
            (None, None) => return None,
        };
        let start = from + offset;
        if lower[start..].starts_with(b"<!--") {
            // Comments are skipped whole: a commented-out <meta> is inert.
            let end = find(&lower[start + 4..], b"-->")?;
            from = start + 4 + end + 3;
            continue;
        }
        let tag_end = find(&lower[start..], b">").map_or(lower.len(), |end| start + end);
        let tag = &lower[start..tag_end];
        from = tag_end;
        let Some(at) = find(tag, b"charset") else {
            continue;
        };
        let value = tag[at + b"charset".len()..]
            .trim_ascii_start()
            .strip_prefix(b"=")
            .map(<[u8]>::trim_ascii_start);
        let Some(value) = value else { continue };
        let value = value
            .strip_prefix(b"\"")
            .or_else(|| value.strip_prefix(b"'"))
            .unwrap_or(value);
        let label_end = value
            .iter()
            .position(|&b| {
                matches!(b, b'"' | b'\'' | b';' | b'/' | b'>') || b.is_ascii_whitespace()
            })
            .unwrap_or(value.len());
        let Some(encoding) = Encoding::for_label(&value[..label_end]) else {
            continue;
        };
        return Some(match encoding.name() {
            "UTF-16LE" | "UTF-16BE" => UTF_8,
            "x-user-defined" => WINDOWS_1252,
            _ => encoding,
        });
    }
}

/// The encoding an `<?xml … encoding="…"?>` declaration at the very start
/// of `xml` names, if any (and if known).
fn xml_decl_encoding(xml: &[u8]) -> Option<&'static Encoding> {
    let decl = xml.strip_prefix(b"<?xml")?;
    let decl = &decl[..find(decl, b"?>")?];
    let at = find(decl, b"encoding")?;
    let value = decl[at + b"encoding".len()..]
        .trim_ascii_start()
        .strip_prefix(b"=")?
        .trim_ascii_start();
    let quote = *value.first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value = &value[1..];
    let label = &value[..value.iter().position(|&b| b == quote)?];
    // A declaration readable as ASCII is not UTF-16, whatever it claims
    // (a real UTF-16 document starts with a BOM, handled before this).
    Encoding::for_label(label).map(|encoding| match encoding.name() {
        "UTF-16LE" | "UTF-16BE" => UTF_8,
        _ => encoding,
    })
}

/// Replaces a leading `<?xml …?>` declaration with one naming UTF-8 (the
/// bytes it now prefixes); text without a declaration is returned as is.
fn relabel_xml_decl(text: &str) -> String {
    const UTF8_DECL: &str = r#"<?xml version="1.0" encoding="UTF-8"?>"#;
    match text.strip_prefix("<?xml").and_then(|rest| {
        // `<?xml-stylesheet …?>` is a processing instruction, not the
        // declaration: the declaration's name ends at whitespace.
        rest.starts_with(|c: char| c.is_ascii_whitespace())
            .then(|| rest.find("?>"))
            .flatten()
            .map(|end| &rest[end + 2..])
    }) {
        Some(after) => format!("{UTF8_DECL}{after}"),
        None => text.to_owned(),
    }
}

/// XML's `S` production: space, tab, CR, LF.
fn is_xml_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

fn trim_xml_space(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|&b| !matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
        .unwrap_or(bytes.len());
    &bytes[start..]
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests;
