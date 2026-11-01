//! Turning an import file's raw bytes into text: a bounded read plus
//! encoding detection, so a Latin-1 OPML or an Excel UTF-16 CSV imports
//! instead of being rejected as "not UTF-8".

use std::io::Read as _;
use std::path::Path;

use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252};

/// The largest import file Curio reads (50 MiB). Real subscription lists
/// and read-later exports are kilobytes to a few megabytes; anything bigger
/// is the wrong file, and reading it whole would only exhaust memory.
pub const MAX_IMPORT_BYTES: u64 = 50 * 1024 * 1024;

/// Reads an import file and decodes it to text (see [`decode_text`]),
/// refusing files over [`MAX_IMPORT_BYTES`] without reading them whole.
///
/// # Errors
///
/// The file's I/O error, or [`std::io::ErrorKind::FileTooLarge`] for a file
/// over the cap.
pub fn read_import_file(path: &Path) -> std::io::Result<String> {
    let file = std::fs::File::open(path)?;
    let mut bytes = Vec::new();
    // One byte past the cap is enough to know the file is too big.
    file.take(MAX_IMPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_IMPORT_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::FileTooLarge,
            format!(
                "the import file is larger than the {} MiB limit",
                MAX_IMPORT_BYTES / (1024 * 1024)
            ),
        ));
    }
    Ok(decode_text(&bytes))
}

/// Decodes an import file's bytes to text, never failing:
///
/// 1. a byte-order mark wins (UTF-8, UTF-16LE, UTF-16BE);
/// 2. BOM-less UTF-16 is recognized by its zero high bytes around the
///    first (ASCII) character — every OPML and CSV starts with one;
/// 3. an XML declaration's `encoding="…"` label is honoured (`ISO-8859-1`,
///    `windows-1252`, `Shift_JIS`, … — WHATWG label resolution);
/// 4. otherwise valid UTF-8 is taken as is, and anything else is read as
///    Windows-1252 (the superset of Latin-1 that every legacy Windows and
///    Mac exporter actually wrote), which maps every byte to a character.
///
/// A declared `UTF-8` that is not valid UTF-8 falls through to step 4 —
/// exporters that stamp a UTF-8 header over Windows-1252 bytes are common.
#[must_use]
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some((encoding, bom_len)) = Encoding::for_bom(bytes) {
        return decode_with(encoding, bytes.get(bom_len..).unwrap_or_default());
    }
    if let Some(encoding) = sniff_utf16(bytes) {
        return decode_with(encoding, bytes);
    }
    if let Some(encoding) = xml_declared_encoding(bytes)
        && encoding != UTF_8
        && encoding.is_ascii_compatible()
    {
        return decode_with(encoding, bytes);
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text.to_owned(),
        Err(_) => decode_with(WINDOWS_1252, bytes),
    }
}

/// Decodes without BOM sniffing (the caller already handled any BOM);
/// malformed sequences become U+FFFD rather than failing.
fn decode_with(encoding: &'static Encoding, bytes: &[u8]) -> String {
    encoding.decode_without_bom_handling(bytes).0.into_owned()
}

/// BOM-less UTF-16: the first code unit is an ASCII character (`<`, a
/// header letter), so exactly one of the first two bytes is zero.
fn sniff_utf16(bytes: &[u8]) -> Option<&'static Encoding> {
    match bytes {
        [lo, 0, ..] if *lo != 0 && lo.is_ascii() => Some(UTF_16LE),
        [0, lo, ..] if *lo != 0 && lo.is_ascii() => Some(UTF_16BE),
        _ => None,
    }
}

