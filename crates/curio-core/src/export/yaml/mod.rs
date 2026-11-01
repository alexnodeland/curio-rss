//! The YAML dialect of note frontmatter: a safe-Rust serde serializer and
//! deserializer that reproduce, byte for byte, what the notes in users'
//! vaults were written with (`serde_yaml` 0.9 over libyaml, issue #180).
//!
//! The `curio.frontmatter.v1` bytes and the managed-region checksum make
//! every quoting, ordering or indentation change visible in a user's
//! vault, so the output is not merely "valid YAML": it is the exact
//! libyaml block style — `- ` items flush under their key, two-space
//! nesting, `[]`/`{}` for empty collections, `|` literals for multi-line
//! strings, single quotes only where a plain scalar would read back as
//! something else, double quotes with escapes for control characters, and
//! `? ` explicit keys past 128 bytes. The goldens under
//! `fixtures/export/frontmatter/` pin it.
//!
//! Parsing runs on [`saphyr_parser`] (YAML 1.2, safe Rust) and reads
//! hand-edited frontmatter the way the old reader did: comments, flow
//! style, anchors and aliases, block scalars, `!!str`/`!!int`-style core
//! tags, and the libyaml leniencies YAML 1.2 dropped, by mending the text
//! where the parser refuses it: a TAB after `key:`, lone CR and NEL line
//! breaks, continuation lines of quoted scalars and flow collections at or
//! left of their key, a comment with no blank before it. Plain scalars
//! resolve by the YAML 1.2 core schema (`yes`/`on` stay strings).
//!
//! Known differences from the old reader:
//!
//! - U+2028/U+2029 at a line start or inside a block scalar is refused
//!   rather than misread — the one difference Curio's own output can hit
//!   (a multi-line string that also carries one, #249);
//! - a block scalar header alone at column 0 on the line after its key
//!   (`key:\n|\n  text`) is refused;
//! - a lone `-` as a flow collection entry (`[-]`) is refused;
//! - a few documents libyaml refused now parse (YAML 1.2 is laxer about,
//!   e.g., a TAB before a top-level plain scalar).

mod de;
mod emit;
mod load;
mod repair;
mod resolve;
mod ser;

use std::fmt;

use serde::Serialize;
use serde::de::DeserializeOwned;

/// A frontmatter YAML (de)serialization failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
    /// 1-based line and column of the offending node, when known.
    location: Option<(usize, usize)>,
}

impl Error {
    pub(super) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            location: None,
        }
    }

    /// Attaches a location if the error has none yet.
    fn at(mut self, location: (usize, usize)) -> Self {
        self.location.get_or_insert(location);
        self
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if let Some((line, column)) = self.location {
            write!(f, " at line {line} column {column}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {}

impl serde::ser::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::new(msg.to_string())
    }
}

impl serde::de::Error for Error {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::new(msg.to_string())
    }
}

/// Serializes `value` as a block-style YAML document (no `---`), ending
/// in a newline.
///
/// # Errors
///
/// [`Error`] for values YAML frontmatter cannot carry: byte strings and
/// enum variants with data.
pub fn to_string<T: ?Sized + Serialize>(value: &T) -> Result<String, Error> {
    let node = value.serialize(ser::NodeSerializer)?;
    Ok(emit::emit(&node))
}

/// Deserializes one YAML document.
///
/// # Errors
///
/// [`Error`] for malformed YAML, more than one document, an unknown
/// alias, custom (`!local`) tags, or a document that does not match `T`.
pub fn from_str<T: DeserializeOwned>(input: &str) -> Result<T, Error> {
    de::from_str(input)
}
