//! `curio.manifest.v1` — the per-destination export-idempotency oracle
//! (`.curio/manifest.json`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::checksum::Checksum;
use crate::id::CurioId;
use crate::marker::ManifestSchemaV1Marker;
use crate::timestamp::Timestamp;

/// The `curio.manifest.v1` document at `.curio/manifest.json` in every
/// destination, mapping `curio_id` → export record.
///
/// Published schema: `$defs/manifest` of
/// `https://curio.dev/schemas/frontmatter.v1.json`.
///
/// Contract rules the writer upholds:
///
/// 1. keys sorted, 2-space indent, one entry per line → git-mergeable diffs
///    ([`to_canonical_json`](Self::to_canonical_json) produces exactly this
///    form; the [`BTreeMap`] keeps keys sorted by construction);
/// 2. write ordering: note file first, manifest second, via temp-file +
///    atomic rename — a crash leaves an orphan note, never a dangling
///    manifest entry;
/// 3. export is idempotent on `(curio_id, checksum)`: unchanged → no write;
/// 4. consumers read the manifest to learn write-ownership — paths present
///    here are Curio-owned at the managed-region level.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestV1 {
    /// The literal `curio.manifest.v1`.
    pub schema: ManifestSchemaV1Marker,
    /// Map from `curio_id` to export record. Sorted by construction.
    pub notes: BTreeMap<CurioId, ManifestEntry>,
}

impl ManifestV1 {
    /// An empty manifest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Renders the canonical on-disk form: keys sorted, 2-space indent, one
    /// note entry per line, trailing newline — the git-mergeable format the
    /// contract requires.
    #[must_use]
    pub fn to_canonical_json(&self) -> String {
        fn json_str(s: &str) -> String {
            // Display on serde_json::Value is infallible.
            serde_json::Value::String(s.to_owned()).to_string()
        }

        let mut out = String::new();
        out.push_str("{\n  \"schema\": \"curio.manifest.v1\",\n  \"notes\": {");
        for (i, (curio_id, entry)) in self.notes.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("\n    \"");
            out.push_str(&curio_id.to_string());
            out.push_str("\": { \"path\": ");
            out.push_str(&json_str(&entry.path));
            out.push_str(", \"checksum\": ");
            out.push_str(&json_str(&entry.checksum.to_string()));
            out.push_str(", \"exported_at\": ");
            out.push_str(&json_str(&entry.exported_at.to_string()));
            out.push_str(" }");
        }
        if self.notes.is_empty() {
            out.push_str("}\n}\n");
        } else {
            out.push_str("\n  }\n}\n");
        }
        out
    }
}

/// One export record inside [`ManifestV1`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Note path relative to the destination root.
    pub path: String,
    /// Checksum of the managed region as last exported. Change token ONLY.
    pub checksum: Checksum,
    /// Time of the last export of this note.
    pub exported_at: Timestamp,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn sample() -> ManifestV1 {
        let mut manifest = ManifestV1::new();
        manifest.notes.insert(
            "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap(),
            ManifestEntry {
                path: "curio/article-title.md".to_owned(),
                checksum: "sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08"
                    .parse()
                    .unwrap(),
                exported_at: "2026-07-03T09:15:00.123Z".parse().unwrap(),
            },
        );
        manifest
    }

    #[test]
    fn canonical_json_is_one_entry_per_line_with_two_space_indent() {
        let rendered = sample().to_canonical_json();
        let expected = concat!(
            "{\n",
            "  \"schema\": \"curio.manifest.v1\",\n",
            "  \"notes\": {\n",
            "    \"0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d\": ",
            "{ \"path\": \"curio/article-title.md\", ",
            "\"checksum\": \"sha256:9f86d081884c7d659a2feaa0c55ad015",
            "a3bf4f1b2b0b822cd15d6c15b0f00a08\", ",
            "\"exported_at\": \"2026-07-03T09:15:00.123Z\" }\n",
            "  }\n",
            "}\n",
        );
        assert_eq!(rendered, expected);
    }

    #[test]
    fn canonical_json_parses_back_to_the_same_manifest() {
        let manifest = sample();
        let reparsed: ManifestV1 = serde_json::from_str(&manifest.to_canonical_json()).unwrap();
        assert_eq!(reparsed, manifest);
    }

    #[test]
    fn empty_manifest_renders_and_parses() {
        let manifest = ManifestV1::new();
        let rendered = manifest.to_canonical_json();
        let reparsed: ManifestV1 = serde_json::from_str(&rendered).unwrap();
        assert_eq!(reparsed, manifest);
    }

    #[test]
    fn canonical_keys_are_sorted() {
        let mut manifest = sample();
        manifest.notes.insert(
            "0097b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap(),
            ManifestEntry {
                path: "curio/earlier.md".to_owned(),
                checksum: "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                    .parse()
                    .unwrap(),
                exported_at: "2026-07-01T00:00:00.000Z".parse().unwrap(),
            },
        );
        let rendered = manifest.to_canonical_json();
        let first = rendered.find("0097b2c4").unwrap();
        let second = rendered.find("0197b2c4").unwrap();
        assert!(first < second);
    }
}
