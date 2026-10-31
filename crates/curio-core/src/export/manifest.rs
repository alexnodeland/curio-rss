//! `.curio/manifest.json` — the per-destination export-idempotency
//! oracle. Canonical form (sorted keys, 2-space indent, one entry per
//! line) comes from [`ManifestV1::to_canonical_json`]; writes go through
//! temp-file + atomic rename, and always *after* the note they record.
//!
//! Destinations often live in a synced folder, and sync tools resolve a
//! concurrent edit by parking the losing side next to the original —
//! `manifest (conflicted copy).json` (Dropbox, Nextcloud),
//! `manifest 2.json` (iCloud), `manifest.sync-conflict-….json`
//! (Syncthing), `manifest-HOSTNAME.json` (Microsoft `OneDrive`). Those entries are
//! real exports; [`manifest_conflict_copies`] finds such files and the
//! exporter folds them back in (union, newest `exported_at` wins) instead
//! of silently losing them. Only those sync-tool name shapes count: a
//! `manifest.backup.json` of the user's own is never touched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use curio_types::{CurioId, ManifestV1};

use super::lock::LOCK_FILE;
use super::{ExportError, contained_path, note, write_atomic};

const MANIFEST_FILE: &str = "manifest.json";

/// Path of the manifest inside a destination root.
#[must_use]
pub fn manifest_path(root: &Path) -> PathBuf {
    root.join(".curio").join(MANIFEST_FILE)
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
    parse_manifest(&raw, path)
}

fn parse_manifest(raw: &str, path: PathBuf) -> Result<ManifestV1, ExportError> {
    serde_json::from_str(raw).map_err(|err| ExportError::ManifestParse {
        path,
        message: err.to_string(),
    })
}

/// Whether a `.curio/` file name is a sync tool's conflict copy of
/// `manifest.json`. Only the shapes sync tools actually produce count —
/// anything else named `manifest….json` (a backup the user made) is
/// theirs, not a copy to merge and delete:
///
/// - `manifest (…).json` / `manifest(…).json` — Dropbox, Nextcloud,
///   Seafile, Google Drive, Windows' own `(1)` suffix;
/// - `manifest <n>.json` — iCloud;
/// - `manifest.sync-conflict-….json` — Syncthing;
/// - `manifest-HOSTNAME.json` (upper-case host name) — Microsoft `OneDrive`.
fn is_conflict_copy(name: &str) -> bool {
    let path = Path::new(name);
    let is_json = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
    let Some(rest) = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .and_then(|stem| stem.strip_prefix("manifest"))
    else {
        return false;
    };
    let parenthesized = rest.trim_start().starts_with('(') && rest.ends_with(')');
    let numbered = rest
        .strip_prefix(' ')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()));
    let syncthing = rest.starts_with(".sync-conflict-");
    let host_suffixed = rest.strip_prefix('-').is_some_and(|host| {
        !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
    });
    is_json && (parenthesized || numbered || syncthing || host_suffixed)
}

/// Sync-tool conflict copies of the manifest sitting in the destination's
/// `.curio/` (e.g. `manifest (conflicted copy).json`, iCloud's
/// `manifest 2.json`), sorted by name. Empty when there are none or the
/// destination has no `.curio/` yet.
///
/// # Errors
///
/// [`ExportError::Io`] when `.curio/` exists but cannot be listed.
pub fn manifest_conflict_copies(root: &Path) -> Result<Vec<PathBuf>, ExportError> {
    let dir = root.join(".curio");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(source) => return Err(ExportError::Io { path: dir, source }),
    };
    let mut copies = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|source| ExportError::Io {
            path: dir.clone(),
            source,
        })?;
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if is_file && entry.file_name().to_str().is_some_and(is_conflict_copy) {
            copies.push(entry.path());
        }
    }
    copies.sort();
    Ok(copies)
}

