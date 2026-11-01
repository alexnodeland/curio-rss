//! YAML text → the event list the deserializer replays: one document
//! from [`saphyr_parser`], aliases resolved to the index of the node they
//! name. Where the parser refuses a spot the old libyaml reader took,
//! [`super::repair`] mends it and the text is parsed again.

use std::collections::BTreeMap;

use saphyr_parser::{Event as ParserEvent, Parser, ScalarStyle, ScanError};

use super::Error;
use super::repair::{normalize_breaks, repair};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    Plain,
    Quoted,
    Literal,
    Folded,
}

#[derive(Debug)]
pub(super) struct Scalar {
    pub(super) value: String,
    pub(super) style: Style,
    /// The resolved tag (`tag:yaml.org,2002:str`, `!local`, …).
    pub(super) tag: Option<String>,
}

#[derive(Debug)]
pub(super) enum Event {
    /// Replays the node starting at this event index.
    Alias(usize),
    Scalar(Scalar),
    SequenceStart(Option<String>),
    SequenceEnd,
    MappingStart(Option<String>),
    MappingEnd,
    /// The empty document.
    Void,
}

/// One parsed document: its events and their 1-based positions.
#[derive(Debug)]
pub(super) struct Document {
    pub(super) events: Vec<(Event, (usize, usize))>,
}

fn full_tag(tag: Option<&saphyr_parser::Tag>) -> Option<String> {
    tag.map(|tag| format!("{}{}", tag.handle, tag.suffix))
}

/// U+2028/U+2029 are line breaks to the YAML 1.1 reader the notes were
/// written for (and to the emitter), but plain content to a YAML 1.2
/// parser.
fn is_unicode_break(c: char) -> bool {
    matches!(c, '\u{2028}' | '\u{2029}')
}

/// In a plain or single-quoted scalar the emitter re-indents after a
/// U+2028/U+2029 (`'a<LS>  b'`) and the old reader folded those blanks
/// away; drop them the same way so Curio's own output reads back as
/// written.
fn fold_unicode_breaks(value: &str) -> String {
    if !value.contains(is_unicode_break) {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len());
    let mut after_break = false;
    for c in value.chars() {
        if is_unicode_break(c) {
            out.truncate(out.trim_end_matches([' ', '\t']).len());
            out.push(c);
            after_break = true;
        } else if !(after_break && matches!(c, ' ' | '\t')) {
            out.push(c);
            after_break = false;
        }
    }
    out
}

/// Where the old reader and this one would read a different document
/// shape: a line opening with U+2028/U+2029, or one inside a block
/// scalar (the old reader ended the line there; the text after it may
/// even be the next key). Refused rather than misread.
fn unicode_break_error() -> Error {
    Error::new(
        "U+2028/U+2029 (line or paragraph separator) at a line start or inside a block \
         scalar is not supported",
    )
}

/// Bytes the repairs of one document may re-parse in all. Each repair
/// re-parses the whole text, so this keeps a pathological note (megabytes
/// of hand-edited quirks) from going quadratic, while ordinary frontmatter
/// (a few KiB) has room for thousands of repairs.
const REPARSE_BUDGET: usize = 1 << 24;

pub(super) fn load(input: &str) -> Result<Document, Error> {
    load_within(input, REPARSE_BUDGET)
}

fn load_within(input: &str, reparse_budget: usize) -> Result<Document, Error> {
    let mut text = normalize_breaks(input);
    if text.starts_with(is_unicode_break)
        || text
            .split('\n')
            .any(|line| line.starts_with(is_unicode_break))
    {
        return Err(unicode_break_error());
    }
    let mut reparsed = 0_usize;
    loop {
        match parse(&text) {
            Err(Failure::Scan(err))
                if reparsed + text.len() <= reparse_budget && repair(&mut text, &err) =>
            {
                reparsed += text.len();
            }
            Err(Failure::Scan(err)) => return Err(Error::new(err.to_string())),
            Err(Failure::Load(err)) => return Err(err),
            Ok(document) => return Ok(document),
        }
    }
}

/// Why one parse failed: the scanner (which [`load`] may repair around)
/// or the document itself.
enum Failure {
    Scan(ScanError),
    Load(Error),
}

impl From<Error> for Failure {
    fn from(err: Error) -> Self {
        Self::Load(err)
    }
}

fn parse(input: &str) -> Result<Document, Failure> {
    let mut events = Vec::new();
    // saphyr anchor id (unique per definition) → index of the anchored event.
    let mut anchors: BTreeMap<usize, usize> = BTreeMap::new();
    let mut documents = 0_usize;
    for next in Parser::new_from_str(input) {
        let (event, span) = next.map_err(Failure::Scan)?;
        let location = (span.start.line(), span.start.col() + 1);
        let (event, anchor) = match event {
            ParserEvent::DocumentStart(_) => {
                documents += 1;
                if documents > 1 {
                    return Err(Error::new("frontmatter holds more than one YAML document").into());
                }
                continue;
            }
            ParserEvent::Nothing
            | ParserEvent::StreamStart
            | ParserEvent::StreamEnd
            | ParserEvent::DocumentEnd => continue,
            ParserEvent::Alias(id) => {
                let target = anchors
                    .get(&id)
                    .copied()
                    .ok_or_else(|| Error::new("unknown alias").at(location))?;
                (Event::Alias(target), 0)
            }
            ParserEvent::Scalar(value, style, id, tag) => {
                let value = match style {
                    ScalarStyle::Plain | ScalarStyle::SingleQuoted => fold_unicode_breaks(&value),
                    ScalarStyle::Literal | ScalarStyle::Folded
                        if value.contains(is_unicode_break) =>
                    {
                        return Err(unicode_break_error().at(location).into());
                    }
                    _ => value.into_owned(),
                };
                let style = match style {
                    ScalarStyle::Plain => Style::Plain,
                    ScalarStyle::SingleQuoted | ScalarStyle::DoubleQuoted => Style::Quoted,
                    ScalarStyle::Literal => Style::Literal,
                    ScalarStyle::Folded => Style::Folded,
                };
                let scalar = Scalar {
                    value,
                    style,
                    tag: full_tag(tag.as_deref()),
                };
                (Event::Scalar(scalar), id)
            }
            ParserEvent::SequenceStart(id, tag) => {
                (Event::SequenceStart(full_tag(tag.as_deref())), id)
            }
            ParserEvent::SequenceEnd => (Event::SequenceEnd, 0),
            ParserEvent::MappingStart(id, tag) => {
                (Event::MappingStart(full_tag(tag.as_deref())), id)
            }
            ParserEvent::MappingEnd => (Event::MappingEnd, 0),
        };
        // Anchor id 0 is "no anchor".
        if anchor != 0 {
            anchors.insert(anchor, events.len());
        }
        events.push((event, location));
    }
    if events.is_empty() {
        events.push((Event::Void, (1, 1)));
    }
    Ok(Document { events })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn repairs_stop_at_the_reparse_budget() {
        let input = "a:\tb\nc:\td\n";
        // Two repairs re-parse the 10-byte text twice.
        assert!(load_within(input, 20).is_ok());
        let err = load_within(input, 19).unwrap_err().to_string();
        assert!(err.contains("valid YAML whitespace"), "{err}");
        assert!(load_within(input, 0).is_err());
    }
}
