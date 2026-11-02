//! Batched export: many notes into one destination under one manifest
//! load and one manifest write.
//!
//! [`stage_export_note`](super::stage_export_note) is a batch of one. A
//! bulk export stages a page of notes into one [`ExportBatch`], stages
//! their event intents, then commits the batch's single manifest write —
//! the contract's note → event intent → manifest ordering, per batch
//! instead of per note. Re-reading, re-indexing and rewriting the whole
//! manifest once per note made a bulk export quadratic in the size of the
//! destination.

use std::path::{Path, PathBuf};

use curio_types::{Checksum, Destination, ManifestEntry, ManifestV1, Timestamp};

use super::lock::DestinationLock;
use super::target::{
    NoteWrite, PathIndex, recorded_target, unlocked_idempotency_hit, unrecorded_target,
};
use super::write::{contained_path, write_atomic};
use super::{
    ExportDisposition, ExportError, ExportInput, ExportOutcome, frontmatter, load_manifest,
    manifest, manifest_conflict_copies, note, region_checksum, render_note, write_manifest,
};

/// Why staging one note into an [`ExportBatch`] failed.
#[derive(Debug, thiserror::Error)]
pub enum StageError {
    /// The destination itself failed: its root is missing, its export
    /// lock cannot be taken, or its manifest does not load. Every further
    /// note would fail the same way. Nothing of this note was written.
    #[error(transparent)]
    Destination(ExportError),
    /// This note failed: its file cannot be accounted for, every
    /// candidate path is taken, or its write failed. Nothing of it was
    /// recorded, and the batch is intact — later notes can still be
    /// staged and the batch committed.
    #[error(transparent)]
    Note(ExportError),
}

impl From<StageError> for ExportError {
    fn from(err: StageError) -> Self {
        match err {
            StageError::Destination(err) | StageError::Note(err) => err,
        }
    }
}

/// Notes staged into one destination, waiting for their single manifest
/// write.
///
/// Each [`Self::stage`] writes its note (unless it is an idempotency hit)
/// and records it in the batch's in-memory manifest; [`Self::commit`]
/// writes that manifest once. Between the two the caller stages the event
/// intents of every note the batch wrote, so the crash analysis of
/// [`StagedExport`](super::StagedExport) holds for the whole batch: a
/// crash before the commit leaves notes on disk with no manifest entry,
/// and the next export reconciles each onto its own note as `Updated` —
/// its event is never lost. Dropping a batch without committing abandons
/// its manifest write the same way.
///
/// Idempotency hits are decided against the manifest as read when the
/// batch began, without the destination lock — an unchanged re-export
/// writes nothing, takes no lock, and works on a read-only destination.
/// The first note that needs a write takes the destination's export lock
/// and re-reads the manifest under it (folding in any sync-tool conflict
/// copies); the batch then holds the lock until it is committed or
/// dropped, so keep batches short: another export to the same
/// destination (any thread, any process) waits for it.
#[derive(Debug)]
#[must_use = "call commit() — the manifest write is still pending"]
pub struct ExportBatch {
    root: PathBuf,
    /// The manifest as read without the lock, for lock-free idempotency
    /// hits; `None` when none can be decided that way (a missing root,
    /// or conflict copies waiting to be merged under the lock).
    unlocked: Option<ManifestV1>,
    /// Taken by the first note that needs a write.
    locked: Option<Locked>,
}

/// The locked half of a batch: the manifest re-read under the lock, the
/// path index of its collision checks, and what is pending.
#[derive(Debug)]
struct Locked {
    manifest: ManifestV1,
    paths: PathIndex,
    /// Conflict copies folded into `manifest`; removed once it is durable.
    merged_copies: Vec<PathBuf>,
    /// Whether a note was (re-)recorded since the manifest was read.
    dirty: bool,
    /// Held from the manifest load through the manifest commit.
    _lock: DestinationLock,
}

impl ExportBatch {
    /// Starts a batch into `destination`, reading its manifest (without
    /// the lock) for the idempotency hits.
    ///
    /// # Errors
    ///
    /// The manifest exists but cannot be read or parsed, or `.curio/`
    /// cannot be listed for conflict copies — destination-wide failures.
    /// A missing root is not an error here: the first note that needs a
    /// write reports it as [`ExportError::DestinationMissing`].
    pub fn begin(destination: &Destination) -> Result<Self, ExportError> {
        let root = destination.root.clone();
        let unlocked = if root.is_dir() {
            let manifest = load_manifest(&root)?;
            manifest_conflict_copies(&root)?
                .is_empty()
                .then_some(manifest)
        } else {
            None
        };
        Ok(Self {
            root,
            unlocked,
            locked: None,
        })
    }

