//! Property tests: parse/format round-trips for the contract newtypes.

#![allow(clippy::unwrap_used)]

use curio_types::{Checksum, CurioId, DestinationName, EventId, Timestamp};
use proptest::prelude::*;
use ulid::Ulid;
use uuid::{Builder, Uuid, Variant, Version};

/// Schema pattern for a canonical `UUIDv7`.
const UUID_V7_PATTERN: &str =
    "^[0-9a-f]{8}-[0-9a-f]{4}-7[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$";
/// Schema pattern for a canonical ULID.
const ULID_PATTERN: &str = "^[0-7][0-9A-HJKMNP-TV-Z]{25}$";
/// Schema pattern for a checksum.
const CHECKSUM_PATTERN: &str = "^sha256:[0-9a-f]{64}$";

fn arbitrary_uuid_v7() -> impl Strategy<Value = Uuid> {
    any::<[u8; 16]>().prop_map(|bytes| {
        Builder::from_bytes(bytes)
            .with_version(Version::SortRand)
            .with_variant(Variant::RFC4122)
            .into_uuid()
    })
}

proptest! {
    #[test]
    fn curio_id_display_parse_round_trip(uuid in arbitrary_uuid_v7()) {
        let id = CurioId::from_uuid(uuid).unwrap();
        let s = id.to_string();
        // Canonical form: lowercase hyphenated, version 7, RFC 4122 variant.
        prop_assert_eq!(s.len(), 36);
        prop_assert_eq!(&s, &s.to_lowercase());
        prop_assert_eq!(s.as_bytes()[14], b'7');
        prop_assert!(matches!(s.as_bytes()[19], b'8' | b'9' | b'a' | b'b'));
        let parsed: CurioId = s.parse().unwrap();
        prop_assert_eq!(parsed, id);
    }

    #[test]
    fn curio_id_serde_round_trip(uuid in arbitrary_uuid_v7()) {
        let id = CurioId::from_uuid(uuid).unwrap();
        let json = serde_json::to_string(&id).unwrap();
        let back: CurioId = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, id);
    }

    #[test]
    fn curio_id_rejects_uppercase(uuid in arbitrary_uuid_v7()) {
        let upper = uuid.to_string().to_uppercase();
        prop_assert!(upper.parse::<CurioId>().is_err());
    }

    #[test]
    fn event_id_display_parse_round_trip(raw in any::<u128>()) {
        let id = EventId::from_ulid(Ulid(raw));
        let s = id.to_string();
        prop_assert_eq!(s.len(), 26);
        // First char carries only 3 bits: always 0-7.
        prop_assert!(matches!(s.as_bytes()[0], b'0'..=b'7'));
        // Crockford alphabet, uppercase, no I/L/O/U.
        prop_assert!(s.bytes().all(|b| b.is_ascii_digit()
            || (b.is_ascii_uppercase() && !matches!(b, b'I' | b'L' | b'O' | b'U'))));
        let parsed: EventId = s.parse().unwrap();
        prop_assert_eq!(parsed, id);
    }

    #[test]
    fn event_id_serde_round_trip(raw in any::<u128>()) {
        let id = EventId::from_ulid(Ulid(raw));
        let json = serde_json::to_string(&id).unwrap();
        let back: EventId = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, id);
    }

    #[test]
    fn checksum_display_parse_round_trip(bytes in any::<[u8; 32]>()) {
        let checksum = Checksum::from_bytes(bytes);
        let s = checksum.to_string();
        prop_assert_eq!(s.len(), "sha256:".len() + 64);
        prop_assert!(s.starts_with("sha256:"));
        prop_assert!(s["sha256:".len()..]
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
        let parsed: Checksum = s.parse().unwrap();
        prop_assert_eq!(parsed, checksum);
        prop_assert_eq!(parsed.as_bytes(), &bytes);
    }

    #[test]
    fn checksum_serde_round_trip(bytes in any::<[u8; 32]>()) {
        let checksum = Checksum::from_bytes(bytes);
        let json = serde_json::to_string(&checksum).unwrap();
        let back: Checksum = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, checksum);
    }

    #[test]
    fn checksum_rejects_wrong_prefix(bytes in any::<[u8; 32]>(), prefix in "[a-z0-9]{1,8}") {
        prop_assume!(prefix != "sha256");
        let hex = bytes.iter().fold(String::new(), |mut acc, b| {
            use std::fmt::Write as _;
            let _ = write!(acc, "{b:02x}");
            acc
        });
        let candidate = format!("{prefix}:{hex}");
        prop_assert!(candidate.parse::<Checksum>().is_err());
    }

    #[test]
    fn timestamp_round_trips_at_millisecond_precision(
        // 0 .. year-9999 in millis, keeping Display 4-digit-year canonical.
        millis in 0_i64..=253_402_300_799_999
    ) {
        let ts = Timestamp::from_unix_millis(millis).unwrap();
        let s = ts.to_string();
        // Exactly-millis wire form: ...T..:..:..\.\d{3}Z
        prop_assert_eq!(s.len(), 24);
        prop_assert!(s.ends_with('Z'));
        prop_assert_eq!(s.as_bytes()[19], b'.');
        let parsed: Timestamp = s.parse().unwrap();
        prop_assert_eq!(parsed, ts);
        prop_assert_eq!(parsed.unix_millis(), millis);
    }

    #[test]
    fn timestamp_serde_round_trip(millis in 0_i64..=253_402_300_799_999) {
        let ts = Timestamp::from_unix_millis(millis).unwrap();
        let json = serde_json::to_string(&ts).unwrap();
        let back: Timestamp = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, ts);
    }

    #[test]
    fn destination_name_round_trips(name in "[A-Za-z0-9][A-Za-z0-9 _.-]{0,30}[A-Za-z0-9]") {
        let parsed: DestinationName = name.parse().unwrap();
        prop_assert_eq!(parsed.as_str(), name.as_str());
        let json = serde_json::to_string(&parsed).unwrap();
        let back: DestinationName = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(back, parsed);
    }

    #[test]
    fn destination_name_rejects_separators(prefix in "[a-z]{0,5}", suffix in "[a-z]{0,5}") {
        for sep in ['/', '\\'] {
            let name = format!("{prefix}{sep}{suffix}");
            prop_assert!(name.parse::<DestinationName>().is_err());
        }
    }
}

#[test]
fn schema_patterns_are_pinned() {
    // The property tests above assert the *shape* the schemas' regexes
    // demand; this pins the regex literals themselves so a schema edit
    // that loosens them shows up here.
    let events = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/events.v1.json"),
    )
    .unwrap();
    assert!(events.contains(UUID_V7_PATTERN));
    assert!(events.contains(ULID_PATTERN));
    assert!(events.contains(CHECKSUM_PATTERN));
}