/// The encoding an XML declaration (`<?xml … encoding="…"?>`) names, if
/// the document opens with one and the label is known.
fn xml_declared_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    let start = bytes.iter().position(|b| !b.is_ascii_whitespace())?;
    let rest = bytes.get(start..)?;
    if !rest.starts_with(b"<?xml") {
        return None;
    }
    let end = rest.windows(2).position(|pair| pair == b"?>")?;
    let declaration = std::str::from_utf8(rest.get(..end)?).ok()?;
    let after = declaration.split_once("encoding")?.1.trim_start();
    let after = after.strip_prefix('=')?.trim_start();
    let quote = after.chars().next().filter(|c| matches!(c, '"' | '\''))?;
    let label = after.get(1..)?.split(quote).next()?;
    Encoding::for_label(label.trim().as_bytes())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn utf16le(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        out.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        out
    }

    fn utf16be(text: &str, bom: bool) -> Vec<u8> {
        let mut out = if bom { vec![0xFE, 0xFF] } else { Vec::new() };
        out.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
        out
    }

    #[test]
    fn plain_utf8_passes_through_and_a_utf8_bom_is_stripped() {
        assert_eq!(decode_text("café".as_bytes()), "café");
        let mut with_bom = vec![0xEF, 0xBB, 0xBF];
        with_bom.extend_from_slice("url,title\n".as_bytes());
        assert_eq!(decode_text(&with_bom), "url,title\n");
    }

    #[test]
    fn utf16_decodes_with_or_without_a_bom() {
        let text = "url,title\nhttps://example.com/é,Café\n";
        assert_eq!(decode_text(&utf16le(text, true)), text);
        assert_eq!(decode_text(&utf16le(text, false)), text);
        assert_eq!(decode_text(&utf16be(text, true)), text);
        assert_eq!(decode_text(&utf16be(text, false)), text);
    }

    #[test]
    fn the_xml_declaration_names_the_encoding() {
        // 0xE9 is `é` in Latin-1 / Windows-1252 — invalid as UTF-8.
        let latin1 = b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><opml title=\"Caf\xE9\"/>";
        assert_eq!(
            decode_text(latin1),
            "<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><opml title=\"Café\"/>"
        );
        // Single quotes and whitespace around `=` are both legal XML.
        let quoted = b"  <?xml version='1.0' encoding = 'windows-1252' ?><a>\x93q\x94</a>";
        assert!(decode_text(quoted).ends_with("<a>\u{201C}q\u{201D}</a>"));
    }

    #[test]
    fn undeclared_or_mislabelled_bytes_fall_back_to_windows_1252() {
        assert_eq!(decode_text(b"title\nCaf\xE9\n"), "title\nCafé\n");
        // Declared UTF-8, written as Windows-1252.
        let liar = b"<?xml version=\"1.0\" encoding=\"UTF-8\"?><a>Caf\xE9</a>";
        assert!(decode_text(liar).ends_with("<a>Café</a>"));
        // An unknown label is ignored rather than trusted.
        let unknown = b"<?xml version=\"1.0\" encoding=\"klingon\"?><a/>";
        assert!(decode_text(unknown).ends_with("<a/>"));
        // A declaration that never closes names nothing.
        assert_eq!(
            decode_text(b"<?xml encoding=\"latin1\""),
            "<?xml encoding=\"latin1\""
        );
    }

    #[test]
    fn empty_input_is_empty_text() {
        assert_eq!(decode_text(b""), "");
    }

    #[test]
    fn read_import_file_decodes_and_caps_the_size() {
        let dir = tempfile::tempdir().unwrap();
        let small = dir.path().join("subs.csv");
        std::fs::write(&small, utf16le("url\nhttps://a.test/\n", true)).unwrap();
        assert_eq!(read_import_file(&small).unwrap(), "url\nhttps://a.test/\n");

        let big = dir.path().join("huge.opml");
        let file = std::fs::File::create(&big).unwrap();
        // Sparse: a 1-byte-over-the-cap file costs no real disk.
        file.set_len(MAX_IMPORT_BYTES + 1).unwrap();
        let error = read_import_file(&big).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
        assert!(error.to_string().contains("50 MiB"), "{error}");

        let missing = read_import_file(&dir.path().join("missing.opml")).unwrap_err();
        assert_eq!(missing.kind(), std::io::ErrorKind::NotFound);
    }
}
