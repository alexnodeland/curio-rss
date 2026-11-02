//! Where an export writes: the occupant of each candidate note path (a
//! blank note reads as vacant), the target decision for recorded and
//! unrecorded articles, the lock-free idempotency hit, the deterministic
//! candidate paths, and the path index their collision checks consult.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use icu_normalizer::ComposingNormalizerBorrowed;

use curio_types::{Checksum, CurioId, ManifestEntry, ManifestV1};

use super::slug::slugify;
use super::write::contained_path;
use super::{ExportDisposition, ExportError, ExportInput, ExportOutcome, note};

/// The manifest's note paths, NFC-normalized, mapped to the articles that
/// claim them — the collision check of [`unique_note_path`] in one lookup
/// instead of a scan of every entry (which made a bulk export quadratic).
///
/// Manifests written before slugs were normalized can hold an NFD spelling
/// that names the same file on APFS/HFS+, so keys are normalized. A path
/// normally has one claimant; a hand-edited manifest can give it several,
/// and every one of them counts.
#[derive(Debug, Default)]
pub(super) struct PathIndex(BTreeMap<String, BTreeSet<CurioId>>);

impl PathIndex {
    /// Indexes every entry of `manifest`.
    pub(super) fn build(manifest: &ManifestV1) -> Self {
        let mut index = Self::default();
        for (id, entry) in &manifest.notes {
            index.insert(*id, &entry.path);
        }
        index
    }

    /// Whether the manifest gives `path` (already NFC) to an article other
    /// than `id`.
    fn held_by_other(&self, path: &str, id: CurioId) -> bool {
        self.0
            .get(path)
            .is_some_and(|ids| ids.iter().any(|other| *other != id))
    }

    /// Moves `id`'s claim from `old` (its previous manifest path, if any)
    /// to `new` — mirroring a manifest entry being (re-)recorded.
    pub(super) fn record(&mut self, id: CurioId, old: Option<&str>, new: &str) {
        if let Some(old) = old {
            let key = normalize(old);
            if let Some(ids) = self.0.get_mut(&key) {
                ids.remove(&id);
                if ids.is_empty() {
                    self.0.remove(&key);
                }
            }
        }
        self.insert(id, new);
    }

    fn insert(&mut self, id: CurioId, path: &str) {
        self.0.entry(normalize(path)).or_default().insert(id);
    }
}

fn normalize(path: &str) -> String {
    ComposingNormalizerBorrowed::new_nfc()
        .normalize(path)
        .into_owned()
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
pub(super) struct Target {
    pub(super) rel_path: String,
    pub(super) disposition: ExportDisposition,
    /// `None` = nothing to write (the idempotency hit).
    pub(super) write: Option<NoteWrite>,
}

pub(super) enum NoteWrite {
    /// Render a fresh note.
    Fresh,
    /// Managed-region surgery on this article's existing note bytes.
    Replace(String),
}

/// Decides the target for an article the manifest already records.
pub(super) fn recorded_target(
    root: &Path,
    paths: &PathIndex,
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
            return unrecorded_target(root, paths, input, ExportDisposition::Updated);
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
pub(super) fn unrecorded_target(
    root: &Path,
    paths: &PathIndex,
    input: &ExportInput,
    fresh: ExportDisposition,
) -> Result<Target, ExportError> {
    let (rel_path, existing) = unique_note_path(root, paths, input)?;
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
/// read-only destination). `manifest` is the destination's manifest as
/// read without the lock, with no conflict copies pending (the caller
/// checks). Anything short of a clean hit — no entry, a changed checksum,
/// a moved or missing note — is `None`, and the caller redoes the
/// decision under the lock.
///
/// # Errors
///
/// The note at the recorded path cannot be read, or cannot be accounted
/// for: even with an unchanged checksum, such a note is never reported as
/// exported (see [`recorded_target`]).
pub(super) fn unlocked_idempotency_hit(
    root: &Path,
    manifest: &ManifestV1,
    input: &ExportInput,
    checksum: Checksum,
) -> Result<Option<ExportOutcome>, ExportError> {
    let Some(entry) = manifest.notes.get(&input.curio_id) else {
        return Ok(None);
    };
    if entry.checksum != checksum {
        return Ok(None);
    }
    // Exactly the hit arm of `recorded_target`: only the article's own
    // note at the recorded path is a hit; an unaccountable file there is
    // the same refusal; anything else needs a write, under the lock.
    match occupant(&contained_path(root, &entry.path)?, input.curio_id)? {
        Occupant::Ours(_) => Ok(Some(ExportOutcome {
            path: entry.path.clone(),
            checksum,
            disposition: ExportDisposition::Unchanged,
        })),
        Occupant::Unaccountable(err) => Err(err),
        Occupant::Vacant | Occupant::OtherArticle => Ok(None),
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
/// Manifest paths compare NFC-normalized (see [`PathIndex`]). The disk
/// check catches every other way two byte-distinct names can resolve to
/// one file (case folding the slug's lowercasing does not cover, a
/// manifest lost or rolled back by a sync tool).
fn unique_note_path(
    root: &Path,
    paths: &PathIndex,
    input: &ExportInput,
) -> Result<(String, Option<String>), ExportError> {
    let slug = slugify(&input.title, &input.curio_id);
    let id = input.curio_id.to_string();
    let short = id.get(..8).unwrap_or(&id);
    let candidates = [
        format!("curio/{slug}.md"),
        format!("curio/{slug}-{short}.md"),
        format!("curio/{slug}-{id}.md"),
    ];
    for candidate in &candidates {
        if paths.held_by_other(candidate, input.curio_id) {
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
/// any other read failure as sorted by [`ExportError::io`].
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
        Err(source) => return Err(ExportError::io(path, source)),
    };
    let blank = content.chars().all(|c| c == '\0' || c.is_whitespace());
    Ok((!blank).then_some(content))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::super::{frontmatter, region_checksum, render_note, tests::sample_input};
    use super::*;

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
        let err = unique_note_path(dir.path(), &PathIndex::default(), &input).unwrap_err();
        assert!(matches!(err, ExportError::PathsTaken { .. }), "{err}");
    }

    #[test]
    fn the_path_index_follows_recorded_claims_across_spellings() {
        let mut manifest = ManifestV1::new();
        let legacy = CurioId::new();
        manifest.notes.insert(
            legacy,
            ManifestEntry {
                path: "curio/cafe\u{301}.md".to_owned(),
                checksum: region_checksum("b"),
                exported_at: curio_types::Timestamp::now(),
            },
        );
        let mut index = PathIndex::build(&manifest);
        let other = CurioId::new();
        // An NFD manifest path claims the NFC candidate — but not from
        // its own article.
        assert!(index.held_by_other("curio/caf\u{e9}.md", other));
        assert!(!index.held_by_other("curio/caf\u{e9}.md", legacy));
        // Re-recording moves the claim: the old path is free again.
        index.record(legacy, Some("curio/cafe\u{301}.md"), "curio/new.md");
        assert!(!index.held_by_other("curio/caf\u{e9}.md", other));
        assert!(index.held_by_other("curio/new.md", other));
        // Two claimants of one path both count, and leave one by one.
        index.record(other, None, "curio/new.md");
        assert!(index.held_by_other("curio/new.md", CurioId::new()));
        index.record(legacy, Some("curio/new.md"), "curio/elsewhere.md");
        assert!(index.held_by_other("curio/new.md", legacy));
        assert!(!index.held_by_other("curio/new.md", other));
    }
}
