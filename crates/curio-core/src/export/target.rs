//! Where an export writes: the occupant of each candidate note path (a
//! blank note reads as vacant), the target decision for recorded and
//! unrecorded articles, the lock-free idempotency hit, and the
//! deterministic candidate paths.

use std::path::Path;

use icu_normalizer::ComposingNormalizerBorrowed;

use curio_types::{Checksum, CurioId, ManifestEntry};

use super::slug::slugify;
use super::write::contained_path;
use super::{
    ExportDisposition, ExportError, ExportInput, ExportOutcome, load_manifest,
    manifest_conflict_copies, note,
};
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
pub(super) fn unrecorded_target(
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
pub(super) fn unlocked_idempotency_hit(
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
        let err =
            unique_note_path(dir.path(), &curio_types::ManifestV1::new(), &input).unwrap_err();
        assert!(matches!(err, ExportError::PathsTaken { .. }), "{err}");
    }
}
