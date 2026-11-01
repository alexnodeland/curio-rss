//! `curio.frontmatter.v1` note export into named destinations.
//!
//! The strategic seam: a destination is any configured directory of
//! Markdown + YAML (generic, never Obsidian-branded), referenced by
//! **name only** — raw filesystem paths never cross IPC or CLI flags.
//!
//! Contract obligations this module upholds (contracts-draft.md):
//!
//! - **Identity vs change token**: `curio_id` is identity; `checksum`
//!   (sha256 over exactly the managed-region bytes) is a change token
//!   ONLY.
//! - **Managed region**: Curio owns the frontmatter machine keys plus
//!   the one marked region. Re-export replaces those and preserves
//!   everything else byte-for-byte; unknown frontmatter keys survive.
//! - **Manifest** (`.curio/manifest.json`): sorted keys, 2-space
//!   indent, one entry per line; **note first, manifest second**, both
//!   via temp-file + atomic rename — a crash leaves an orphan note,
//!   never a dangling manifest entry.
//! - **Idempotency** on `(curio_id, checksum)`: unchanged → no write.
//!
//! Safety beyond the contract text, because the manifest is a file in
//! the user's folder and can be lost, reset, stale, or forked by a sync
//! tool:
//!
//! - **Ownership**: an existing file is only rewritten when its own
//!   frontmatter `curio_id` is the article being exported. A path held by
//!   another article (or by a file that is not a Curio note) is skipped
//!   for the next deterministic candidate — never overwritten.
//! - **Serialization**: one export per destination at a time, across
//!   threads and processes (an in-process queue plus an advisory lock on
//!   `.curio/manifest.lock`); temp files are unique per write.
//! - **No resurrection**: a missing destination root is
//!   [`ExportError::DestinationMissing`], never recreated.
//! - **Conflict copies** of the manifest are merged back in (see
//!   [`manifest_conflict_copies`]).
//!
//! Event emission (`article.saved` / `article.updated`) is wired by the
//! [`crate::CoreHandle`] facade, which owns the storage + events pair;
//! this module reports the [`ExportDisposition`] the facade maps onto
//! the contract events.

mod lock;
mod manifest;
mod note;
mod slug;
mod target;
mod write;

pub use manifest::{load_manifest, manifest_conflict_copies, write_manifest};
pub use note::{NoteParts, render_note, split_note};
pub use write::write_file_atomic;

use std::path::{Path, PathBuf};

use lock::DestinationLock;
use target::{NoteWrite, recorded_target, unlocked_idempotency_hit, unrecorded_target};
use write::{contained_path, write_atomic};

use curio_types::{ArticleFrontmatter, Checksum, CurioId, Destination, ManifestEntry, Timestamp};
use sha2::{Digest as _, Sha256};

/// Everything the exporter needs to write one note. Storage-agnostic:
/// the facade assembles it from the article row, its feed, and its tags.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportInput {
    /// Article identity (`UUIDv7`, never changes).
    pub curio_id: CurioId,
    /// Article title.
    pub title: String,
    /// Canonical article URL.
    pub source: String,
    /// Feed URL, `None` for manual saves.
    pub feed: Option<String>,
    /// Human feed name.
    pub feed_title: Option<String>,
    /// Author, if known.
    pub author: Option<String>,
    /// Publication time, if known.
    pub published: Option<Timestamp>,
    /// When the article was saved in Curio.
    pub saved: Timestamp,
    /// Curio-assigned tags at export time.
    pub tags: Vec<String>,
    /// BCP-47 language tag, if detected.
    pub lang: Option<String>,
    /// Word count, if computed.
    pub word_count: Option<u32>,
    /// The extracted article as `CommonMark` — the managed region body.
    pub markdown: String,
}

/// What one export call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportDisposition {
    /// First export of this `curio_id` into the destination →
    /// `article.saved`.
    Created,
    /// Re-export with a changed checksum → `article.updated`.
    Updated,
    /// Idempotency hit: `(curio_id, checksum)` already exported → no
    /// write, no event.
    Unchanged,
}

/// The result of one export.
#[derive(Debug, Clone, PartialEq)]
pub struct ExportOutcome {
    /// Note path relative to the destination root (the contract `path`).
    pub path: String,
    /// Checksum of the managed-region bytes (change token only).
    pub checksum: Checksum,
    /// What happened.
    pub disposition: ExportDisposition,
}

