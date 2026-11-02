//! CJK segmentation for the full-text index (#138).
//!
//! FTS5's `unicode61` tokenizer splits on spaces and punctuation, so a run
//! of Chinese, Japanese or Korean text with no spaces — `東京都の天気` —
//! indexes as **one** token and nothing inside it can be found: `天気`
//! misses. Rather than replace the tokenizer (a trigram index cannot match
//! the two-character words most CJK queries are, and a custom tokenizer
//! needs `unsafe` FFI), the text is segmented before `unicode61` sees it:
//! every CJK run is rewritten as its overlapping **bigrams** followed by
//! its last character on its own —
//!
//! ```text
//! 東京都の天気  →  東京 京都 都の の天 天気 気
//! ```
//!
//! — while every other character passes through untouched, so non-CJK
//! text indexes exactly as before. A query is segmented the same way (see
//! [`segment_query`]), which turns any substring of a run into a phrase of
//! adjacent bigrams: `天気` → `"天気"`, `京都の` → `"京都 都の"`. The
//! trailing single character makes every character of a run the start of
//! some token, so a one-character query matches as a prefix (`"天"*`)
//! wherever that character stands.
//!
//! The index side is the SQL function [`INDEX_FUNCTION`], called by the
//! `articles_fts` triggers and the `articles_fts_source` view (migration
//! 0016), so every connection that writes articles or reads the index's
//! columns must have it registered ([`register_functions`]). Changing the
//! segmentation changes the tokens on disk: it needs a migration that
//! rebuilds the index.

use std::borrow::Cow;

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

/// The SQL name of the index-side segmenter (see the module docs).
pub(crate) const INDEX_FUNCTION: &str = "curio_fts_segment";

/// Registers Curio's SQL functions on `conn`. Every connection that writes
/// `articles` (its triggers maintain the search index through the CJK
/// segmenter `curio_fts_segment`), rebuilds or checks the search index, or
/// reads its columns needs them. Every connection [`super::Storage`] opens
/// has them; call this on a connection of your own before writing.
///
/// # Errors
///
/// `SQLite` refusing the registration.
pub fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    conn.create_scalar_function(
        INDEX_FUNCTION,
        1,
        // Deterministic and side-effect free: safe in triggers and views
        // even under `trusted_schema = OFF`.
        FunctionFlags::SQLITE_UTF8
            | FunctionFlags::SQLITE_DETERMINISTIC
            | FunctionFlags::SQLITE_INNOCUOUS,
        |ctx| {
            let text: Option<String> = ctx.get(0)?;
            Ok(text.map(|text| {
                // Text without CJK (most of it) goes back as it came in,
                // with no second copy.
                let segmented = match segment_index(&text) {
                    Cow::Owned(segmented) => Some(segmented),
                    Cow::Borrowed(_) => None,
                };
                segmented.unwrap_or(text)
            }))
        },
    )
}

/// Whether `c` is a CJK letter the index segments: Han ideographs, kana,
/// Hangul and Bopomofo — the scripts written without spaces between
/// words. Only letters and numbers: CJK punctuation (`。`, `・`, `「`) is
/// a separator to `unicode61` already.
pub(crate) fn is_cjk(c: char) -> bool {
    matches!(
        c,
        '\u{1100}'..='\u{11FF}'     // Hangul Jamo
        | '\u{3005}'..='\u{3007}'   // 々 〆 〇
        | '\u{3021}'..='\u{3029}'   // Hangzhou numerals
        | '\u{3031}'..='\u{3035}'   // kana repeat marks
        | '\u{303B}'..='\u{303C}'   // 〻 〼
        | '\u{3040}'..='\u{30FF}'   // Hiragana, Katakana
        | '\u{3100}'..='\u{312F}'   // Bopomofo
        | '\u{3130}'..='\u{318F}'   // Hangul Compatibility Jamo
        | '\u{31A0}'..='\u{31BF}'   // Bopomofo Extended
        | '\u{31F0}'..='\u{31FF}'   // Katakana Phonetic Extensions
        | '\u{3400}'..='\u{4DBF}'   // CJK Extension A
        | '\u{4E00}'..='\u{9FFF}'   // CJK Unified Ideographs
        | '\u{A960}'..='\u{A97F}'   // Hangul Jamo Extended-A
        | '\u{AC00}'..='\u{D7FF}'   // Hangul Syllables, Jamo Extended-B
        | '\u{F900}'..='\u{FAFF}'   // CJK Compatibility Ideographs
        | '\u{FF66}'..='\u{FFDC}'   // Halfwidth Katakana and Hangul
        | '\u{20000}'..='\u{3FFFF}' // CJK Extensions B and later
    ) && c.is_alphanumeric()
}

/// How a CJK run is written out.
#[derive(Clone, Copy, PartialEq, Eq)]
enum RunEnd {
    /// The run is known to end here: its bigrams, then its last character
    /// alone — the document form, and a query run followed by more words.
    Closed,
    /// The text ends inside the run, which may go on in a document: its
    /// bigrams only (a one-character run stays one character, and the
    /// caller matches it as a prefix).
    Open,
}

