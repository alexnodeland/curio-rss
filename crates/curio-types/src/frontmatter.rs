//! `curio.frontmatter.v1` — the YAML frontmatter of exported notes.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::checksum::Checksum;
use crate::id::CurioId;
use crate::marker::FrontmatterSchemaV1Marker;
use crate::timestamp::Timestamp;

/// The `curio.frontmatter.v1` frontmatter carried by every markdown note
/// Curio exports into a named destination.
///
/// Published schema: `https://curio.dev/schemas/frontmatter.v1.json`
/// (`schemas/frontmatter.v1.json`).
///
/// `curio_id` is identity (`UUIDv7`, assigned at save, never changes);
/// `checksum` is a change token ONLY and covers exactly the bytes of the
/// managed content region. `feed` and `published` are always present but
/// nullable; `feed_title`, `author`, `lang` and `word_count` are omitted
/// when absent. User-added frontmatter keys unknown to Curio are preserved
/// round-trip in [`extra`](Self::extra).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArticleFrontmatter {
    /// The literal `curio.frontmatter.v1`.
    pub schema: FrontmatterSchemaV1Marker,
    /// Article identity: `UUIDv7`, assigned at save, never changes.
    pub curio_id: CurioId,
    /// Article title.
    pub title: String,
    /// Canonical article URL.
    pub source: String,
    /// Feed URL the article came from; `None` (serialized as null) if the
    /// article was saved manually.
    pub feed: Option<String>,
    /// Human-readable feed name. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feed_title: Option<String>,
    /// Article author. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Publication time; `None` (serialized as null) if unknown.
    pub published: Option<Timestamp>,
    /// Time the article was saved in Curio.
    pub saved: Timestamp,
    /// Curio-assigned tags at export time.
    pub tags: Vec<String>,
    /// sha256 of the managed content region bytes. Change token ONLY —
    /// never identity.
    pub checksum: Checksum,
    /// BCP-47 language tag. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    /// Word count of the extracted article. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub word_count: Option<u64>,
    /// User-added frontmatter keys unknown to Curio — preserved
    /// byte-for-byte on re-export, never interpreted.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}
