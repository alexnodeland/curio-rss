//! Note filename stems that are safe on every OS a destination may live on.
//!
//! A slug becomes `curio/<slug>.md` (possibly `<slug>-<curio_id prefix>`
//! or `<slug>-<curio_id>` after deduplication), so it has to survive the
//! strictest of the filesystems a notes vault syncs across:
//!
//! - **Byte-capped, not char-capped.** `NAME_MAX` is 255 *bytes* on ext4
//!   and APFS; 80 CJK or emoji characters are already 240–320 bytes. The
//!   stem is capped at [`SLUG_MAX_BYTES`] so even the longest dedup suffix
//!   (`-` + a 36-byte UUID + `.md`) keeps the name far below the limit.
//! - **NFC-normalized.** APFS/HFS+ treat NFC and NFD spellings of "café"
//!   as the same file; without normalization the two spellings would get
//!   two manifest paths for one on-disk note, and the second export would
//!   overwrite the first article's note.
//! - **Never a Windows reserved device name.** `con.md`, `nul.md`,
//!   `com1.md`… cannot be created on Windows regardless of extension, so
//!   such stems get a `-note` suffix.

use curio_types::CurioId;
use icu_normalizer::ComposingNormalizerBorrowed;

/// Byte budget of a slug, before any dedup suffix and the `.md` extension.
/// Longest final name: 100 + `-` + 36-byte UUID + `.md` = 140 bytes. The
/// longest name actually *created* is the write's temp file,
/// `.{name}.curio-tmp-{pid}-{seq}`: 1 + 140 + 11 + up to 10 pid digits +
/// 1 + up to 20 sequence digits = 183 bytes. Both stay well under the 255-byte `NAME_MAX` of ext4/APFS/NTFS
/// (NTFS counts UTF-16 units, never more than UTF-8 bytes) — and it was
/// the temp name that overflowed before this cap existed.
pub(crate) const SLUG_MAX_BYTES: usize = 100;

/// Windows device names, reserved with *any* extension (`CON.md` is still
/// the console). Compared against the already-lowercased stem; includes
/// the superscript-digit ports Windows also reserves (`¹²³` are
/// alphanumeric, so they can reach a slug).
const WINDOWS_RESERVED: &[&str] = &[
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7",
    "com8", "com9", "com¹", "com²", "com³", "lpt0", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6",
    "lpt7", "lpt8", "lpt9", "lpt¹", "lpt²", "lpt³",
];

/// Suffix that takes a reserved stem out of the device namespace.
const RESERVED_SUFFIX: &str = "-note";

/// Turns a title into a lowercase, dash-separated, NFC-normalized stem of
/// at most [`SLUG_MAX_BYTES`] bytes that is a legal filename everywhere.
/// Titles with no alphanumerics fall back to the `curio_id`.
pub(crate) fn slugify(title: &str, curio_id: &CurioId) -> String {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    // Normalize first: an NFD combining accent is not alphanumeric, so
    // without composing it the loop below would split "cafe\u{301}" at the
    // accent and drop it.
    let title = nfc.normalize(title);
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            let dash = usize::from(pending_dash && !slug.is_empty());
            if slug.len() + dash + c.len_utf8() > SLUG_MAX_BYTES {
                break;
            }
            if dash == 1 {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c);
        } else {
            pending_dash = true;
        }
    }
    // Lowercasing can (rarely) leave a non-NFC sequence; a second pass
    // makes the output canonical. It may change the length, so re-cap.
    let slug = truncate_to_bytes(nfc.normalize(&slug).into_owned(), SLUG_MAX_BYTES);
    if slug.is_empty() {
        return curio_id.to_string();
    }
    if WINDOWS_RESERVED.contains(&slug.as_str()) {
        return format!("{slug}{RESERVED_SUFFIX}");
    }
    slug
}