/// Appends `run`'s tokens to `out`, space-separated from each other and
/// from any token already there.
fn push_run(out: &mut String, run: &[char], end: RunEnd) {
    let mut push_token = |token: &[char]| {
        if out.chars().next_back().is_some_and(|c| !c.is_whitespace()) {
            out.push(' ');
        }
        out.extend(token);
    };
    for pair in run.windows(2) {
        push_token(pair);
    }
    if end == RunEnd::Closed || run.len() == 1 {
        push_token(&run[run.len().saturating_sub(1)..]);
    }
}

/// Segments `text` with every run [`RunEnd::Closed`] except, when
/// `last_open`, one that ends the text's letters and digits.
fn segment(text: &str, last_open: bool) -> Cow<'_, str> {
    if !text.chars().any(is_cjk) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len() * 3);
    let mut run: Vec<char> = Vec::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if is_cjk(c) {
            run.push(c);
            if chars.peek().is_some_and(|&next| is_cjk(next)) {
                continue;
            }
            let end = if last_open && !chars.clone().any(char::is_alphanumeric) {
                RunEnd::Open
            } else {
                RunEnd::Closed
            };
            push_run(&mut out, &run, end);
            run.clear();
            // Separate the run from a token glued to its end (`製品a`).
            if chars.peek().is_some_and(|&next| !next.is_whitespace()) {
                out.push(' ');
            }
        } else {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// The index form of a title or body (see the module docs): borrowed
/// unchanged when it holds no CJK.
pub(crate) fn segment_index(text: &str) -> Cow<'_, str> {
    segment(text, false)
}

/// The query form of one atom's text, and whether the atom must match as
/// a prefix. A CJK run followed by more letters or digits ends there in any
/// matching document, so it is segmented exactly as the index wrote it; the
/// run the atom ends on may continue in the document, so it contributes
/// its bigrams only — and, when it is a single character, matches as a
/// prefix (`天` → `"天"*`, which reaches every `天` in the index).
pub(crate) fn segment_query(text: &str) -> (Cow<'_, str>, bool) {
    let segmented = segment(text, true);
    let lone_last = {
        let mut tail = text.chars().rev().skip_while(|c| !c.is_alphanumeric());
        tail.next().is_some_and(is_cjk) && !tail.next().is_some_and(is_cjk)
    };
    (segmented, lone_last)
}

#[cfg(test)]
mod tests {
    // Test code: a panicking unwrap is the failure report.
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn tokens(text: &str) -> Vec<&str> {
        text.split_whitespace().collect()
    }

    #[test]
    fn index_form_is_bigrams_then_the_last_character() {
        assert_eq!(
            tokens(&segment_index("東京都の天気")),
            ["東京", "京都", "都の", "の天", "天気", "気"]
        );
        assert_eq!(tokens(&segment_index("雨")), ["雨"]);
        assert_eq!(
            tokens(&segment_index("서울의 날씨")),
            ["서울", "울의", "의", "날씨", "씨"]
        );
    }

    #[test]
    fn non_cjk_text_is_borrowed_unchanged() {
        let text = "Ünïcode café — rust 2024, naïve";
        assert!(matches!(segment_index(text), Cow::Borrowed(t) if t == text));
        assert!(matches!(segment_query(text).0, Cow::Borrowed(t) if t == text));
    }

    #[test]
    fn runs_are_split_from_adjacent_latin_and_punctuation() {
        assert_eq!(
            tokens(&segment_index("iPhone発売。2024年、Rust製")),
            ["iPhone", "発売", "売", "。2024", "年", "、Rust", "製"]
        );
    }

    #[test]
    fn cjk_punctuation_is_not_a_letter() {
        assert!(is_cjk('天'));
        assert!(is_cjk('の'));
        assert!(is_cjk('カ'));
        assert!(is_cjk('한'));
        assert!(is_cjk('々'));
        assert!(is_cjk('ｶ'));
        assert!(!is_cjk('。'));
        assert!(!is_cjk('・'));
        assert!(!is_cjk('「'));
        assert!(!is_cjk('a'));
        assert!(!is_cjk('７'));
    }

    #[test]
    fn query_form_leaves_the_last_run_open() {
        assert_eq!(
            segment_query("天気"),
            (Cow::Owned("天気".to_owned()), false)
        );
        assert_eq!(
            tokens(&segment_query("京都の").0),
            ["京都", "都の"],
            "no trailing character: the document's run may go on"
        );
        assert_eq!(
            tokens(&segment_query("東京 tower").0),
            ["東京", "京", "tower"],
            "a run followed by more words ends there in the document too"
        );
        assert_eq!(tokens(&segment_query("東京。").0), ["東京", "。"]);
    }

    #[test]
    fn a_lone_last_character_asks_for_a_prefix() {
        assert_eq!(segment_query("天"), (Cow::Owned("天".to_owned()), true));
        assert!(segment_query("rust 天。").1);
        assert!(segment_query("Rust製").1);
        assert!(!segment_query("天 rust").1);
        assert!(!segment_query("天気").1);
        assert!(!segment_query("rust").1);
    }

    #[test]
    fn the_sql_function_segments_and_passes_null_through() {
        let conn = Connection::open_in_memory().unwrap();
        register_functions(&conn).unwrap();
        let (segmented, null): (String, Option<String>) = conn
            .query_row(
                &format!("SELECT {INDEX_FUNCTION}('東京'), {INDEX_FUNCTION}(NULL)"),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tokens(&segmented), ["東京", "京"]);
        assert_eq!(null, None);
    }
}
