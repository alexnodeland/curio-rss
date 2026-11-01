//! Where the YAML 1.2 parser is stricter than the libyaml reader the notes
//! in users' vaults were written for, the text is mended so hand-edited
//! frontmatter that read before still reads, into the same values.
//!
//! Lone CR and NEL breaks are rewritten before the parse
//! ([`normalize_breaks`]). The rest is driven by saphyr-parser's own
//! refusals: each names a spot libyaml took, [`repair`] mends that spot
//! without changing any value, and the caller parses again:
//!
//! - a TAB as the only separator after a mapping `:` (`key:<TAB>value`);
//! - continuation lines of a multi-line quoted scalar indented no deeper
//!   than their key (`summary: "a long\nline"`), or led by a TAB;
//! - continuation lines of a flow collection indented no deeper than
//!   their key (`tags: [a,\nb]`);
//! - a comment right after a token (`title: 'x'#note`, `body: |#note`).
//!
//! Every mend only touches blanks the YAML reader discards: the separator
//! after a `:` or before a comment, or the leading blanks of a
//! continuation line, which flow and quoted scalars fold away. Curio's own output never needs one.

use saphyr_parser::ScanError;

/// `key:<TAB>value`.
const TAB_AFTER_COLON: &str = "':' must be followed by a valid YAML whitespace";
/// A continuation line of a quoted scalar at or left of its key's column;
/// the mark is the opening quote.
const QUOTED_UNDER_INDENTED: &str = "invalid indentation in quoted scalar";
/// A TAB leading an under-indented continuation line of a quoted scalar;
/// the mark is the TAB.
const QUOTED_TAB_INDENT: &str = "tab cannot be used as indentation";
/// A plain scalar opening an under-indented line of a flow collection.
const FLOW_UNDER_INDENTED: &str = "invalid indentation in flow construct";
/// Any other token opening an under-indented line of a flow collection
/// (block context unrolls its indentation before this check, so only flow
/// context reaches it).
const UNDER_INDENTED: &str = "invalid indentation";
/// A `#` straight after a token (a closing quote, a block scalar header);
/// the mark is the `#`.
const UNSEPARATED_COMMENT: &str = "comments must be separated from other tokens by whitespace";

/// A lone CR and NEL (U+0085) are line breaks to libyaml, which reads
/// both as LF everywhere: between tokens, inside quoted scalars (where
/// the break folds to a space) and inside block scalars. The YAML 1.2
/// parser reads NEL as content and a lone CR as a break that its quoted
/// scalars mis-indent, so both become LF up front. Curio's emitter
/// escapes both, so only hand-edited notes carry them raw. Char for char,
/// so lines and columns stay put.
pub(super) fn normalize_breaks(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' if chars.peek() != Some(&'\n') => out.push('\n'),
            '\u{85}' => out.push('\n'),
            c => out.push(c),
        }
    }
    out
}

/// Mends the spot `err` refuses, if it is one libyaml read; `false` when
/// it is not (the caller reports `err`).
pub(super) fn repair(text: &mut String, err: &ScanError) -> bool {
    let at = byte_at(text, err.marker().index());
    match err.info() {
        TAB_AFTER_COLON => tab_after_colon(text, at),
        QUOTED_UNDER_INDENTED => indent_quoted(text, at, err.marker().col()),
        QUOTED_TAB_INDENT => tab_to_space(text, at),
        FLOW_UNDER_INDENTED | UNDER_INDENTED => indent_line(text, at),
        UNSEPARATED_COMMENT => separate_comment(text, at),
        _ => false,
    }
}

/// The byte offset of the `index`th char (parser marks count chars).
fn byte_at(text: &str, index: usize) -> usize {
    text.char_indices()
        .nth(index)
        .map_or(text.len(), |(at, _)| at)
}

/// The blanks (spaces and TABs, a column each) opening `line`.
fn leading_blanks(line: &str) -> usize {
    line.chars().take_while(|c| matches!(c, ' ' | '\t')).count()
}

/// libyaml (and YAML 1.2's `s-separate-in-line`) take a TAB as the
/// separator after a mapping `:`; saphyr-parser 0.3 refuses one no space
/// joins when a plain scalar or `-` follows, marking the end of the blanks
/// after that `:`. The first TAB there becomes a space: a separator
/// either way, and one char for one.
fn tab_after_colon(text: &mut String, end: usize) -> bool {
    let colon = text[..end].trim_end_matches([' ', '\t']).len();
    match text[colon..end].find('\t') {
        Some(tab) if text[..colon].ends_with(':') => {
            text.replace_range(colon + tab..=colon + tab, " ");
            true
        }
        _ => false,
    }
}