/// Export failures.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// Filesystem failure.
    #[error("export io at {path}: {source}")]
    Io {
        /// The path being touched.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Frontmatter (de)serialization failed.
    #[error("frontmatter yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    /// The manifest failed to parse — refusing to guess and clobber it.
    #[error("manifest parse at {path}: {message}")]
    ManifestParse {
        /// The manifest path.
        path: PathBuf,
        /// Parse detail.
        message: String,
    },
    /// An existing note could not be re-parsed for a safe re-export.
    #[error("note parse at {path}: {message}")]
    NoteParse {
        /// The note path.
        path: PathBuf,
        /// What was wrong.
        message: String,
    },
    /// The computed note path escaped the destination root.
    #[error("path {path:?} escapes destination root — refusing to write")]
    Containment {
        /// The offending relative path.
        path: String,
    },
    /// The destination's root directory (or, for a destination that has
    /// exported before, its `.curio/` marker) is gone — moved, renamed or
    /// unmounted. Curio refuses to recreate it empty at the old path and
    /// re-export everything into it.
    #[error(
        "destination folder {root} is missing, or no longer holds Curio's .curio/ folder \
         (moved, renamed or unmounted?) — reconnect it, or register the destination again \
         to start it afresh"
    )]
    DestinationMissing {
        /// The registered destination root.
        root: PathBuf,
    },
    /// Every candidate path for the note is held by another article's
    /// note or by a file that is not a Curio note — refusing to
    /// overwrite either.
    #[error(
        "every candidate note path up to {path:?} is taken by another file — refusing to overwrite it"
    )]
    PathsTaken {
        /// The last candidate tried (relative to the destination root).
        path: String,
    },
}

/// A note whose bytes are already durably on disk but whose manifest
/// update is deliberately held back, so the facade can stage the
/// matching event intent *between* the two writes.
///
/// Ordering: note → event intent → manifest. Staging the intent before
/// the manifest write means no crash window can lose the contract
/// event: with no intent and no manifest entry, the next export re-runs
/// (as `Updated` once the article's own note is on disk) and stages then; with an intent but no manifest
/// entry, startup replay emits it (at worst a consumer later sees a
/// duplicate snapshot for the same `curio_id`, which folds
/// idempotently). Were the manifest written first, its
/// `(curio_id, checksum)` idempotency hit would suppress the event on
/// every retry — losing it forever.
///
/// A staged export that writes holds the destination's export lock until
/// it is committed or dropped, so keep the window short: another export to
/// the same destination (any thread, any process) waits for it. The pure
/// idempotency hit writes nothing and takes no lock — an unchanged export
/// still works on a read-only destination.
#[derive(Debug)]
#[must_use = "call commit() — the manifest write is still pending"]
pub struct StagedExport {
    outcome: ExportOutcome,
    pending: Option<PendingManifest>,
    /// Held from the manifest load through the manifest commit; `None`
    /// for the lock-free idempotency hit.
    _lock: Option<DestinationLock>,
}

#[derive(Debug)]
struct PendingManifest {
    root: PathBuf,
    manifest: curio_types::ManifestV1,
    /// Conflict copies folded into `manifest`; removed once it is durable.
    merged_copies: Vec<PathBuf>,
}

impl StagedExport {
    /// What this export did (path, checksum, disposition).
    #[must_use]
    pub fn outcome(&self) -> &ExportOutcome {
        &self.outcome
    }

    /// Commits the held-back manifest write (the contract's second
    /// write; a no-op for the pure idempotency hit).
    ///
    /// # Errors
    ///
    /// [`ExportError::Io`] on manifest write failures.
    pub fn commit(self) -> Result<ExportOutcome, ExportError> {
        if let Some(pending) = self.pending {
            write_manifest(&pending.root, &pending.manifest)?;
            for copy in pending.merged_copies {
                // Its entries are in the durable manifest now; a copy
                // that refuses to go is merged again (idempotently) next
                // time, so this is worth a log line, not a failed export.
                if let Err(err) = std::fs::remove_file(&copy)
                    && err.kind() != std::io::ErrorKind::NotFound
                {
                    tracing::warn!(path = %copy.display(), %err, "could not remove a merged manifest conflict copy");
                }
            }
        }
        Ok(self.outcome)
    }
}

/// Exports one article into a destination, per the contract flow:
/// checksum → manifest lookup → (maybe) note write → manifest write.
///
/// # Errors
///
/// [`ExportError`] on filesystem, YAML, manifest or note-parse failures.
/// The note is never half-written (temp + rename), and the manifest is
/// only updated after the note landed.
pub fn export_note(
    destination: &Destination,
    input: &ExportInput,
) -> Result<ExportOutcome, ExportError> {
    stage_export_note(destination, input)?.commit()
}