    /// Stages one note: checksum → idempotency check → (maybe) note write
    /// → manifest entry recorded in memory. The manifest itself is only
    /// written by [`Self::commit`].
    ///
    /// # Errors
    ///
    /// [`StageError::Destination`] when the destination cannot be locked
    /// or its manifest re-read; [`StageError::Note`] for anything about
    /// this note alone. Either way nothing of this note is recorded.
    pub fn stage(&mut self, input: &ExportInput) -> Result<ExportOutcome, StageError> {
        // Neutralize managed-region marker literals smuggled in via feed
        // content (e.g. inside a code fence): the bytes between the markers
        // must never parse as a marker, or the next re-export's region
        // surgery would truncate at the embedded marker. The checksum covers
        // the neutralized bytes — exactly what lands on disk.
        let body = note::neutralize_markers(input.markdown.trim_end());
        let body = body.as_ref();
        let checksum = region_checksum(body);
        if self.locked.is_none()
            && let Some(manifest) = &self.unlocked
            && let Some(hit) = unlocked_idempotency_hit(&self.root, manifest, input, checksum)
                .map_err(StageError::Note)?
        {
            return Ok(hit);
        }
        let locked = match self.locked.take() {
            Some(locked) => locked,
            None => Locked::acquire(&self.root).map_err(StageError::Destination)?,
        };
        self.locked
            .insert(locked)
            .stage(&self.root, input, body, checksum)
            .map_err(StageError::Note)
    }

    /// Commits the batch's manifest write — the contract's second write
    /// for every note staged — and releases the destination lock. A no-op
    /// for a batch of idempotency hits.
    ///
    /// # Errors
    ///
    /// A filesystem error (sorted by [`ExportError::io`]) on manifest
    /// write failures.
    pub fn commit(self) -> Result<(), ExportError> {
        let Some(locked) = self.locked else {
            return Ok(());
        };
        if !locked.dirty && locked.merged_copies.is_empty() {
            // Every note was a hit after all: nothing to write.
            return Ok(());
        }
        write_manifest(&self.root, &locked.manifest)?;
        for copy in locked.merged_copies {
            // Its entries are in the durable manifest now; a copy that
            // refuses to go is merged again (idempotently) next time, so
            // this is worth a log line, not a failed export.
            if let Err(err) = std::fs::remove_file(&copy)
                && err.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(path = %copy.display(), %err, "could not remove a merged manifest conflict copy");
            }
        }
        Ok(())
    }
}

impl Locked {
    /// One export per destination at a time, from the manifest load
    /// through the manifest commit — across threads and processes.
    fn acquire(root: &Path) -> Result<Self, ExportError> {
        let lock = DestinationLock::acquire(root)?;
        let mut manifest = load_manifest(root)?;
        let merged_copies = manifest::merge_conflict_copies(root, &mut manifest)?;
        Ok(Self {
            paths: PathIndex::build(&manifest),
            manifest,
            merged_copies,
            dirty: false,
            _lock: lock,
        })
    }

