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

mod batch;
#[cfg(test)]
mod golden_tests;
mod lock;
mod manifest;
mod note;
#[cfg(test)]
mod roundtrip_tests;
mod slug;
mod target;
mod write;
pub mod yaml;

pub use batch::{ExportBatch, StageError};
#[cfg(test)]
pub(crate) use manifest::WRITES as MANIFEST_WRITES;
pub use manifest::{exported_ids, load_manifest, manifest_conflict_copies, write_manifest};
pub use note::{NoteParts, render_note, split_note};
pub use write::write_file_atomic;

use std::path::{Path, PathBuf};

use write::{contained_path, write_atomic};

use curio_types::{ArticleFrontmatter, Checksum, CurioId, Destination, Timestamp};
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
///
/// Filesystem failures are sorted by what the user can do about them:
/// [`ExportError::PermissionDenied`] (fix the folder's permissions, or the
/// read-only volume), [`ExportError::DiskFull`] (free some space) and
/// [`ExportError::DestinationMissing`] (reconnect the vault); anything else
/// is [`ExportError::Io`]. [`ExportError::io`] does the sorting, and every
/// filesystem call site builds its error through it.
#[derive(Debug, thiserror::Error)]
pub enum ExportError {
    /// Filesystem failure of no more specific class.
    #[error("export io at {path}: {source}")]
    Io {
        /// The path being touched.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The destination (or a folder or file inside it) cannot be read or
    /// written: its permissions refuse Curio (`EACCES`/`EPERM`, Windows
    /// "access denied"), or a write hit a read-only volume (`EROFS`). Both
    /// directions land here — an export reads existing notes and the
    /// manifest as well as writing them — so the message names neither.
    /// Exporting again succeeds once the permissions are fixed.
    #[error(
        "cannot access {path}: permission denied or read-only — check that Curio may \
         read and write the destination folder and its notes, and that its volume is \
         not mounted read-only"
    )]
    PermissionDenied {
        /// The path that refused the access.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The disk (or the user's quota) holding the destination is full
    /// (`ENOSPC`/`EDQUOT`, Windows `ERROR_DISK_FULL`). The atomic writes
    /// leave the previous note intact; exporting again succeeds once space
    /// is freed.
    #[error("the disk holding {path} is full — free some space and export again")]
    DiskFull {
        /// The path whose write ran out of space.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// Frontmatter (de)serialization failed.
    #[error("frontmatter yaml: {0}")]
    Yaml(#[from] yaml::Error),
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

impl ExportError {
    /// Sorts a filesystem failure at `path` into its class:
    /// [`ExportError::PermissionDenied`] for a permission refusal or a
    /// read-only filesystem, [`ExportError::DiskFull`] for an exhausted
    /// disk or quota, [`ExportError::Io`] for anything else.
    #[must_use]
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        use std::io::ErrorKind;
        let path = path.into();
        match source.kind() {
            ErrorKind::PermissionDenied | ErrorKind::ReadOnlyFilesystem => {
                Self::PermissionDenied { path, source }
            }
            ErrorKind::StorageFull | ErrorKind::QuotaExceeded => Self::DiskFull { path, source },
            _ => Self::Io { path, source },
        }
    }
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
///
/// It is an [`ExportBatch`] of one note; a bulk export stages many notes
/// into one batch and commits their manifest write once.
#[derive(Debug)]
#[must_use = "call commit() — the manifest write is still pending"]
pub struct StagedExport {
    outcome: ExportOutcome,
    batch: ExportBatch,
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
    /// A filesystem error (sorted by [`ExportError::io`]) on manifest
    /// write failures.
    pub fn commit(self) -> Result<ExportOutcome, ExportError> {
        self.batch.commit()?;
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
    let mut batch = ExportBatch::begin(destination)?;
    let outcome = batch.stage(input)?;
    Ok(StagedExport { outcome, batch })
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
