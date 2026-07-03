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

pub use manifest::{load_manifest, write_manifest};
pub use note::{NoteParts, render_note, split_note};

use std::path::{Component, Path, PathBuf};

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
            unique_note_path(&manifest, input),
            ExportDisposition::Created,
        ),
    };

    let abs_path = contained_path(&destination.root, &rel_path)?;
    let note_exists = abs_path.is_file();

    if disposition == ExportDisposition::Unchanged && note_exists {
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
    let content = if note_exists {
        let old = read_file(&abs_path)?;
        note::replace_managed(&old, &frontmatter(input, checksum), body, &abs_path)?
    } else {
        render_note(&frontmatter(input, checksum), body)?
    };

    // Contract write ordering: note first…
    write_atomic(&abs_path, content.as_bytes())?;
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
fn unique_note_path(manifest: &curio_types::ManifestV1, input: &ExportInput) -> String {
    let slug = slugify(&input.title, &input.curio_id);
    let candidate = format!("curio/{slug}.md");
    let taken = |path: &str| {
        manifest
            .notes
            .iter()
            .any(|(id, entry)| *id != input.curio_id && entry.path == path)
    };
    if !taken(&candidate) {
        return candidate;
    }
    let id = input.curio_id.to_string();
    let short = id.get(..8).unwrap_or(&id);
    let suffixed = format!("curio/{slug}-{short}.md");
    if taken(&suffixed) {
        format!("curio/{slug}-{id}.md")
    } else {
        suffixed
    }
}

fn slugify(title: &str, curio_id: &CurioId) -> String {
    let mut slug = String::new();
    let mut pending_dash = false;
    for c in title.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() {
            if pending_dash && !slug.is_empty() {
                slug.push('-');
            }
            pending_dash = false;
            slug.push(c);
            if slug.chars().count() >= 80 {
                break;
            }
        } else {
            pending_dash = true;
        }
    }
    if slug.is_empty() {
        curio_id.to_string()
    } else {
        slug
    }
}

/// Joins a relative note path under the root, refusing traversal.
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

fn read_file(path: &Path) -> Result<String, ExportError> {
    std::fs::read_to_string(path).map_err(|source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Temp-file + atomic-rename write in the target's own directory.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let io_err = |source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().ok_or_else(|| ExportError::Containment {
        path: path.display().to_string(),
    })?;
    std::fs::create_dir_all(dir).map_err(io_err)?;
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = dir.join(format!(".{file_name}.curio-tmp-{}", std::process::id()));
    std::fs::write(&tmp, bytes).map_err(|source| ExportError::Io {
        path: tmp.clone(),
        source,
    })?;
    std::fs::rename(&tmp, path).map_err(io_err)
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

    #[test]
    fn slugify_is_boring_and_safe() {
        let id = CurioId::new();
        assert_eq!(slugify("Hello, World!", &id), "hello-world");
        assert_eq!(slugify("../../etc/passwd", &id), "etc-passwd");
        assert_eq!(slugify("Ünïcode Tîtle", &id), "ünïcode-tîtle");
        assert_eq!(slugify("", &id), id.to_string());
        assert_eq!(slugify("!!!", &id), id.to_string());
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
