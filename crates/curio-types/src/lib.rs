//! Shared contract types and identifiers for Curio.
//!
//! This crate is the home of the two published, versioned contract surfaces
//! (`curio.frontmatter.v1` and `curio.events.v1`) plus the DTOs shared by
//! every head. The authoritative human-readable spec is
//! `docs/design/contracts-draft.md`; the machine-readable JSON Schema
//! artifacts are published under `schemas/`, and this crate's round-trip
//! test suite validates every serialized type against those files — the
//! types are pinned to the published schemas mechanically.
//!
//! Contract rule: schema files are versioned-immutable — a breaking change
//! mints `*.v2`, it never edits v1 semantics.

mod checksum;
mod destination;
mod event;
mod frontmatter;
mod id;
mod manifest;
mod marker;
mod timestamp;

pub use checksum::{Checksum, ParseChecksumError};
pub use destination::{Destination, DestinationName, ParseDestinationNameError};
pub use event::{ArticleSnapshot, EventEnvelope, EventPayload};
pub use frontmatter::ArticleFrontmatter;
pub use id::{CurioId, EventId, ParseCurioIdError, ParseEventIdError};
pub use manifest::{ManifestEntry, ManifestV1};
pub use marker::{EventsSchemaV1Marker, FrontmatterSchemaV1Marker, ManifestSchemaV1Marker};
pub use timestamp::{ParseTimestampError, Timestamp};

/// Schema identifier carried in the YAML frontmatter of every exported note.
pub const FRONTMATTER_SCHEMA_V1: &str = "curio.frontmatter.v1";

/// Schema identifier carried in every event envelope of the append-only
/// behavioral event log (`.curio/events/events-YYYYMMDD.jsonl`).
pub const EVENTS_SCHEMA_V1: &str = "curio.events.v1";

/// Schema identifier of the per-destination export-idempotency manifest
/// (`.curio/manifest.json`).
pub const MANIFEST_SCHEMA_V1: &str = "curio.manifest.v1";

/// Opening marker of the Curio-managed content region inside an exported
/// note. Everything outside the region belongs to the user / other tools and
/// is preserved byte-for-byte on re-export.
pub const MANAGED_REGION_BEGIN_V1: &str = "<!-- curio:managed:begin v1 -->";

/// Closing marker of the Curio-managed content region.
pub const MANAGED_REGION_END_V1: &str = "<!-- curio:managed:end -->";

/// Stable `$id` of the published frontmatter/v1 JSON Schema
/// (placeholder domain until the project-identity decision lands).
pub const FRONTMATTER_SCHEMA_V1_ID: &str = "https://curio.dev/schemas/frontmatter.v1.json";

/// Stable `$id` of the published events/v1 JSON Schema.
pub const EVENTS_SCHEMA_V1_ID: &str = "https://curio.dev/schemas/events.v1.json";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_identifiers_match_the_published_contract() {
        assert_eq!(FRONTMATTER_SCHEMA_V1, "curio.frontmatter.v1");
        assert_eq!(EVENTS_SCHEMA_V1, "curio.events.v1");
        assert_eq!(MANIFEST_SCHEMA_V1, "curio.manifest.v1");
    }

    #[test]
    fn marker_literals_match_the_constants() {
        assert_eq!(FrontmatterSchemaV1Marker::LITERAL, FRONTMATTER_SCHEMA_V1);
        assert_eq!(EventsSchemaV1Marker::LITERAL, EVENTS_SCHEMA_V1);
        assert_eq!(ManifestSchemaV1Marker::LITERAL, MANIFEST_SCHEMA_V1);
    }

    #[test]
    fn managed_region_markers_match_the_published_contract() {
        assert_eq!(MANAGED_REGION_BEGIN_V1, "<!-- curio:managed:begin v1 -->");
        assert_eq!(MANAGED_REGION_END_V1, "<!-- curio:managed:end -->");
    }

    #[test]
    fn schema_ids_are_stable() {
        assert!(FRONTMATTER_SCHEMA_V1_ID.ends_with("/schemas/frontmatter.v1.json"));
        assert!(EVENTS_SCHEMA_V1_ID.ends_with("/schemas/events.v1.json"));
    }
}