/// libyaml let a quoted scalar's continuation lines sit at any column;
/// YAML 1.2 wants them right of the key. Every continuation line of the
/// scalar opening at `quote` is padded out to the quote's `column` (its
/// key is left of it); a continuation line's leading blanks fold away, so
/// the value is unchanged. `false` when no line needed padding.
fn indent_quoted(text: &mut String, quote: usize, column: usize) -> bool {
    let single = match text[quote..].chars().next() {
        Some('\'') => true,
        Some('"') => false,
        _ => return false,
    };
    let body = quote + 1;
    // (line start, spaces to insert), in text order.
    let mut pads = Vec::new();
    let mut chars = text[body..].char_indices().peekable();
    let mut escaped = false;
    while let Some((offset, c)) = chars.next() {
        if std::mem::take(&mut escaped) && c != '\n' {
            continue;
        }
        match c {
            '\n' => {
                let start = body + offset + 1;
                let blanks = leading_blanks(&text[start..]);
                if blanks < column {
                    pads.push((start, column - blanks));
                }
            }
            '\\' if !single => escaped = true,
            '\'' if single => {
                // `''` is an escaped quote; a lone one closes the scalar.
                if chars.next_if(|&(_, next)| next == '\'').is_none() {
                    break;
                }
            }
            '"' if !single => break,
            _ => {}
        }
    }
    for &(start, count) in pads.iter().rev() {
        text.insert_str(start, &" ".repeat(count));
    }
    !pads.is_empty()
}

/// The TAB at `at` leads a continuation line of a quoted scalar; as a
/// space it folds away the same (and [`indent_quoted`] can then pad it).
fn tab_to_space(text: &mut String, at: usize) -> bool {
    if text[at..].starts_with('\t') {
        text.replace_range(at..=at, " ");
        true
    } else {
        false
    }
}

/// libyaml read a `#` straight after a token as a comment; YAML 1.2 wants
/// a blank before it, so one goes in.
fn separate_comment(text: &mut String, at: usize) -> bool {
    if text[at..].starts_with('#') {
        text.insert(at, ' ');
        true
    } else {
        false
    }
}

/// libyaml let a flow collection's continuation lines sit at any column;
/// YAML 1.2 wants them right of the key. The line holding `at` is padded
/// one column past the line above (which opened the collection, or is an
/// already-padded continuation of it), and at least one column past where
/// it was, so repeated refusals of one line always make progress. Inside a
/// flow collection a line's leading blanks are separation (or fold away
/// in a multi-line scalar), so no value changes.
fn indent_line(text: &mut String, at: usize) -> bool {
    let Some(newline) = text[..at].rfind('\n') else {
        return false;
    };
    let start = newline + 1;
    let above = text[..newline].rfind('\n').map_or(0, |i| i + 1);
    let own = leading_blanks(&text[start..]);
    let want = (leading_blanks(&text[above..]) + 1).max(own + 1);
    text.insert_str(start, &" ".repeat(want - own));
    true
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use saphyr_parser::Parser;

    use super::*;

    fn scan_error(input: &str) -> ScanError {
        match Parser::new_from_str(input).find_map(Result::err) {
            Some(err) => err,
            None => panic!("{input:?} parsed"),
        }
    }

    fn repaired(input: &str) -> Option<String> {
        let err = scan_error(input);
        let mut text = input.to_owned();
        repair(&mut text, &err).then_some(text)
    }

    #[test]
    fn breaks_normalize_char_for_char() {
        assert_eq!(normalize_breaks("a\rb\r\nc\u{85}d"), "a\nb\r\nc\nd");
        assert_eq!(normalize_breaks("x\r"), "x\n");
    }

    #[test]
    fn each_refusal_mends_its_spot() {
        assert_eq!(repaired("a:\tb").as_deref(), Some("a: b"));
        assert_eq!(repaired("a:\t\tb").as_deref(), Some("a: \tb"));
        assert_eq!(repaired("a: \"x\ny\"").as_deref(), Some("a: \"x\n   y\""));
        assert_eq!(
            repaired("a: 'it''s\nok'").as_deref(),
            Some("a: 'it''s\n   ok'")
        );
        assert_eq!(
            repaired("a: \"x\\\"\\\ny\"").as_deref(),
            Some("a: \"x\\\"\\\n   y\"")
        );
        assert_eq!(
            repaired("a:\n  b: \"x\n\ty\"").as_deref(),
            Some("a:\n  b: \"x\n y\"")
        );
        assert_eq!(repaired("t: [a,\nb]").as_deref(), Some("t: [a,\n b]"));
        assert_eq!(repaired("t: 'x'#c").as_deref(), Some("t: 'x' #c"));
        assert_eq!(
            repaired("a:\n  b: [x,\n y]").as_deref(),
            Some("a:\n  b: [x,\n   y]")
        );
    }

    #[test]
    fn refusals_libyaml_shared_are_left_alone() {
        // A different refusal entirely.
        assert_eq!(repaired("a: [1"), None);
        // Marks that do not point where the mend expects.
        let colon = scan_error("a:\tb");
        let quoted = scan_error("a: \"x\ny\"");
        let tab = scan_error("a:\n  b: \"x\n\ty\"");
        let comment = scan_error("t: 'x'#c");
        for (err, text) in [
            (&comment, "t: 'x' c"),
            (&colon, "ab\tb"),
            (&colon, "a: b"),
            (&quoted, "a: x\ny"),
            (&quoted, "a: \"x y\""),
            (&tab, "a:\n  b: \"x\n  y\""),
        ] {
            let mut mended = text.to_owned();
            assert!(!repair(&mut mended, err), "{text:?}");
            assert_eq!(mended, text);
        }
        // An unterminated quoted scalar is padded to its end.
        let mut text = "a: \"x\ny".to_owned();
        assert!(indent_quoted(&mut text, 3, 3));
        assert_eq!(text, "a: \"x\n   y");
        // The first line has no line above to continue.
        let mut text = "[x]".to_owned();
        assert!(!indent_line(&mut text, 1));
    }
}