/// Folds every parseable conflict copy into `manifest`: the union of the
/// entries, and for a `curio_id` present on both sides the record with
/// the newer `exported_at`. Returns the copies that were merged — the
/// caller deletes them once the merged manifest is durably written. A
/// copy that does not load is left in place and reported in the log; it
/// is never guessed at.
///
/// A union can claim one path for two articles (two devices each
/// exported a different same-titled article to `curio/weekly-links.md`).
/// The manifest tells consumers who owns a note, so such a claim is
/// settled by the note itself — see [`settle_contested_paths`].
///
/// # Errors
///
/// [`ExportError::Io`] when `.curio/` cannot be listed.
pub(super) fn merge_conflict_copies(
    root: &Path,
    manifest: &mut ManifestV1,
) -> Result<Vec<PathBuf>, ExportError> {
    let mut merged = Vec::new();
    for path in manifest_conflict_copies(root)? {
        let loaded = std::fs::read_to_string(&path)
            .map_err(|source| ExportError::Io {
                path: path.clone(),
                source,
            })
            .and_then(|raw| parse_manifest(&raw, path.clone()));
        let copy = match loaded {
            Ok(copy) => copy,
            Err(err) => {
                tracing::warn!(%err, "manifest conflict copy left in place: it does not load");
                continue;
            }
        };
        for (id, entry) in copy.notes {
            match manifest.notes.get(&id) {
                Some(current) if current.exported_at >= entry.exported_at => {}
                _ => {
                    manifest.notes.insert(id, entry);
                }
            }
        }
        tracing::info!(path = %path.display(), "merged a manifest conflict copy");
        merged.push(path);
    }
    if !merged.is_empty() {
        settle_contested_paths(root, manifest);
    }
    Ok(merged)
}

/// Keeps at most one manifest entry per note path: the one whose
/// `curio_id` the note's own frontmatter names. Every other claimant is
/// dropped (and logged) — the article is then simply unrecorded, and its
/// next export finds it a path of its own (or its own note) through the
/// normal ownership checks. When the note is gone or names none of the
/// claimants, all of them are dropped.
fn settle_contested_paths(root: &Path, manifest: &mut ManifestV1) {
    let mut claimants: BTreeMap<&str, Vec<CurioId>> = BTreeMap::new();
    for (id, entry) in &manifest.notes {
        claimants.entry(entry.path.as_str()).or_default().push(*id);
    }
    let contested: Vec<(String, Vec<CurioId>)> = claimants
        .into_iter()
        .filter(|(_, ids)| ids.len() > 1)
        .map(|(path, ids)| (path.to_owned(), ids))
        .collect();
    for (path, ids) in contested {
        let owner = contained_path(root, &path).ok().and_then(|abs| {
            std::fs::read_to_string(&abs)
                .ok()
                .and_then(|content| note::note_owner(&content, &abs).ok())
        });
        for id in ids {
            if Some(id) != owner {
                manifest.notes.remove(&id);
                tracing::warn!(%path, curio_id = %id, "dropped a merged manifest entry: another article owns that note path");
            }
        }
    }
}

/// Writes the manifest in canonical form via temp-file + atomic rename.
///
/// Also maintains `.curio/.gitignore`, ignoring `events/` and the export
/// lock file: the contract says the event log is never committed to git
/// *in any destination* either, and the intended destination is often a
/// git repository (an Obsidian vault) — the manifest itself stays
/// committable (canonical form exists for git-mergeable diffs).
///
/// # Errors
///
/// [`ExportError::Io`] on write failures.
pub fn write_manifest(root: &Path, manifest: &ManifestV1) -> Result<(), ExportError> {
    let path = manifest_path(root);
    write_atomic(root, &path, manifest.to_canonical_json().as_bytes())?;
    ensure_gitignore(root)
}

