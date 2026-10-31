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
//! Event emission (`article.saved` / `article.updated`) is wired by the
//! [`crate::CoreHandle`] facade, which owns the storage + events pair;
//! this module reports the [`ExportDisposition`] the facade maps onto
//! the contract events.

mod manifest;
mod note;
mod slug;

pub use manifest::{load_manifest, write_manifest};
pub use note::{NoteParts, render_note, split_note};

use std::path::{Component, Path, PathBuf};

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
}

/// A note whose bytes are already durably on disk but whose manifest
/// update is deliberately held back, so the facade can stage the
/// matching event intent *between* the two writes.
///
/// Ordering: note → event intent → manifest. Staging the intent before
/// the manifest write means no crash window can lose the contract
/// event: with no intent and no manifest entry, the next export re-runs
/// as Created/Updated and stages then; with an intent but no manifest
/// entry, startup replay emits it (at worst a consumer later sees a
/// duplicate snapshot for the same `curio_id`, which folds
/// idempotently). Were the manifest written first, its
/// `(curio_id, checksum)` idempotency hit would suppress the event on
/// every retry — losing it forever.
#[derive(Debug)]
#[must_use = "call commit() — the manifest write is still pending"]
pub struct StagedExport {
    outcome: ExportOutcome,
    pending: Option<PendingManifest>,
}

#[derive(Debug)]
struct PendingManifest {
    root: PathBuf,
    manifest: curio_types::ManifestV1,
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
    let mut manifest = load_manifest(&destination.root)?;

    let existing = manifest.notes.get(&input.curio_id).cloned();
    let (rel_path, disposition) = match &existing {
        Some(entry) => {
            let unchanged = entry.checksum == checksum;
            (
                entry.path.clone(),
                if unchanged {
                    ExportDisposition::Unchanged
                } else {
                    ExportDisposition::Updated
                },
            )
        }
        None => (
            unique_note_path(&destination.root, &manifest, input)?,
            ExportDisposition::Created,
        ),
    };

    let abs_path = contained_path(&destination.root, &rel_path)?;
    let old = read_existing_note(&abs_path)?;

    // The idempotency hit trusts the manifest only while the note on disk
    // is still a note: a blank file (power loss under a pre-fsync write,
    // a sync tool truncating it) falls through and is rewritten fresh,
    // and a non-blank one that no longer parses surfaces as NoteParse
    // instead of being silently reported as exported.
    if let (ExportDisposition::Unchanged, Some(old)) = (disposition, &old) {
        note::parse_note(old, &abs_path)?;
        return Ok(StagedExport {
            outcome: ExportOutcome {
                path: rel_path,
                checksum,
                disposition,
            },
            pending: None,
        });
    }

    // Build the note: fresh, or a managed-region/machine-key replacement
    // that preserves user content byte-for-byte.
    let content = match &old {
        Some(old) => note::replace_managed(old, &frontmatter(input, checksum), body, &abs_path)?,
        None => render_note(&frontmatter(input, checksum), body)?,
    };