/// The staged form of [`export_note`]: performs everything up to and
/// including the note write, returning the manifest write for the
/// caller to [`StagedExport::commit`] after staging its event intent.
///
/// # Errors
///
/// See [`export_note`].
pub fn stage_export_note(
    destination: &Destination,
    input: &ExportInput,
) -> Result<StagedExport, ExportError> {
    // Neutralize managed-region marker literals smuggled in via feed
    // content (e.g. inside a code fence): the bytes between the markers
    // must never parse as a marker, or the next re-export's region
    // surgery would truncate at the embedded marker. The checksum covers
    // the neutralized bytes — exactly what lands on disk.
    let body = note::neutralize_markers(input.markdown.trim_end());
    let body = body.as_ref();
    let checksum = region_checksum(body);
    let root = &destination.root;
    if let Some(outcome) = unlocked_idempotency_hit(root, input, checksum)? {
        return Ok(StagedExport {
            outcome,
            pending: None,
            _lock: None,
        });
    }
    // One export per destination at a time, from the manifest load
    // through the manifest commit — across threads and processes.
    let lock = DestinationLock::acquire(root)?;
    let mut manifest = load_manifest(root)?;
    let merged_copies = manifest::merge_conflict_copies(root, &mut manifest)?;

    let target = match manifest.notes.get(&input.curio_id) {
        Some(entry) => recorded_target(root, &manifest, input, entry, checksum)?,
        None => unrecorded_target(root, &manifest, input, ExportDisposition::Created)?,
    };
    // Every target but the idempotency hit (whose manifest entry already
    // says exactly this) is (re-)recorded — also when the note on disk
    // turns out to be current already and its write is skipped.
    let record = target.write.is_some();
    let outcome = ExportOutcome {
        path: target.rel_path,
        checksum,
        disposition: target.disposition,
    };

    if let Some(write) = target.write {
        // Build the note: fresh, or a managed-region/machine-key
        // replacement that preserves user content byte-for-byte.
        let abs_path = contained_path(root, &outcome.path)?;
        let fm = frontmatter(input, checksum);
        let (content, current) = match write {
            NoteWrite::Replace(old) => {
                let content = note::replace_managed(&old, &fm, body, &abs_path)?;
                let current = content == old;
                (content, current)
            }
            NoteWrite::Fresh => (render_note(&fm, body)?, false),
        };
        // Contract write ordering: note first (skipped when the note on
        // disk already has exactly these bytes)…
        if !current {
            write_atomic(root, &abs_path, content.as_bytes())?;
        }
    }
    if record {
        // …manifest second — held back so the caller can stage its event
        // intent between the two writes (see [`StagedExport`]).
        manifest.notes.insert(
            input.curio_id,
            ManifestEntry {
                path: outcome.path.clone(),
                checksum,
                exported_at: Timestamp::now(),
            },
        );
    } else if merged_copies.is_empty() {
        // The pure idempotency hit: nothing to write at all.
        return Ok(StagedExport {
            outcome,
            pending: None,
            _lock: Some(lock),
        });
    }
    Ok(StagedExport {
        outcome,
        pending: Some(PendingManifest {
            root: root.clone(),
            manifest,
            merged_copies,
        }),
        _lock: Some(lock),
    })
}

/// Checks that a destination which has exported before is still there:
/// its root and the `.curio/` marker the first export created. Callers
/// that know the destination has exported (the facade records it) run
/// this before [`stage_export_note`]; a fresh destination has no marker
/// yet, and the export creates it.
///
/// # Errors
///
/// [`ExportError::DestinationMissing`] when either is gone.
pub fn require_marker(root: &Path) -> Result<(), ExportError> {
    if root.join(".curio").is_dir() {
        Ok(())
    } else {
        Err(ExportError::DestinationMissing {
            root: root.to_path_buf(),
        })
    }
}

/// sha256 over exactly the managed-region bytes (the trimmed markdown
/// body, without the marker lines or their delimiting newlines).
#[must_use]
pub fn region_checksum(body: &str) -> Checksum {
    let digest = Sha256::digest(body.as_bytes());
    Checksum::from_bytes(digest.into())
}

fn frontmatter(input: &ExportInput, checksum: Checksum) -> ArticleFrontmatter {
    ArticleFrontmatter {
        schema: curio_types::FrontmatterSchemaV1Marker,
        curio_id: input.curio_id,
        title: input.title.clone(),
        source: input.source.clone(),
        feed: input.feed.clone(),
        feed_title: input.feed_title.clone(),
        author: input.author.clone(),
        published: input.published,
        saved: input.saved,
        tags: input.tags.clone(),
        checksum,
        lang: input.lang.clone(),
        word_count: input.word_count.map(u64::from),
        extra: std::collections::BTreeMap::new(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn checksum_is_the_sha256_of_the_body_bytes() {
        // sha256("") is the well-known empty digest.
        assert_eq!(
            region_checksum("").to_string(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    pub(super) fn sample_input(curio_id: CurioId, title: &str) -> ExportInput {
        ExportInput {
            curio_id,
            title: title.to_owned(),
            source: "https://example.com/a".to_owned(),
            feed: None,
            feed_title: None,
            author: None,
            published: None,
            saved: Timestamp::now(),
            tags: vec![],
            lang: None,
            word_count: None,
            markdown: "body".to_owned(),
        }
    }

    #[test]
    fn require_marker_wants_the_curio_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(matches!(
            require_marker(dir.path()),
            Err(ExportError::DestinationMissing { .. })
        ));
        std::fs::create_dir(dir.path().join(".curio")).unwrap();
        assert!(require_marker(dir.path()).is_ok());
    }
}