/// Appends whichever of the Curio-owned ignore lines are missing, so a
/// destination scaffolded by an older build picks up the lock file too.
fn ensure_gitignore(root: &Path) -> Result<(), ExportError> {
    let gitignore = root.join(".curio").join(".gitignore");
    let io_err = |source| ExportError::Io {
        path: gitignore.clone(),
        source,
    };
    let current = match std::fs::read_to_string(&gitignore) {
        Ok(current) => current,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(source) => return Err(io_err(source)),
    };
    let mut next = current.clone();
    for line in ["events/", LOCK_FILE] {
        if !current.lines().any(|l| l.trim() == line) {
            if !next.is_empty() && !next.ends_with('\n') {
                next.push('\n');
            }
            next.push_str(line);
            next.push('\n');
        }
    }
    if next != current {
        std::fs::write(&gitignore, next).map_err(io_err)?;
    }
    Ok(())
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
    fn gitignore_covers_events_and_the_lock_file_and_upgrades_old_ones() {
        let dir = tempfile::tempdir().unwrap();
        write_manifest(dir.path(), &ManifestV1::new()).unwrap();
        let gitignore = dir.path().join(".curio/.gitignore");
        assert_eq!(
            std::fs::read_to_string(&gitignore).unwrap(),
            "events/\nmanifest.lock\n"
        );
        // Unchanged content is not rewritten.
        write_manifest(dir.path(), &ManifestV1::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&gitignore).unwrap(),
            "events/\nmanifest.lock\n"
        );
        // A destination scaffolded by an older build only ignores events/
        // (and the user may have added an unterminated line of their own).
        std::fs::write(&gitignore, "events/\nmine").unwrap();
        write_manifest(dir.path(), &ManifestV1::new()).unwrap();
        assert_eq!(
            std::fs::read_to_string(&gitignore).unwrap(),
            "events/\nmine\nmanifest.lock\n"
        );
    }

    #[test]
    fn conflict_copy_names_are_recognized() {
        for name in [
            "manifest (conflicted copy).json",
            "manifest (Alex's conflicted copy 2026-10-31).json",
            "manifest 2.json",
            "manifest(1).json",
            "manifest.sync-conflict-20261031-120000-ABCDEFG.json",
            "manifest-DESKTOP-1A2B3C.json",
            "manifest 3.JSON",
        ] {
            assert!(is_conflict_copy(name), "{name}");
        }
        for name in [
            "manifest.json",
            ".manifest.json.curio-tmp-1-2",
            "manifest.lock",
            "manifest.json.bak",
            "manifest.backup.json",
            "manifest-backup.json",
            "manifest_old.json",
            "manifest .json",
            "manifest 2b.json",
            "manifest (1).txt",
            "events",
            ".gitignore",
        ] {
            assert!(!is_conflict_copy(name), "{name}");
        }
    }

    #[test]
    fn merge_unions_entries_newest_wins_and_skips_unloadable_copies() {
        let dir = tempfile::tempdir().unwrap();
        let curio = dir.path().join(".curio");
        std::fs::create_dir_all(&curio).unwrap();
        let shared = "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d".parse().unwrap();
        let only_copy = "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6e".parse().unwrap();
        let entry = |path: &str, at: &str| ManifestEntry {
            path: path.to_owned(),
            checksum: region_checksum(path),
            exported_at: at.parse().unwrap(),
        };
        let mut ours = ManifestV1::new();
        ours.notes
            .insert(shared, entry("curio/old.md", "2026-07-01T00:00:00.000Z"));
        let mut theirs = ManifestV1::new();
        theirs
            .notes
            .insert(shared, entry("curio/new.md", "2026-07-02T00:00:00.000Z"));
        theirs
            .notes
            .insert(only_copy, entry("curio/b.md", "2026-07-01T00:00:00.000Z"));
        std::fs::write(curio.join("manifest 2.json"), theirs.to_canonical_json()).unwrap();
        std::fs::write(curio.join("manifest (broken).json"), "{ nope").unwrap();

        let merged = merge_conflict_copies(dir.path(), &mut ours).unwrap();
        assert_eq!(merged, vec![curio.join("manifest 2.json")]);
        assert_eq!(ours.notes[&shared].path, "curio/new.md", "newest wins");
        assert_eq!(ours.notes[&only_copy].path, "curio/b.md", "union");

        // An older copy never overrides a newer record.
        let mut stale = ManifestV1::new();
        stale
            .notes
            .insert(shared, entry("curio/stale.md", "2026-06-01T00:00:00.000Z"));
        std::fs::write(curio.join("manifest 2.json"), stale.to_canonical_json()).unwrap();
        merge_conflict_copies(dir.path(), &mut ours).unwrap();
        assert_eq!(ours.notes[&shared].path, "curio/new.md");
    }

    #[test]
    fn no_curio_dir_means_no_conflict_copies() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            manifest_conflict_copies(dir.path()).unwrap(),
            Vec::<PathBuf>::new()
        );
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
