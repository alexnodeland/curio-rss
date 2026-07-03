//! Contract identifiers: `CurioId` (`UUIDv7` article identity) and `EventId`
//! (ULID event identity).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use ulid::Ulid;
use uuid::Uuid;

/// Article identity: a `UUIDv7`, assigned at save, never changes.
///
/// `curio_id` is *identity*; `checksum` is a change token only — an edited
/// article keeps its id, and two articles can share identical content.
/// The canonical wire form is lowercase hyphenated
/// (e.g. `0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d`); parsing is strict about
/// canonical form and rejects any UUID whose version is not 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CurioId(Uuid);

impl CurioId {
    /// Mints a fresh `UUIDv7` id from the current time.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Wraps an existing UUID, verifying it is version 7.
    ///
    /// # Errors
    ///
    /// Returns [`ParseCurioIdError::NotVersion7`] if the UUID's version
    /// number is not 7.
    pub fn from_uuid(uuid: Uuid) -> Result<Self, ParseCurioIdError> {
        if uuid.get_version_num() == 7 {
            Ok(Self(uuid))
        } else {
            Err(ParseCurioIdError::NotVersion7 {
                version: uuid.get_version_num(),
            })
        }
    }

    /// The underlying UUID.
    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for CurioId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CurioId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Uuid's Display is the canonical lowercase hyphenated form.
        self.0.fmt(f)
    }
}

/// Error parsing a [`CurioId`].
#[derive(Debug, thiserror::Error)]
pub enum ParseCurioIdError {
    /// The string is not a UUID at all.
    #[error("invalid UUID: {0}")]
    NotAUuid(#[from] uuid::Error),
    /// The UUID parsed but is not version 7.
    #[error("curio_id must be a `UUIDv7`, got version {version}")]
    NotVersion7 {
        /// The version number that was found.
        version: usize,
    },
    /// The UUID parsed but was not in canonical lowercase hyphenated form.
    #[error("curio_id must be in canonical lowercase hyphenated form")]
    NonCanonical,
}

impl FromStr for CurioId {
    type Err = ParseCurioIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(s)?;
        // Uuid::parse_str accepts uppercase, braced, simple and urn forms;
        // the contract's wire form is canonical lowercase hyphenated only.
        if uuid.to_string() != s {
            return Err(ParseCurioIdError::NonCanonical);
        }
        Self::from_uuid(uuid)
    }
}

impl Serialize for CurioId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for CurioId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

/// Event identity: a ULID, unique per event.
///
/// Consumers dedupe (and gain replay-idempotency) by `event_id`. The
/// canonical wire form is 26 characters of uppercase Crockford base32
/// (e.g. `01J1PZ2M3N4P5Q6R7S8T9V0W1X`); parsing is strict about canonical
/// form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(Ulid);

impl EventId {
    /// Mints a fresh ULID from the current time.
    #[must_use]
    pub fn new() -> Self {
        Self(Ulid::new())
    }

    /// Wraps an existing ULID.
    #[must_use]
    pub fn from_ulid(ulid: Ulid) -> Self {
        Self(ulid)
    }

    /// The underlying ULID.
    #[must_use]
    pub fn as_ulid(&self) -> Ulid {
        self.0
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Ulid's Display is the canonical 26-char uppercase Crockford form.
        self.0.fmt(f)
    }
}

/// Error parsing an [`EventId`].
#[derive(Debug, thiserror::Error)]
pub enum ParseEventIdError {
    /// The string is not a ULID at all.
    #[error("invalid ULID: {0}")]
    NotAUlid(#[from] ulid::DecodeError),
    /// The ULID parsed but was not in canonical uppercase form.
    #[error("event_id must be in canonical uppercase Crockford base32 form")]
    NonCanonical,
}

impl FromStr for EventId {
    type Err = ParseEventIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let ulid = Ulid::from_string(s)?;
        // Ulid::from_string decodes case-insensitively; the contract's wire
        // form is canonical uppercase only.
        if ulid.to_string() != s {
            return Err(ParseEventIdError::NonCanonical);
        }
        Ok(Self(ulid))
    }
}

impl Serialize for EventId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for EventId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn curio_id_new_is_version_7_and_canonical() {
        let id = CurioId::new();
        assert_eq!(id.as_uuid().get_version_num(), 7);
        let s = id.to_string();
        assert_eq!(s, s.to_lowercase());
        assert_eq!(s.parse::<CurioId>().unwrap(), id);
    }

    #[test]
    fn curio_id_rejects_non_v7() {
        let v4 = "9f86d081-884c-4d63-a1b2-3e9f10aa4b6d"; // version nibble 4
        assert!(matches!(
            v4.parse::<CurioId>(),
            Err(ParseCurioIdError::NotVersion7 { version: 4 })
        ));
    }

    #[test]
    fn curio_id_rejects_non_canonical_forms() {
        assert!(matches!(
            "0197B2C4-8F3E-7CC1-A5D2-3E9F10AA4B6D".parse::<CurioId>(),
            Err(ParseCurioIdError::NonCanonical)
        ));
        assert!(matches!(
            "0197b2c48f3e7cc1a5d23e9f10aa4b6d".parse::<CurioId>(),
            Err(ParseCurioIdError::NonCanonical)
        ));
        assert!("not-a-uuid".parse::<CurioId>().is_err());
    }

    #[test]
    fn event_id_round_trips_canonically() {
        let id: EventId = "01J1PZ2M3N4P5Q6R7S8T9V0W1X".parse().unwrap();
        assert_eq!(id.to_string(), "01J1PZ2M3N4P5Q6R7S8T9V0W1X");
    }

    #[test]
    fn event_id_rejects_non_canonical_forms() {
        assert!(matches!(
            "01j1pz2m3n4p5q6r7s8t9v0w1x".parse::<EventId>(),
            Err(ParseEventIdError::NonCanonical)
        ));
        assert!("too-short".parse::<EventId>().is_err());
        // Overflow: first char > 7 exceeds 128 bits.
        assert!("81J1PZ2M3N4P5Q6R7S8T9V0W1X".parse::<EventId>().is_err());
    }
}
