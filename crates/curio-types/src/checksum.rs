//! The `sha256:`-prefixed content checksum — a change token, never identity.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// A SHA-256 digest with the literal `sha256:` prefix and lowercase hex
/// body (e.g. `sha256:9f86d081…`, 71 characters total).
///
/// Covers exactly the bytes of the managed content region of an exported
/// note — never the whole file. **Change token ONLY**: consumers MUST NOT
/// use the checksum as identity (two articles can have identical content;
/// an edited article keeps its `curio_id` while the checksum changes).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Checksum([u8; 32]);

impl Checksum {
    /// The literal prefix of the wire form.
    pub const PREFIX: &'static str = "sha256:";

    /// Wraps a raw 32-byte SHA-256 digest.
    #[must_use]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// The raw digest bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(Self::PREFIX)?;
        for byte in &self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Checksum {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Checksum({self})")
    }
}

/// Error parsing a [`Checksum`] from its `sha256:`-prefixed wire form.
#[derive(Debug, thiserror::Error)]
pub enum ParseChecksumError {
    /// The `sha256:` prefix is missing.
    #[error("checksum must start with the literal `sha256:` prefix")]
    MissingPrefix,
    /// The hex body is not exactly 64 characters.
    #[error("checksum hex body must be exactly 64 characters, got {0}")]
    BadLength(usize),
    /// The hex body contains a character outside `[0-9a-f]`.
    #[error("checksum hex body must be lowercase hex, got {0:?}")]
    BadHexDigit(char),
}

fn hex_value(byte: u8) -> Result<u8, ParseChecksumError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        other => Err(ParseChecksumError::BadHexDigit(char::from(other))),
    }
}

impl FromStr for Checksum {
    type Err = ParseChecksumError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s
            .strip_prefix(Self::PREFIX)
            .ok_or(ParseChecksumError::MissingPrefix)?;
        if hex.len() != 64 || !hex.is_ascii() {
            return Err(ParseChecksumError::BadLength(hex.chars().count()));
        }
        let mut bytes = [0u8; 32];
        for (i, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            bytes[i] = (hex_value(pair[0])? << 4) | hex_value(pair[1])?;
        }
        Ok(Self(bytes))
    }
}

impl Serialize for Checksum {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Checksum {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const SHA256_EMPTY: &str =
        "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn round_trips_the_wire_form() {
        let checksum: Checksum = SHA256_EMPTY.parse().unwrap();
        assert_eq!(checksum.to_string(), SHA256_EMPTY);
    }

    #[test]
    fn rejects_missing_prefix() {
        let bare = &SHA256_EMPTY[Checksum::PREFIX.len()..];
        assert!(matches!(
            bare.parse::<Checksum>(),
            Err(ParseChecksumError::MissingPrefix)
        ));
        assert!(matches!(
            format!("md5:{bare}").parse::<Checksum>(),
            Err(ParseChecksumError::MissingPrefix)
        ));
    }

    #[test]
    fn rejects_bad_length() {
        assert!(matches!(
            "sha256:abcd".parse::<Checksum>(),
            Err(ParseChecksumError::BadLength(4))
        ));
    }

    #[test]
    fn rejects_uppercase_hex() {
        let upper = SHA256_EMPTY.to_uppercase().replace("SHA256", "sha256");
        assert!(matches!(
            upper.parse::<Checksum>(),
            Err(ParseChecksumError::BadHexDigit(_))
        ));
    }
}
