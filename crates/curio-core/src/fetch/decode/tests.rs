#![allow(clippy::unwrap_used)]

use std::borrow::Cow;

use encoding_rs::{SHIFT_JIS, WINDOWS_1251};

use super::*;

const RUSSIAN: &str = "Съешь же ещё этих мягких французских булок";
const JAPANESE: &str = "こんにちは世界";

fn encode(encoding: &'static Encoding, text: &str) -> Vec<u8> {
    encoding.encode(text).0.into_owned()
}

fn utf16(text: &str, big_endian: bool) -> Vec<u8> {
    let mut out = if big_endian {
        vec![0xFE, 0xFF]
    } else {
        vec![0xFF, 0xFE]
    };
    for unit in text.encode_utf16() {
        let bytes = if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        };
        out.extend_from_slice(&bytes);
    }
    out
}

#[test]
fn mime_essence_strips_parameters_and_lowercases() {
    assert_eq!(
        mime_essence("Text/HTML; charset=UTF-8").as_deref(),
        Some("text/html")
    );
    assert_eq!(
        mime_essence("application/pdf").as_deref(),
        Some("application/pdf")
    );
    assert_eq!(mime_essence(""), None);
    assert_eq!(mime_essence("garbage"), None);
    assert_eq!(mime_essence("text/"), None);
    assert_eq!(mime_essence("/html"), None);
}

#[test]
fn charset_param_reads_quoted_unquoted_and_case_insensitive_names() {
    assert_eq!(
        charset_param("text/html; charset=windows-1251"),
        Some(WINDOWS_1251)
    );
    assert_eq!(
        charset_param("text/xml; Charset=\"Shift_JIS\""),
        Some(SHIFT_JIS)
    );
    assert_eq!(charset_param("text/html;charset='utf-8'"), Some(UTF_8));
    assert_eq!(
        charset_param("text/html; format=flowed; charset=latin1"),
        Some(WINDOWS_1252),
        "latin1 is windows-1252 per the Encoding Standard"
    );
    assert_eq!(charset_param("text/html"), None);
    assert_eq!(charset_param("text/html; charset=klingon"), None);
    assert_eq!(charset_param("text/html; charset"), None);
}

#[test]
fn html_without_any_declaration_is_utf8_and_borrowed() {
    let body = "<p>naïve — ok</p>".as_bytes();
    let text = decode_html(body, Some("text/html"));
    assert!(matches!(text, Cow::Borrowed(_)), "no copy for clean UTF-8");
    assert_eq!(text, "<p>naïve — ok</p>");
}

#[test]
fn invalid_utf8_without_a_declaration_decodes_lossily() {
    let text = decode_html(b"<p>a\xffb</p>", None);
    assert_eq!(text, "<p>a\u{fffd}b</p>");
}

#[test]
fn the_http_charset_decodes_a_page_with_no_meta() {
    let mut body = b"<html><body><p>".to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    body.extend_from_slice(b"</p></body></html>");
    let text = decode_html(&body, Some("text/html; charset=windows-1251"));
    assert!(text.contains(RUSSIAN), "{text}");
}

#[test]
fn meta_charset_decodes_a_page_the_header_is_silent_about() {
    for meta in [
        r#"<meta charset="windows-1251">"#,
        "<meta charset=windows-1251>",
        "<META CHARSET='Windows-1251'/>",
        r#"<meta http-equiv="Content-Type" content="text/html; charset=windows-1251">"#,
    ] {
        let mut body = format!("<html><head>{meta}<title>").into_bytes();
        body.extend(encode(WINDOWS_1251, RUSSIAN));
        body.extend_from_slice(b"</title></head></html>");
        let text = decode_html(&body, Some("text/html"));
        assert!(text.contains(RUSSIAN), "{meta}: {text}");
    }
}

#[test]
fn the_http_charset_beats_a_conflicting_meta() {
    let mut body = br#"<meta charset="iso-8859-5"><p>"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    let text = decode_html(&body, Some("text/html; charset=windows-1251"));
    assert!(text.contains(RUSSIAN), "{text}");
}

