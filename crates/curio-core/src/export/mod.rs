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

pub use manifest::{load_manifest, manifest_conflict_copies, write_manifest};
pub use note::{NoteParts, render_note, split_note};

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lock::DestinationLock;

use icu_normalizer::ComposingNormalizerBorrowed;
use slug::slugify;

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

/// What occupies a candidate note path.
enum Occupant {
    /// Nothing — or a blank file (empty, whitespace-only, the NUL fill a
    /// crash mid-write can leave) that carries none of the user's bytes.
    /// A fresh note goes here.
    Vacant,
    /// This article's own note; carries its current bytes.
    Ours(String),
    /// Another article's note.
    OtherArticle,
    /// A file Curio cannot account for (not a Curio note, not UTF-8, a
    /// directory…); carries the diagnosis.
    Unaccountable(ExportError),
}

fn occupant(abs_path: &Path, curio_id: CurioId) -> Result<Occupant, ExportError> {
    let unaccountable = |message: &str| {
        Occupant::Unaccountable(ExportError::NoteParse {
            path: abs_path.to_path_buf(),
            message: message.to_owned(),
        })
    };
    if abs_path.is_dir() {
        return Ok(unaccountable("a directory sits at the note path"));
    }
    let content = match read_existing_note(abs_path) {
        Ok(Some(content)) => content,
        Ok(None) => return Ok(Occupant::Vacant),
        Err(err @ ExportError::NoteParse { .. }) => return Ok(Occupant::Unaccountable(err)),
        Err(err) => return Err(err),
    };
    Ok(match note::note_owner(&content, abs_path) {
        Ok(owner) if owner == curio_id => Occupant::Ours(content),
        Ok(_) => Occupant::OtherArticle,
        Err(err) => Occupant::Unaccountable(err),
    })
}

/// Where this export writes, and how.
struct Target {
    rel_path: String,
    disposition: ExportDisposition,
    /// `None` = nothing to write (the idempotency hit).
    write: Option<NoteWrite>,
}

enum NoteWrite {
    /// Render a fresh note.
    Fresh,
    /// Managed-region surgery on this article's existing note bytes.
    Replace(String),
}

/// Decides the target for an article the manifest already records.
fn recorded_target(
    root: &Path,
    manifest: &curio_types::ManifestV1,
    input: &ExportInput,
    entry: &ManifestEntry,
    checksum: Checksum,
) -> Result<Target, ExportError> {
    let unchanged = entry.checksum == checksum;
    let changed = if unchanged {
        ExportDisposition::Unchanged
    } else {
        ExportDisposition::Updated
    };
    let abs_path = contained_path(root, &entry.path)?;
    let (rel_path, disposition, write) = match occupant(&abs_path, input.curio_id)? {
        // The idempotency hit: the manifest is trusted only while the note
        // on disk is still this article's note.
        Occupant::Ours(_) if unchanged => (entry.path.clone(), changed, None),
        Occupant::Ours(old) => (entry.path.clone(), changed, Some(NoteWrite::Replace(old))),
        // The note was deleted, or blanked (power loss under an unflushed
        // write, a sync tool truncating it): write it back where the
        // manifest says.
        Occupant::Vacant => (entry.path.clone(), changed, Some(NoteWrite::Fresh)),
        // Refuse to rewrite a file it cannot account for byte-by-byte —
        // and, even with an unchanged checksum, refuse to report it as
        // exported: the failure surfaces instead of being skipped silently.
        Occupant::Unaccountable(err) => return Err(err),
        // A stale manifest: the recorded path now holds another
        // article's note. Never touch it — move to a free path.
        Occupant::OtherArticle => {
            return unrecorded_target(root, manifest, input, ExportDisposition::Updated);
        }
    };
    Ok(Target {
        rel_path,
        disposition,
        write,
    })
}

