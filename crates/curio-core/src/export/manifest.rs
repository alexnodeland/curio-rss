//! `.curio/manifest.json` — the per-destination export-idempotency
//! oracle. Canonical form (sorted keys, 2-space indent, one entry per
//! line) comes from [`ManifestV1::to_canonical_json`]; writes go through
//! temp-file + atomic rename, and always *after* the note they record.

use std::path::{Path, PathBuf};

use curio_types::ManifestV1;

use super::{ExportError, write_atomic};

/// Path of the manifest inside a destination root.
#[must_use]
pub fn manifest_path(root: &Path) -> PathBuf {
    root.join(".curio").join("manifest.json")
}

/// Loads the destination's manifest; a missing file is an empty
/// manifest (fresh destination, or wipe-and-reinstall reconciliation).
///
/// # Errors
///
/// [`ExportError::Io`] on read failures other than not-found;
/// [`ExportError::ManifestParse`] when the file exists but does not
/// parse — refusing to guess and clobber the oracle.
pub fn load_manifest(root: &Path) -> Result<ManifestV1, ExportError> {
    let path = manifest_path(root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ManifestV1::new());
        }
        Err(source) => return Err(ExportError::Io { path, source }),
    };
    serde_json::from_str(&raw).map_err(|err| ExportError::ManifestParse {
        path,
        message: err.to_string(),
    })
}

/// Writes the manifest in canonical form via temp-file + atomic rename.
///
/// # Errors
///
/// [`ExportError::Io`] on write failures.
pub fn write_manifest(root: &Path, manifest: &ManifestV1) -> Result<(), ExportError> {
    let path = manifest_path(root);
    write_atomic(root, &path, manifest.to_canonical_json().as_bytes())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use curio_types::{ManifestEntry, Timestamp};

    use super::super::region_checksum;
    use super::*;

    #[test]
    fn missing_manifest_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = load_manifest(dir.path()).unwrap();
        assert!(manifest.notes.is_empty());
    }

    #[test]
    fn manifest_round_trips_canonically() {
        let dir = tempfile::tempdir().unwrap();
        let mut manifest = ManifestV1::new();
        manifest.notes.insert(
            "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap(),
            ManifestEntry {
                path: "curio/a.md".to_owned(),
                checksum: region_checksum("a"),
                exported_at: Timestamp::now(),
            },
        );
        write_manifest(dir.path(), &manifest).unwrap();
        let raw = std::fs::read_to_string(manifest_path(dir.path())).unwrap();
        assert_eq!(raw, manifest.to_canonical_json());
        assert_eq!(load_manifest(dir.path()).unwrap(), manifest);
    }

    #[test]
    fn corrupt_manifest_is_an_error_not_a_reset() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".curio")).unwrap();
        std::fs::write(manifest_path(dir.path()), "{ not json").unwrap();
        assert!(matches!(
            load_manifest(dir.path()),
            Err(ExportError::ManifestParse { .. })
        ));
    }
}