    fn stage(
        &mut self,
        root: &Path,
        input: &ExportInput,
        body: &str,
        checksum: Checksum,
    ) -> Result<ExportOutcome, ExportError> {
        let target = match self.manifest.notes.get(&input.curio_id) {
            Some(entry) => recorded_target(root, &self.paths, input, entry, checksum)?,
            None => unrecorded_target(root, &self.paths, input, ExportDisposition::Created)?,
        };
        let outcome = ExportOutcome {
            path: target.rel_path,
            checksum,
            disposition: target.disposition,
        };
        // The pure idempotency hit: its manifest entry already says
        // exactly this.
        let Some(write) = target.write else {
            return Ok(outcome);
        };
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
        // …manifest second — held back for the commit, so the caller can
        // stage its event intents between the two writes. Every target but
        // the idempotency hit is (re-)recorded, also when the note on disk
        // turned out to be current already.
        let previous = self.manifest.notes.insert(
            input.curio_id,
            ManifestEntry {
                path: outcome.path.clone(),
                checksum,
                exported_at: Timestamp::now(),
            },
        );
        self.paths.record(
            input.curio_id,
            previous.as_ref().map(|entry| entry.path.as_str()),
            &outcome.path,
        );
        self.dirty = true;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::cell::Cell;

    use curio_types::CurioId;

    use super::super::MANIFEST_WRITES;
    use super::super::tests::sample_input;
    use super::*;

    fn writes() -> usize {
        MANIFEST_WRITES.with(Cell::get)
    }

    fn destination(root: &Path) -> Destination {
        Destination {
            name: "vault".parse().unwrap(),
            root: root.to_path_buf(),
        }
    }

    #[test]
    fn a_batch_writes_every_note_then_the_manifest_once() {
        let dir = tempfile::tempdir().unwrap();
        let dest = destination(dir.path());
        // Two same-titled articles in one batch: the second sees the
        // first's claim before any manifest is on disk.
        let first = sample_input(CurioId::new(), "Weekly links");
        let second = sample_input(CurioId::new(), "Weekly links");
        let before = writes();
        let mut batch = ExportBatch::begin(&dest).unwrap();
        let a = batch.stage(&first).unwrap();
        let b = batch.stage(&second).unwrap();
        assert_eq!(a.path, "curio/weekly-links.md");
        assert_ne!(b.path, a.path, "the in-batch claim is honoured");
        assert_eq!(
            (a.disposition, b.disposition),
            (ExportDisposition::Created, ExportDisposition::Created)
        );
        assert!(dir.path().join(&b.path).is_file(), "notes land first");
        assert_eq!(writes(), before, "no manifest write before the commit");
        batch.commit().unwrap();
        assert_eq!(writes(), before + 1);
        let manifest = load_manifest(dir.path()).unwrap();
        assert_eq!(manifest.notes[&first.curio_id].path, a.path);
        assert_eq!(manifest.notes[&second.curio_id].path, b.path);

        // Unchanged re-export: lock-free hits, and the commit writes nothing.
        let mut batch = ExportBatch::begin(&dest).unwrap();
        for input in [&first, &second] {
            assert_eq!(
                batch.stage(input).unwrap().disposition,
                ExportDisposition::Unchanged
            );
        }
        assert!(batch.locked.is_none(), "hits take no lock");
        batch.commit().unwrap();
        assert_eq!(writes(), before + 1);
    }

    #[test]
    fn a_failed_note_leaves_the_batch_intact() {
        let dir = tempfile::tempdir().unwrap();
        let dest = destination(dir.path());
        let busy = sample_input(CurioId::new(), "Busy");
        let id = busy.curio_id.to_string();
        std::fs::create_dir_all(dir.path().join("curio")).unwrap();
        for name in [
            "busy.md".to_owned(),
            format!("busy-{}.md", &id[..8]),
            format!("busy-{id}.md"),
        ] {
            std::fs::write(dir.path().join("curio").join(name), "mine\n").unwrap();
        }
        let fine = sample_input(CurioId::new(), "Fine");
        let mut batch = ExportBatch::begin(&dest).unwrap();
        let err = batch.stage(&busy).unwrap_err();
        assert!(
            matches!(err, StageError::Note(ExportError::PathsTaken { .. })),
            "{err:?}"
        );
        batch.stage(&fine).unwrap();
        batch.commit().unwrap();
        let manifest = load_manifest(dir.path()).unwrap();
        assert_eq!(manifest.notes.len(), 1);
        assert!(manifest.notes.contains_key(&fine.curio_id));
    }

    #[test]
    fn a_missing_root_is_a_destination_failure() {
        let dir = tempfile::tempdir().unwrap();
        let dest = destination(&dir.path().join("gone"));
        let mut batch = ExportBatch::begin(&dest).unwrap();
        let err = batch.stage(&sample_input(CurioId::new(), "T")).unwrap_err();
        assert!(
            matches!(
                err,
                StageError::Destination(ExportError::DestinationMissing { .. })
            ),
            "{err:?}"
        );
        assert!(matches!(
            ExportError::from(err),
            ExportError::DestinationMissing { .. }
        ));
        assert!(!dir.path().join("gone").exists(), "never recreated");
    }

    #[test]
    fn an_unparseable_manifest_fails_the_batch_up_front() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".curio")).unwrap();
        std::fs::write(dir.path().join(".curio/manifest.json"), "{ nope").unwrap();
        let err = ExportBatch::begin(&destination(dir.path())).unwrap_err();
        assert!(matches!(err, ExportError::ManifestParse { .. }), "{err}");
    }
}