/// Decides the target for an article the manifest does not (truthfully)
/// record: the first free deterministic candidate path, or this article's
/// own note when one is already on disk (a lost or stale manifest).
///
/// `fresh` is the disposition of a brand-new note — `Created` for an
/// article the manifest never knew, `Updated` for one whose recorded path
/// turned out to hold another article's note. Finding the article's own
/// note is `Updated`, never `Created`: the note proves an earlier export,
/// so a second `article.saved` would be a lie. Nor is it `Unchanged`
/// (no event), even when the note already carries this checksum — the
/// note may be the leftover of an export that crashed before staging its
/// event intent, and suppressing the event here would lose it for good.
/// `article.updated` carries the same full snapshot and folds
/// idempotently. Either way the manifest records the note again.
fn unrecorded_target(
    root: &Path,
    manifest: &curio_types::ManifestV1,
    input: &ExportInput,
    fresh: ExportDisposition,
) -> Result<Target, ExportError> {
    let (rel_path, existing) = unique_note_path(root, manifest, input)?;
    let (disposition, write) = match existing {
        None => (fresh, Some(NoteWrite::Fresh)),
        Some(old) => (ExportDisposition::Updated, Some(NoteWrite::Replace(old))),
    };
    Ok(Target {
        rel_path,
        disposition,
        write,
    })
}