    // Contract write ordering: note first…
    write_atomic(&destination.root, &abs_path, content.as_bytes())?;
    // …manifest second — held back so the caller can stage its event
    // intent between the two writes (see [`StagedExport`]).
    manifest.notes.insert(
        input.curio_id,
        ManifestEntry {
            path: rel_path.clone(),
            checksum,
            exported_at: Timestamp::now(),
        },
    );
    Ok(StagedExport {
        outcome: ExportOutcome {
            path: rel_path,
            checksum,
            disposition,
        },
        pending: Some(PendingManifest {
            root: destination.root.clone(),
            manifest,
        }),
    })
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

/// `curio/<slug>.md`, deduplicated against paths owned by *other*
/// articles by suffixing a `curio_id` prefix (deterministic, no counters).
///
/// A path counts as owned by another article when the manifest says so —
/// compared NFC-normalized, because manifests written before slugs were
/// normalized can hold an NFD spelling that names the same file on
/// APFS/HFS+ — or when the note already on disk at that path parses with
/// a different `curio_id`. The disk check catches every other way two
/// byte-distinct names can resolve to one file (case folding the slug's
/// lowercasing does not cover, a manifest lost or rolled back by a sync
/// tool), so a new article never takes over another one's note.
///
/// # Errors
///
/// [`ExportError::Io`] when an existing candidate note cannot be read.
fn unique_note_path(
    root: &Path,
    manifest: &curio_types::ManifestV1,
    input: &ExportInput,
) -> Result<String, ExportError> {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    let slug = slugify(&input.title, &input.curio_id);
    let id = input.curio_id.to_string();
    let short = id.get(..8).unwrap_or(&id);
    let candidates = [
        format!("curio/{slug}.md"),
        format!("curio/{slug}-{short}.md"),
    ];
    for candidate in candidates {
        let in_manifest = manifest.notes.iter().any(|(owner, entry)| {
            *owner != input.curio_id && nfc.normalize(&entry.path) == candidate
        });
        if !in_manifest && !owned_on_disk_by_another(root, &candidate, &input.curio_id)? {
            return Ok(candidate);
        }
    }
    // The full id is unique by construction (lowercase hex, so no case
    // folding can merge two of them).
    Ok(format!("curio/{slug}-{id}.md"))
}

/// Whether the note at `rel` under `root` is another article's: it exists,
/// is not blank, and its frontmatter names a different `curio_id`. A note
/// that does not parse is left to the export itself, which refuses it.
fn owned_on_disk_by_another(
    root: &Path,
    rel: &str,
    curio_id: &CurioId,
) -> Result<bool, ExportError> {
    let path = contained_path(root, rel)?;
    let existing = match read_existing_note(&path) {
        Ok(Some(existing)) => existing,
        // Absent, blank, or not even UTF-8: no other article's identity
        // to protect here.
        Ok(None) | Err(ExportError::NoteParse { .. }) => return Ok(false),
        Err(err) => return Err(err),
    };
    Ok(note::parse_note(&existing, &path)
        .is_ok_and(|(_, frontmatter)| frontmatter.curio_id != *curio_id))
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
fn read_existing_note(path: &Path) -> Result<Option<String>, ExportError> {
    if !path.is_file() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(path).map_err(|source| {
        // Stale blocks a crash left behind are no more a note than a
        // garbled header: report them as unparseable, not as an I/O fault.
        if source.kind() == std::io::ErrorKind::InvalidData {
            ExportError::NoteParse {
                path: path.to_path_buf(),
                message: format!("not valid UTF-8: {source}"),
            }
        } else {
            ExportError::Io {
                path: path.to_path_buf(),
                source,
            }
        }
    })?;
    let blank = content.chars().all(|c| c == '\0' || c.is_whitespace());
    Ok((!blank).then_some(content))
}

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
/// is written without it (see [`tolerate_unsupported_sync`]). Any failure removes the temp file, so no `.curio-tmp-*`
/// stray is left for a sync tool to pick up.
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
    let tmp = dir.join(format!(".{file_name}.curio-tmp-{}", std::process::id()));
    if let Err(source) = write_synced(&tmp, bytes) {
        discard_temp(&tmp);
        return Err(ExportError::Io { path: tmp, source });
    }
    if let Err(source) = std::fs::rename(&tmp, path) {
        discard_temp(&tmp);
        return Err(io_err(source));
    }
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

/// Writes `bytes` to a new file at `path` and flushes them to stable
/// storage before returning — as far as the filesystem can: one that
/// cannot `fsync` at all is tolerated (see [`tolerate_unsupported_sync`]).
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::File::create(path)?;
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

    /// Injected temp-write failure: the temp name itself is occupied by a
    /// directory, so creating the file fails. The error names the temp
    /// path, and the cleanup (which cannot remove a directory with
    /// `remove_file`) is best-effort rather than masking the real error.
    #[test]
    fn a_failed_temp_write_reports_the_temp_path() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("curio");
        let target = dir.join("note.md");
        let tmp = dir.join(format!(".note.md.curio-tmp-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();

        let err = write_atomic(root.path(), &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == tmp),
            "{err}"
        );
        assert!(!target.exists());
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
}
