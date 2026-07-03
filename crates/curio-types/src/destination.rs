//! Named export destinations.
//!
//! Promotion is always "promote article X to destination NAME" — raw
//! filesystem paths never cross IPC or CLI flags. The name-to-directory
//! mapping lives in local configuration ([`Destination`]).

use std::fmt;
use std::path::PathBuf;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// The name of a configured export destination.
///
/// Names are opaque labels, never paths: they must be non-empty, contain no
/// path separators or control characters, must not be `.` or `..`, and must
/// not carry leading/trailing whitespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DestinationName(String);

impl DestinationName {
    /// The name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DestinationName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Error validating a [`DestinationName`].
#[derive(Debug, thiserror::Error)]
pub enum ParseDestinationNameError {
    /// The name is empty.
    #[error("destination name must not be empty")]
    Empty,
    /// The name contains a path separator or control character.
    #[error("destination name must not contain {0:?} — names are labels, never paths")]
    ForbiddenCharacter(char),
    /// The name is `.` or `..`.
    #[error("destination name must not be a dot segment")]
    DotSegment,
    /// The name has leading or trailing whitespace.
    #[error("destination name must not have leading or trailing whitespace")]
    SurroundingWhitespace,
}

impl FromStr for DestinationName {
    type Err = ParseDestinationNameError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ParseDestinationNameError::Empty);
        }
        if s == "." || s == ".." {
            return Err(ParseDestinationNameError::DotSegment);
        }
        if s.trim() != s {
            return Err(ParseDestinationNameError::SurroundingWhitespace);
        }
        if let Some(c) = s.chars().find(|&c| c == '/' || c == '\\' || c.is_control()) {
            return Err(ParseDestinationNameError::ForbiddenCharacter(c));
        }
        Ok(Self(s.to_owned()))
    }
}

impl TryFrom<String> for DestinationName {
    type Error = ParseDestinationNameError;

    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

impl Serialize for DestinationName {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for DestinationName {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

/// A configured export destination: a named directory of Markdown + YAML
/// (generic — never Obsidian-branded).
///
/// Local configuration only; the `root` path never appears in published
/// contract payloads, which carry the [`DestinationName`] instead.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Destination {
    /// The destination's name — the only way the destination is referenced
    /// across IPC, CLI flags, and published events.
    pub name: DestinationName,
    /// Directory the destination writes into. Every write canonicalizes
    /// and asserts containment under this root.
    pub root: PathBuf,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn accepts_reasonable_names() {
        for name in ["vault", "notes-inbox", "Obsidian Vault", "kb_2026"] {
            assert_eq!(name.parse::<DestinationName>().unwrap().as_str(), name);
        }
    }

    #[test]
    fn rejects_paths_and_dot_segments() {
        assert!(matches!(
            "a/b".parse::<DestinationName>(),
            Err(ParseDestinationNameError::ForbiddenCharacter('/'))
        ));
        assert!(matches!(
            "a\\b".parse::<DestinationName>(),
            Err(ParseDestinationNameError::ForbiddenCharacter('\\'))
        ));
        assert!(matches!(
            ".".parse::<DestinationName>(),
            Err(ParseDestinationNameError::DotSegment)
        ));
        assert!(matches!(
            "..".parse::<DestinationName>(),
            Err(ParseDestinationNameError::DotSegment)
        ));
    }

    #[test]
    fn rejects_empty_whitespace_and_control() {
        assert!(matches!(
            "".parse::<DestinationName>(),
            Err(ParseDestinationNameError::Empty)
        ));
        assert!(matches!(
            " vault".parse::<DestinationName>(),
            Err(ParseDestinationNameError::SurroundingWhitespace)
        ));
        assert!(matches!(
            "vault\n".parse::<DestinationName>(),
            Err(ParseDestinationNameError::SurroundingWhitespace)
        ));
        assert!(matches!(
            "va\0ult".parse::<DestinationName>(),
            Err(ParseDestinationNameError::ForbiddenCharacter('\0'))
        ));
    }
}