/// The idempotency hit, decided without the destination lock: it writes
/// nothing, so it needs no serialization (manifest and note are replaced
/// by atomic renames — a concurrent reader sees one version or the
/// other), and it keeps working where the lock cannot be taken (a
/// read-only destination). Anything short of a clean hit — no entry, a
/// changed checksum, a moved or missing note, pending conflict copies —
/// is `None`, and the caller redoes the decision under the lock.
fn unlocked_idempotency_hit(
    root: &Path,
    input: &ExportInput,
    checksum: Checksum,
) -> Result<Option<ExportOutcome>, ExportError> {
    if !root.is_dir() {
        // The locked path reports the missing destination.
        return Ok(None);
    }
    let manifest = load_manifest(root)?;
    let Some(entry) = manifest.notes.get(&input.curio_id) else {
        return Ok(None);
    };
    if entry.checksum != checksum || !manifest_conflict_copies(root)?.is_empty() {
        return Ok(None);
    }
    let target = recorded_target(root, &manifest, input, entry, checksum)?;
    let hit = target.write.is_none() && target.rel_path == entry.path;
    Ok(hit.then_some(ExportOutcome {
        path: target.rel_path,
        checksum,
        disposition: ExportDisposition::Unchanged,
    }))
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

/// `curio/<slug>.md`, deduplicated by suffixing a `curio_id` prefix, then
/// the whole `curio_id` (deterministic, no counters). A candidate is
/// skipped when the manifest gives it to *another* article, and when the
/// file on disk is not this article's own note — another article's note
/// (the manifest that would have said so is lost or stale) or a file that
/// is not a Curio note at all. Returns the path plus, when it holds this
/// article's note already (a lost manifest), that note's current bytes.
///
/// Manifest paths compare NFC-normalized: manifests written before slugs
/// were normalized can hold an NFD spelling that names the same file on
/// APFS/HFS+. The disk check catches every other way two byte-distinct
/// names can resolve to one file (case folding the slug's lowercasing
/// does not cover, a manifest lost or rolled back by a sync tool).
fn unique_note_path(
    root: &Path,
    manifest: &curio_types::ManifestV1,
    input: &ExportInput,
) -> Result<(String, Option<String>), ExportError> {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    let slug = slugify(&input.title, &input.curio_id);
    let id = input.curio_id.to_string();
    let short = id.get(..8).unwrap_or(&id);
    let candidates = [
        format!("curio/{slug}.md"),
        format!("curio/{slug}-{short}.md"),
        format!("curio/{slug}-{id}.md"),
    ];
    let recorded_for_other = |path: &str| {
        manifest
            .notes
            .iter()
            .any(|(other, entry)| *other != input.curio_id && nfc.normalize(&entry.path) == path)
    };
    for candidate in &candidates {
        if recorded_for_other(candidate) {
            continue;
        }
        match occupant(&contained_path(root, candidate)?, input.curio_id)? {
            Occupant::Vacant => return Ok((candidate.clone(), None)),
            Occupant::Ours(existing) => return Ok((candidate.clone(), Some(existing))),
            Occupant::OtherArticle | Occupant::Unaccountable(_) => {}
        }
    }
    let [.., last] = candidates;
    Err(ExportError::PathsTaken { path: last })
}

/// Joins a relative note path under the root, refusing traversal. This
/// is the lexical half of containment; [`write_atomic`] adds the
/// realpath half (symlinked intermediate directories cannot redirect a
/// write outside the root).
fn contained_path(root: &Path, rel: &str) -> Result<PathBuf, ExportError> {
    let rel_path = Path::new(rel);
    let safe = rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !safe || rel_path.is_absolute() {
        return Err(ExportError::Containment {
            path: rel.to_owned(),
        });
    }
    Ok(root.join(rel_path))
}

/// The note currently at `path`, or `None` when there is nothing worth
/// preserving: no file, or a *blank* one — empty, whitespace-only, or the
/// NUL fill some filesystems leave after a crash mid-write. A blank note
/// carries none of the user's bytes, so it is rewritten fresh rather than
/// refused as unparseable.
///
/// # Errors
///
/// [`ExportError::NoteParse`] for a file that is not UTF-8 (stale blocks
/// a crash left behind are no more a note than a garbled header),
/// [`ExportError::Io`] for any other read failure.
fn read_existing_note(path: &Path) -> Result<Option<String>, ExportError> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) if err.kind() == std::io::ErrorKind::InvalidData => {
            return Err(ExportError::NoteParse {
                path: path.to_path_buf(),
                message: format!("the file is not UTF-8 text: {err}"),
            });
        }
        Err(source) => {
            return Err(ExportError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let blank = content.chars().all(|c| c == '\0' || c.is_whitespace());
    Ok((!blank).then_some(content))
}

/// Per-process sequence for temp-file names: the pid alone collides
/// between two writes of one process.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Temp-file + atomic-rename write in the target's own directory,
/// with realpath containment under `root`: after the parent directories
/// exist, both they and the root are canonicalized and the write is
/// refused unless the parent still resolves under the root — a symlinked
/// intermediate directory planted inside the destination cannot redirect
/// the write elsewhere. (The final component is not followed either:
/// `rename` replaces a symlink itself rather than writing through it.)
///
/// Durable, not just atomic: the temp file is `fsync`ed before the
/// rename and the directory after it (plus the parent of every directory
/// this call created, up to the first one that already existed), so
/// after a power loss the target holds the old bytes or the new ones,
/// never the zero-length file a journaled rename of unflushed data can
/// leave behind. That is what makes the contract's note-then-manifest
/// ordering hold across a crash. A filesystem that cannot `fsync` at all
/// is written without it (see [`tolerate_unsupported_sync`]). Any failure
/// removes the temp file, so no `.curio-tmp-*` stray is left for a sync
/// tool to pick up. Temp names are unique per write (pid + a per-process
/// sequence) and created exclusively, so concurrent writes never share
/// one.
pub(crate) fn write_atomic(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let io_err = |source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().ok_or_else(|| ExportError::Containment {
        path: path.display().to_string(),
    })?;
    // The deepest directory that already exists: everything below it is
    // created here, and each new entry needs its parent flushed too.
    let existing_ancestor = dir.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf);
    std::fs::create_dir_all(dir).map_err(io_err)?;
    let canonical_root = root.canonicalize().map_err(|source| ExportError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let canonical_dir = dir.canonicalize().map_err(io_err)?;
    if !canonical_dir.starts_with(&canonical_root) {
        return Err(ExportError::Containment {
            path: path.display().to_string(),
        });
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{file_name}.curio-tmp-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    land_via_temp(&tmp, path, bytes)?;
    sync_dir(dir).map_err(io_err)?;
    for created in dir
        .ancestors()
        .take_while(|a| Some(*a) != existing_ancestor.as_deref())
    {
        if let Some(parent) = created.parent() {
            sync_dir(parent).map_err(io_err)?;
        }
    }
    Ok(())
}

/// Writes `bytes` to the fresh temp file `tmp`, flushes it, and renames
/// it over `path`. On failure the temp file is removed again — unless it
/// could not be created because something already sits at that name,
/// which is not ours to delete.
fn land_via_temp(tmp: &Path, path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    if let Err(source) = write_synced(tmp, bytes) {
        if source.kind() != std::io::ErrorKind::AlreadyExists {
            discard_temp(tmp);
        }
        return Err(ExportError::Io {
            path: tmp.to_path_buf(),
            source,
        });
    }
    if let Err(source) = std::fs::rename(tmp, path) {
        discard_temp(tmp);
        return Err(ExportError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// Writes `bytes` to a new file at `path` — never one that already
/// exists — and flushes them to stable storage before returning, as far
/// as the filesystem can: one that cannot `fsync` at all is tolerated
/// (see [`tolerate_unsupported_sync`]).
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    tolerate_unsupported_sync(file.sync_all(), path)
}

/// Whether an `fsync` failure only says the filesystem cannot flush this
/// handle, rather than that the data failed to reach the disk.
///
/// On Apple platforms `sync_all` is `fcntl(F_FULLFSYNC)`, which Apple
/// documents only for HFS, FAT, UDF and APFS: SMB/NFS shares, exFAT
/// drives and FUSE mounts can answer `ENOTSUP`/`EINVAL`. Some network and
/// FUSE mounts answer a directory `fsync` with `EINVAL` on Linux too.
fn is_sync_unsupported(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
    )
}

/// Maps an `fsync` the filesystem does not support to success: such a
/// filesystem offers no stronger guarantee to wait for, and refusing
/// would make every export to it fail. Real I/O errors pass through.
fn tolerate_unsupported_sync(result: std::io::Result<()>, path: &Path) -> std::io::Result<()> {
    match result {
        Err(err) if is_sync_unsupported(&err) => {
            tracing::debug!(path = %path.display(), %err, "fsync unsupported by this filesystem");
            Ok(())
        }
        other => other,
    }
}

/// Best-effort removal of a temp file after a failed write: the
/// original error is what the caller reports, so a failed cleanup is
/// only logged.
fn discard_temp(tmp: &Path) {
    if let Err(err) = std::fs::remove_file(tmp)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %tmp.display(), %err, "could not remove export temp file");
    }
}

/// Flushes a directory's entries (the rename that just landed) to
/// stable storage. Filesystems that cannot `fsync` a directory are
/// tolerated (see [`tolerate_unsupported_sync`]). On Windows a directory
/// cannot be opened as a file and NTFS journals the rename itself, so
/// this is a no-op there.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let handle = std::fs::File::open(dir)?;
        tolerate_unsupported_sync(handle.sync_all(), dir)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
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

    /// Names of every `.curio-tmp-*` stray left in `dir`.
    fn temp_strays(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".curio-tmp-"))
            .collect()
    }

    #[test]
    fn write_atomic_creates_parents_and_lands_the_bytes() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("curio").join("note.md");
        write_atomic(root.path(), &target, b"first").unwrap();
        write_atomic(root.path(), &target, b"second").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second");
        assert_eq!(
            temp_strays(&root.path().join("curio")),
            Vec::<String>::new()
        );
    }

    /// Injected rename failure: a directory squats on the target name, so
    /// the temp file is fully written and synced but `rename` refuses
    /// (EISDIR on unix, access denied on Windows). The error must name
    /// the target and the temp file must be gone.
    #[test]
    fn a_failed_rename_leaves_no_temp_file() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("curio");
        let target = dir.join("note.md");
        std::fs::create_dir_all(target.join("occupied")).unwrap();

        let err = write_atomic(root.path(), &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == target),
            "{err}"
        );
        assert_eq!(temp_strays(&dir), Vec::<String>::new());
        assert!(
            target.join("occupied").is_dir(),
            "the squatter is untouched"
        );
    }

    /// Injected temp-write failure: the temp name itself is occupied (by a
    /// directory, and by a file), so creating the temp file fails. The
    /// error names the temp path, nothing lands at the target, and the
    /// squatter — not ours — is left exactly as it was.
    #[test]
    fn a_failed_temp_write_reports_the_temp_path() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("curio");
        let target = dir.join("note.md");
        let tmp = dir.join(".note.md.curio-tmp-squatted");
        std::fs::create_dir_all(&tmp).unwrap();

        let err = land_via_temp(&tmp, &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == tmp),
            "{err}"
        );
        assert!(!target.exists());
        assert!(tmp.is_dir(), "the squatting directory is untouched");

        std::fs::remove_dir(&tmp).unwrap();
        std::fs::write(&tmp, b"someone else's").unwrap();
        let err = land_via_temp(&tmp, &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == tmp),
            "{err}"
        );
        assert!(!target.exists());
        assert_eq!(std::fs::read(&tmp).unwrap(), b"someone else's");
    }

    #[test]
    fn write_atomic_creates_several_missing_levels() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("a").join("b").join("c").join("note.md");
        write_atomic(root.path(), &target, b"deep").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"deep");
    }

    #[test]
    fn unsupported_fsync_is_tolerated_but_real_errors_are_not() {
        use std::io::{Error, ErrorKind};
        let path = Path::new("/vault/curio/note.md");
        for kind in [ErrorKind::Unsupported, ErrorKind::InvalidInput] {
            assert!(is_sync_unsupported(&Error::from(kind)), "{kind:?}");
            assert!(tolerate_unsupported_sync(Err(Error::from(kind)), path).is_ok());
        }
        // EINVAL (22 on every unix) as the OS reports it maps the same way.
        #[cfg(unix)]
        assert!(is_sync_unsupported(&Error::from_raw_os_error(22)));
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::StorageFull,
            ErrorKind::Other,
        ] {
            assert!(!is_sync_unsupported(&Error::from(kind)), "{kind:?}");
            let err = tolerate_unsupported_sync(Err(Error::from(kind)), path).unwrap_err();
            assert_eq!(err.kind(), kind);
        }
        assert!(tolerate_unsupported_sync(Ok(()), path).is_ok());
    }

    #[test]
    fn non_utf8_notes_are_parse_errors_not_io_errors() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        std::fs::write(&path, b"---\n\xff\xfe stale blocks").unwrap();
        assert!(matches!(
            read_existing_note(&path),
            Err(ExportError::NoteParse { .. })
        ));
    }

    #[test]
    fn blank_notes_read_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("note.md");
        assert_eq!(read_existing_note(&path).unwrap(), None);
        for blank in ["", " \n\t", "\0\0\0\0"] {
            std::fs::write(&path, blank).unwrap();
            assert_eq!(read_existing_note(&path).unwrap(), None, "{blank:?}");
        }
        std::fs::write(&path, "x").unwrap();
        assert_eq!(read_existing_note(&path).unwrap().as_deref(), Some("x"));
    }

    #[test]
    fn contained_path_refuses_traversal() {
        let root = Path::new("/tmp/dest");
        assert!(contained_path(root, "curio/note.md").is_ok());
        assert!(matches!(
            contained_path(root, "../escape.md"),
            Err(ExportError::Containment { .. })
        ));
        assert!(matches!(
            contained_path(root, "/abs/path.md"),
            Err(ExportError::Containment { .. })
        ));
        assert!(matches!(
            contained_path(root, "a/../../b.md"),
            Err(ExportError::Containment { .. })
        ));
    }

    fn sample_input(curio_id: CurioId, title: &str) -> ExportInput {
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
    fn occupant_classifies_every_kind_of_file() {
        let dir = tempfile::tempdir().unwrap();
        let id = CurioId::new();
        let path = dir.path().join("n.md");
        assert!(matches!(occupant(&path, id).unwrap(), Occupant::Vacant));

        let ours = render_note(
            &frontmatter(&sample_input(id, "T"), region_checksum("b")),
            "b",
        )
        .unwrap();
        std::fs::write(&path, &ours).unwrap();
        assert!(matches!(occupant(&path, id).unwrap(), Occupant::Ours(c) if c == ours));
        assert!(matches!(
            occupant(&path, CurioId::new()).unwrap(),
            Occupant::OtherArticle
        ));

        for bytes in [&b"# just markdown\n"[..], &[0xff, 0xfe, 0x00][..]] {
            std::fs::write(&path, bytes).unwrap();
            assert!(matches!(
                occupant(&path, id).unwrap(),
                Occupant::Unaccountable(ExportError::NoteParse { .. })
            ));
        }

        let sub = dir.path().join("sub.md");
        std::fs::create_dir(&sub).unwrap();
        assert!(matches!(
            occupant(&sub, id).unwrap(),
            Occupant::Unaccountable(ExportError::NoteParse { .. })
        ));
    }

    #[test]
    fn every_candidate_taken_is_refused_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let input = sample_input(CurioId::new(), "Busy");
        let id = input.curio_id.to_string();
        std::fs::create_dir_all(dir.path().join("curio")).unwrap();
        for name in [
            "busy.md".to_owned(),
            format!("busy-{}.md", &id[..8]),
            format!("busy-{id}.md"),
        ] {
            std::fs::write(dir.path().join("curio").join(name), "mine\n").unwrap();
        }
        let err =
            unique_note_path(dir.path(), &curio_types::ManifestV1::new(), &input).unwrap_err();
        assert!(matches!(err, ExportError::PathsTaken { .. }), "{err}");
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

    #[test]
    fn a_failed_atomic_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // The rename target is a non-empty directory: the rename fails
        // after the temp file was written.
        let target = dir.path().join("note.md");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep"), "x").unwrap();
        assert!(write_atomic(dir.path(), &target, b"bytes").is_err());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".curio-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