/// Cuts `s` to at most `max` bytes on a char boundary, never leaving a
/// trailing separator.
fn truncate_to_bytes(mut s: String, max: usize) -> String {
    if s.len() > max {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        s.truncate(end);
    }
    while s.ends_with('-') {
        s.pop();
    }
    s
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn slugify_is_boring_and_safe() {
        let id = CurioId::new();
        assert_eq!(slugify("Hello, World!", &id), "hello-world");
        assert_eq!(slugify("../../etc/passwd", &id), "etc-passwd");
        assert_eq!(slugify("Ünïcode Tîtle", &id), "ünïcode-tîtle");
        assert_eq!(slugify("", &id), id.to_string());
        assert_eq!(slugify("!!!", &id), id.to_string());
    }

    #[test]
    fn nfc_and_nfd_spellings_produce_one_slug() {
        let id = CurioId::new();
        let nfc = "Caf\u{e9} Cr\u{e8}me";
        let nfd = "Cafe\u{301} Cre\u{300}me";
        assert_ne!(nfc, nfd, "the fixture must really differ in bytes");
        assert_eq!(slugify(nfc, &id), "caf\u{e9}-cr\u{e8}me");
        assert_eq!(slugify(nfd, &id), slugify(nfc, &id));
        // Hangul is the other big NFD source (macOS file names, some
        // clipboard paths): conjoining jamo compose to one syllable.
        assert_eq!(slugify("\u{1112}\u{1161}\u{11ab}", &id), "\u{d55c}");
    }

    #[test]
    fn long_multibyte_titles_stay_under_name_max_with_the_longest_suffix() {
        let id = CurioId::new();
        for title in [
            "漢".repeat(200),
            "😀".repeat(200),
            "é".repeat(200),
            format!("{} {}", "a".repeat(99), "漢".repeat(50)),
        ] {
            let slug = slugify(&title, &id);
            assert!(slug.len() <= SLUG_MAX_BYTES, "{} bytes", slug.len());
            assert!(!slug.is_empty() && !slug.ends_with('-'));
            // The longest name unique_note_path can produce…
            let file_name = format!("{slug}-{id}.md");
            assert!(file_name.len() <= 255, "{} bytes", file_name.len());
            // …and the longest one a write creates: its temp file, with
            // the widest possible pid and sequence number.
            let temp_name = format!(".{file_name}.curio-tmp-{}-{}", u32::MAX, u64::MAX);
            assert!(temp_name.len() <= 255, "{} bytes", temp_name.len());
        }
        // 200 CJK chars = 600 bytes; the cap lands on a char boundary.
        assert_eq!(slugify(&"漢".repeat(200), &id), "漢".repeat(33));
    }

    #[test]
    fn ascii_titles_are_capped_by_bytes() {
        let id = CurioId::new();
        let slug = slugify(&"word ".repeat(60), &id);
        assert!(slug.len() <= SLUG_MAX_BYTES);
        assert!(slug.starts_with("word-word"));
        assert!(!slug.ends_with('-'));
    }

    #[test]
    fn windows_reserved_device_names_are_suffixed() {
        let id = CurioId::new();
        for title in [
            "CON", "Con", "con", "PRN", "aux", "NUL", "nul.", "COM1", "lpt9", "COM¹", "LPT0",
        ] {
            let slug = slugify(title, &id);
            assert!(
                !WINDOWS_RESERVED.contains(&slug.as_str()),
                "{title:?} produced reserved stem {slug:?}"
            );
            assert!(slug.ends_with(RESERVED_SUFFIX), "{title:?} → {slug:?}");
        }
        assert_eq!(slugify("CON", &id), "con-note");
        // Reserved only as the whole stem: these are ordinary names.
        assert_eq!(slugify("Console", &id), "console");
        assert_eq!(slugify("con game", &id), "con-game");
        assert_eq!(slugify("COM10", &id), "com10");
    }

    #[test]
    fn truncation_never_splits_a_char_or_leaves_a_dash() {
        assert_eq!(truncate_to_bytes("ab-漢".to_owned(), 4), "ab");
        assert_eq!(truncate_to_bytes("abc".to_owned(), 10), "abc");
        assert_eq!(truncate_to_bytes("ab-".to_owned(), 3), "ab");
    }
}