#[test]
fn a_utf8_header_over_legacy_bytes_yields_to_the_meta() {
    // The `AddDefaultCharset UTF-8` misconfiguration: the header lies.
    let mut body = br#"<meta charset="windows-1251"><p>"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    let text = decode_html(&body, Some("text/html; charset=utf-8"));
    assert!(text.contains(RUSSIAN), "{text}");
    // …but without a declaration to fall back on, the header stands.
    let legacy = encode(WINDOWS_1251, RUSSIAN);
    let text = decode_html(&legacy, Some("text/html; charset=utf-8"));
    assert!(text.contains('\u{fffd}'));
}

#[test]
fn a_legacy_header_over_utf8_declared_utf8_yields_to_utf8() {
    // The reverse misconfiguration: an old `charset=ISO-8859-1` default in
    // front of a page that is UTF-8 and says so.
    let body = r#"<meta charset="utf-8"><p>Café</p>"#.as_bytes();
    let text = decode_html(body, Some("text/html; charset=iso-8859-1"));
    assert_eq!(text, r#"<meta charset="utf-8"><p>Café</p>"#);
    assert!(matches!(text, Cow::Borrowed(_)), "no transcode, no copy");
}

#[test]
fn a_legacy_header_stands_unless_the_utf8_tell_is_complete() {
    let latin1 =
        |text: &str| decode_html(text.as_bytes(), Some("text/html; charset=latin1")).into_owned();
    // Valid non-ASCII UTF-8 but no declaration: the header stands.
    assert_eq!(latin1("<p>Café</p>"), "<p>CafÃ©</p>");
    // A declaration naming another legacy encoding: the header stands.
    assert_eq!(
        latin1(r#"<meta charset="koi8-r"><p>Café</p>"#),
        r#"<meta charset="koi8-r"><p>CafÃ©</p>"#
    );
    // Bytes that are not UTF-8 under a UTF-8 declaration: the header stands.
    let body = b"<meta charset=\"utf-8\"><p>Caf\xe9</p>";
    let text = decode_html(body, Some("text/html; charset=iso-8859-1"));
    assert!(text.ends_with("<p>Café</p>"), "{text}");
    // A multi-byte legacy header is not second-guessed: valid UTF-8 can be
    // valid Shift_JIS too, so the tell is not reliable there.
    let body = "<meta charset=\"utf-8\"><p>日本</p>".as_bytes();
    let text = decode_html(body, Some("text/html; charset=shift_jis"));
    assert!(!text.contains("日本"), "{text}");
}

#[test]
fn an_xhtml_page_is_decoded_by_its_xml_declaration() {
    let mut body = br#"<?xml version="1.0" encoding="windows-1251"?><html><p>"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    for content_type in [None, Some("application/xhtml+xml"), Some("text/html")] {
        let text = decode_html(&body, content_type);
        assert!(text.contains(RUSSIAN), "{content_type:?}: {text}");
    }
    // Leading whitespace ahead of the declaration is tolerated.
    let mut padded = b"\r\n".to_vec();
    padded.extend_from_slice(&body);
    assert!(decode_html(&padded, None).contains(RUSSIAN));
}

#[test]
fn xhtml_reads_the_xml_declaration_before_the_meta_and_html_the_reverse() {
    let mut body =
        br#"<?xml version="1.0" encoding="windows-1251"?><meta charset="koi8-r"><p>"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    // Served as XML, `<meta>` is inert: the declaration decides.
    let text = decode_html(&body, Some("application/xhtml+xml"));
    assert!(text.contains(RUSSIAN), "{text}");
    // Served as HTML, the meta prescan decides.
    let text = decode_html(&body, Some("text/html"));
    assert!(!text.contains(RUSSIAN), "{text}");
}

#[test]
fn a_bom_beats_every_declaration() {
    let mut body = b"\xEF\xBB\xBF".to_vec();
    body.extend_from_slice(r#"<meta charset="windows-1251"><p>Café</p>"#.as_bytes());
    let text = decode_html(&body, Some("text/html; charset=shift_jis"));
    assert_eq!(
        text, r#"<meta charset="windows-1251"><p>Café</p>"#,
        "BOM stripped"
    );

    for big_endian in [false, true] {
        let body = utf16("<p>Привет</p>", big_endian);
        assert_eq!(decode_html(&body, None), "<p>Привет</p>");
    }
}

#[test]
fn meta_prescan_quirks_follow_the_html_standard() {
    // A UTF-16 label in an ASCII-readable meta means UTF-8.
    let text = decode_html("<meta charset=utf-16><p>é</p>".as_bytes(), None);
    assert_eq!(text, "<meta charset=utf-16><p>é</p>");
    // x-user-defined means windows-1252.
    let text = decode_html(b"<meta charset=x-user-defined><p>\xe9</p>", None);
    assert!(text.ends_with("<p>é</p>"), "{text}");
    // A commented-out meta is inert; the real one after it counts.
    let mut body = br#"<!-- <meta charset="shift_jis"> --><meta charset="windows-1251">"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(decode_html(&body, None).contains(RUSSIAN));
    // An unknown label is skipped in favour of a later, known one.
    let mut body = br#"<meta charset="klingon"><meta charset="windows-1251">"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(decode_html(&body, None).contains(RUSSIAN));
    // A meta with no charset (or no value) is passed over.
    let mut body = br#"<meta name="x"><meta charset><meta charset="windows-1251">"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(decode_html(&body, None).contains(RUSSIAN));
    // `<metadata>` is not a `<meta>`; the real meta after it counts.
    let mut body = br#"<metadata charset="shift_jis"><meta charset="windows-1251">"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(decode_html(&body, None).contains(RUSSIAN));
    // `<meta>` at the very end of the window has no name terminator.
    assert_eq!(decode_html(b"<p>x</p><meta", None), "<p>x</p><meta");
    // A self-closing `<meta/…>` spelling is still a meta.
    let mut body = br#"<meta/charset="windows-1251">"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(decode_html(&body, None).contains(RUSSIAN));
    // A `charset` substring with no `=` is passed over for the real one.
    let mut body = br#"<meta name="charset-info" charset = "windows-1251">"#.to_vec();
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(decode_html(&body, None).contains(RUSSIAN));
    // An unterminated comment ends the prescan.
    assert_eq!(
        decode_html(b"<!-- <meta charset=x", None),
        "<!-- <meta charset=x"
    );
}

#[test]
fn a_meta_past_the_prescan_window_is_ignored() {
    let mut body = vec![b' '; PRESCAN_WINDOW];
    body.extend_from_slice(br#"<meta charset="windows-1251">"#);
    body.extend(encode(WINDOWS_1251, RUSSIAN));
    assert!(!decode_html(&body, None).contains(RUSSIAN));
}

fn feed_with(decl: &str, title_bytes: &[u8]) -> Vec<u8> {
    let mut body = decl.as_bytes().to_vec();
    body.extend_from_slice(b"<rss version=\"2.0\"><channel><title>");
    body.extend_from_slice(title_bytes);
    body.extend_from_slice(b"</title></channel></rss>");
    body
}

fn as_text(bytes: &[u8]) -> &str {
    std::str::from_utf8(bytes).unwrap()
}

#[test]
fn a_clean_utf8_feed_is_borrowed_untouched() {
    let body = feed_with(
        r#"<?xml version="1.0" encoding="utf-8"?>"#,
        "日本".as_bytes(),
    );
    let out = feed_to_utf8(&body, Some("application/rss+xml; charset=utf-8"));
    assert!(matches!(out, Cow::Borrowed(_)));
    let body = feed_with("", "日本".as_bytes());
    assert!(matches!(feed_to_utf8(&body, None), Cow::Borrowed(_)));
}

#[test]
fn an_http_only_charset_transcodes_the_feed() {
    let body = feed_with(r#"<?xml version="1.0"?>"#, &encode(SHIFT_JIS, JAPANESE));
    let out = feed_to_utf8(&body, Some("text/xml; charset=Shift_JIS"));
    let text = as_text(&out);
    assert!(
        text.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?><rss"#),
        "{text}"
    );
    assert!(text.contains(JAPANESE), "{text}");
}

#[test]
fn the_http_charset_wins_over_a_disagreeing_declaration_which_is_relabelled() {
    let body = feed_with(
        r#"<?xml version="1.0" encoding="ISO-8859-1"?>"#,
        &encode(WINDOWS_1251, RUSSIAN),
    );
    let out = feed_to_utf8(&body, Some("application/rss+xml; charset=windows-1251"));
    let text = as_text(&out);
    assert!(text.contains(RUSSIAN), "{text}");
    assert!(!text.contains("ISO-8859-1"), "stale label gone: {text}");
}

#[test]
fn a_declared_encoding_is_transcoded_without_a_header() {
    let body = feed_with(
        r"<?xml version='1.0' encoding='windows-1251'?>",
        &encode(WINDOWS_1251, RUSSIAN),
    );
    let text = feed_to_utf8(&body, None).into_owned();
    assert!(as_text(&text).contains(RUSSIAN));
}

#[test]
fn a_utf8_header_over_legacy_feed_bytes_yields_to_the_declaration() {
    let body = feed_with(
        r#"<?xml version="1.0" encoding="windows-1251"?>"#,
        &encode(WINDOWS_1251, RUSSIAN),
    );
    let out = feed_to_utf8(&body, Some("text/xml; charset=utf-8"));
    assert!(as_text(&out).contains(RUSSIAN));
}

#[test]
fn a_utf8_header_over_valid_utf8_relabels_a_legacy_declaration() {
    let body = feed_with(
        r#"<?xml version="1.0" encoding="windows-1251"?>"#,
        RUSSIAN.as_bytes(),
    );
    let out = feed_to_utf8(&body, Some("text/xml; charset=utf-8"));
    let text = as_text(&out);
    assert!(text.contains(RUSSIAN));
    assert!(text.contains(r#"encoding="UTF-8""#));
}

#[test]
fn a_legacy_header_over_a_utf8_feed_declared_utf8_keeps_the_utf8() {
    // `Content-Type: application/rss+xml; charset=iso-8859-1` in front of a
    // feed that is UTF-8 and declares it: no `CafÃ©` mojibake.
    let body = feed_with(
        r#"<?xml version="1.0" encoding="UTF-8"?>"#,
        "Café".as_bytes(),
    );
    let out = feed_to_utf8(&body, Some("application/rss+xml; charset=iso-8859-1"));
    assert!(matches!(out, Cow::Borrowed(_)), "taken as is");
    assert!(as_text(&out).contains("<title>Café</title>"));
}

#[test]
fn a_legacy_feed_header_stands_without_the_full_utf8_tell() {
    // No declaration: the header is the only word, and it stands.
    let body = feed_with("", "Café".as_bytes());
    let out = feed_to_utf8(&body, Some("text/xml; charset=iso-8859-1"));
    assert!(as_text(&out).contains("CafÃ©"));
    // Pure ASCII under a UTF-8 declaration: identical either way, and the
    // stale declaration is relabelled for the transcoded copy.
    let body = feed_with(r#"<?xml version="1.0" encoding="utf-8"?>"#, b"Cafe");
    let out = feed_to_utf8(&body, Some("text/xml; charset=iso-8859-1"));
    assert!(as_text(&out).contains("<title>Cafe</title>"));
    // Genuine Latin-1 bytes under a stale UTF-8 declaration: not valid
    // UTF-8, so the header is right and wins.
    let body = feed_with(r#"<?xml version="1.0" encoding="utf-8"?>"#, b"Caf\xe9");
    let out = feed_to_utf8(&body, Some("text/xml; charset=iso-8859-1"));
    assert!(as_text(&out).contains("<title>Café</title>"));
}

#[test]
fn a_utf8_bom_or_leading_whitespace_alone_costs_no_copy() {
    // The parser skips both itself; nothing needs transcoding.
    let mut body = b"\xEF\xBB\xBF".to_vec();
    body.extend(feed_with(r#"<?xml version="1.0"?>"#, "Café".as_bytes()));
    assert!(matches!(feed_to_utf8(&body, None), Cow::Borrowed(_)));
    let body = feed_with("\r\n \t<?xml version=\"1.0\"?>", b"ok");
    assert!(matches!(feed_to_utf8(&body, None), Cow::Borrowed(_)));
}

#[test]
fn transcoded_output_drops_the_bom_and_leading_whitespace() {
    let legacy = encode(WINDOWS_1251, RUSSIAN);
    let body = feed_with("\r\n \t<?xml version=\"1.0\"?>", &legacy);
    let out = feed_to_utf8(&body, Some("text/xml; charset=windows-1251"));
    assert!(as_text(&out).starts_with("<?xml"), "whitespace gone");

    let body = feed_with("\n", &legacy);
    let out = feed_to_utf8(&body, Some("text/xml; charset=windows-1251"));
    assert!(
        as_text(&out).starts_with("<rss"),
        "no declaration to relabel"
    );

    // A UTF-8 BOM in front of a declaration that names a legacy encoding:
    // the BOM wins and the stale label is rewritten.
    let mut body = b"\xEF\xBB\xBF".to_vec();
    body.extend(feed_with(
        r#"<?xml version="1.0" encoding="windows-1251"?>"#,
        RUSSIAN.as_bytes(),
    ));
    let out = feed_to_utf8(&body, None);
    let text = as_text(&out);
    assert!(
        text.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#),
        "{text}"
    );
    assert!(text.contains(RUSSIAN));
}

#[test]
fn a_utf16_feed_is_read_through_its_bom() {
    for big_endian in [false, true] {
        let xml = r#"<?xml version="1.0" encoding="UTF-16"?><rss><channel><title>Привет</title></channel></rss>"#;
        let body = utf16(xml, big_endian);
        let out = feed_to_utf8(&body, Some("text/xml; charset=utf-8"));
        let text = as_text(&out);
        assert!(text.contains("Привет"), "{text}");
        assert!(text.contains(r#"encoding="UTF-8""#), "{text}");
    }
}

#[test]
fn a_utf16_declaration_without_a_bom_reads_as_utf8() {
    let body = feed_with(r#"<?xml version="1.0" encoding="UTF-16"?>"#, "é".as_bytes());
    assert!(as_text(&feed_to_utf8(&body, None)).contains('é'));
}

#[test]
fn malformed_declarations_fall_back_to_utf8() {
    for decl in [
        r#"<?xml version="1.0" encoding=windows-1251?>"#,
        r#"<?xml version="1.0" encoding="windows-1251?>"#,
        r#"<?xml version="1.0" encoding="klingon"?>"#,
        r#"<?xml version="1.0" encoding"#,
    ] {
        let body = feed_with(decl, "é".as_bytes());
        let out = feed_to_utf8(&body, None);
        assert!(as_text(&out).contains('é'), "{decl}");
    }
}

#[test]
fn a_stylesheet_pi_is_not_mistaken_for_the_declaration() {
    let body = feed_with(
        r#"<?xml-stylesheet href="s.xsl"?>"#,
        &encode(WINDOWS_1251, RUSSIAN),
    );
    let out = feed_to_utf8(&body, Some("text/xml; charset=windows-1251"));
    let text = as_text(&out);
    assert!(text.starts_with("<?xml-stylesheet"), "{text}");
    assert!(text.contains(RUSSIAN));
}

#[test]
fn html_pages_are_recognised_by_type_or_by_sniffing() {
    assert!(is_html_page(Some("text/html; charset=utf-8"), b""));
    assert!(is_html_page(Some("application/xhtml+xml"), b""));
    assert!(is_html_page(None, b"<html></html>"));
    assert!(is_html_page(Some("application/octet-stream"), b"<p>x</p>"));
    assert!(
        is_html_page(Some(""), b"<p>x</p>"),
        "an unusable type sniffs"
    );
    // UTF-16 text is full of NULs, but its BOM marks it as text.
    for big_endian in [false, true] {
        let body = utf16("<html><p>Привет</p></html>", big_endian);
        assert!(is_html_page(None, &body));
        assert!(is_html_page(Some("application/octet-stream"), &body));
    }
}

#[test]
fn pdfs_images_and_other_types_are_not_pages() {
    assert!(!is_html_page(Some("application/pdf"), b"<html>"));
    assert!(!is_html_page(Some("image/png"), b""));
    assert!(!is_html_page(Some("text/plain"), b""));
    assert!(!is_html_page(Some("application/json"), b"{}"));
    for body in [
        &b"%PDF-1.7\n"[..],
        b"\x89PNG\r\n\x1a\n....",
        b"\xff\xd8\xff\xe0",
        b"GIF89a",
        b"GIF87a",
        b"RIFF\0\0\0\0WEBPVP8 ",
        b"\x00\x00\x01\x00", // ICO, caught by its NULs
        b"text with a \0 nul",
    ] {
        assert!(!is_html_page(None, body), "{body:?}");
        assert!(!is_html_page(Some("application/octet-stream"), body));
    }
}
