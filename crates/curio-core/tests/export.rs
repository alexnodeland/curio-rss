//! Contract-level integration tests for the note exporter: idempotency
//! on `(curio_id, checksum)`, managed-region surgery that preserves
//! user content byte-for-byte, canonical manifest form, and note-first
//! write ordering artifacts.

#![allow(clippy::unwrap_used)]

mod common;

use std::path::Path;

use curio_core::export::{
    ExportDisposition, ExportInput, export_note, load_manifest, region_checksum, stage_export_note,
};
use curio_types::{CurioId, Destination, MANAGED_REGION_BEGIN_V1, MANAGED_REGION_END_V1};

fn destination(root: &Path) -> Destination {
    Destination {
        name: "vault".parse().unwrap(),
        root: root.to_path_buf(),
    }
}

fn input(curio_id: CurioId, title: &str, markdown: &str) -> ExportInput {
    ExportInput {
        curio_id,
        title: title.to_owned(),
        source: "https://example.com/article".to_owned(),
        feed: Some("https://example.com/feed.xml".to_owned()),
        feed_title: Some("Example Blog".to_owned()),
        author: Some("Jane Doe".to_owned()),
        published: Some("2026-07-01T12:00:00Z".parse().unwrap()),
        saved: "2026-07-03T09:15:00.123Z".parse().unwrap(),
        tags: vec!["rust".to_owned(), "databases".to_owned()],
        lang: Some("en".to_owned()),
        word_count: Some(42),
        markdown: markdown.to_owned(),
    }
}

#[test]
fn first_export_creates_note_and_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();

    let outcome = export_note(&dest, &input(id, "Hello, World!", "# Hello\n\nBody text.")).unwrap();
    assert_eq!(outcome.disposition, ExportDisposition::Created);
    assert_eq!(outcome.path, "curio/hello-world.md");
    assert_eq!(outcome.checksum, region_checksum("# Hello\n\nBody text."));

    let note = std::fs::read_to_string(dir.path().join(&outcome.path)).unwrap();
    assert!(note.starts_with("---\nschema: curio.frontmatter.v1\n"));
    assert!(note.contains(&format!("curio_id: {id}")));
    assert!(note.contains("title: Hello, World!"));
    assert!(note.contains(&format!("checksum: {}", outcome.checksum)));
    assert!(note.contains(
        "<!-- curio:managed:begin v1 -->\n# Hello\n\nBody text.\n<!-- curio:managed:end -->"
    ));

    let manifest = load_manifest(dir.path()).unwrap();
    let entry = manifest.notes.get(&id).unwrap();
    assert_eq!(entry.path, outcome.path);
    assert_eq!(entry.checksum, outcome.checksum);

    // Canonical manifest form: 2-space indent, one entry per line.
    let raw = std::fs::read_to_string(dir.path().join(".curio/manifest.json")).unwrap();
    assert!(raw.starts_with("{\n  \"schema\": \"curio.manifest.v1\",\n  \"notes\": {\n"));
    assert_eq!(
        raw.lines()
            .filter(|line| line.contains("\"path\":"))
            .count(),
        1
    );

    // Contract: the event log is gitignored in every destination too —
    // a destination is often a git repo (Obsidian vault).
    let gitignore = std::fs::read_to_string(dir.path().join(".curio/.gitignore")).unwrap();
    assert!(gitignore.contains("events/"));
}

#[test]
fn export_is_idempotent_on_curio_id_and_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    let article = input(id, "Stable", "same body");

    let first = export_note(&dest, &article).unwrap();
    let note_path = dir.path().join(&first.path);
    let bytes_after_first = std::fs::read(&note_path).unwrap();
    let manifest_after_first = std::fs::read(dir.path().join(".curio/manifest.json")).unwrap();

    let second = export_note(&dest, &article).unwrap();
    assert_eq!(second.disposition, ExportDisposition::Unchanged);
    assert_eq!(second.path, first.path);
    assert_eq!(std::fs::read(&note_path).unwrap(), bytes_after_first);
    assert_eq!(
        std::fs::read(dir.path().join(".curio/manifest.json")).unwrap(),
        manifest_after_first,
        "unchanged export must not rewrite the manifest"
    );
}

#[test]
fn changed_content_updates_in_place() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();

    let first = export_note(&dest, &input(id, "Evolving", "v1 body")).unwrap();
    let second = export_note(&dest, &input(id, "Evolving", "v2 body")).unwrap();

    assert_eq!(second.disposition, ExportDisposition::Updated);
    assert_eq!(second.path, first.path, "identity keeps its path");
    assert_ne!(second.checksum, first.checksum, "change token moved");

    let note = std::fs::read_to_string(dir.path().join(&second.path)).unwrap();
    assert!(note.contains("v2 body"));
    assert!(!note.contains("v1 body"));

    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes.get(&id).unwrap().checksum, second.checksum);
    assert_eq!(manifest.notes.len(), 1);
}

#[test]
fn re_export_preserves_user_content_and_unknown_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();

    let first = export_note(&dest, &input(id, "Annotated", "original body")).unwrap();
    let note_path = dir.path().join(&first.path);

    // The user (or the KP companion) hand-edits the note: unknown
    // frontmatter keys, a preamble above the region, companion text below.
    let note = std::fs::read_to_string(&note_path).unwrap();
    let edited = note
        .replace(
            "---\n\n<!-- curio:managed:begin v1 -->",
            "kp_enriched: true\nrating: 5\n---\nMy preamble stays.\n\n<!-- curio:managed:begin v1 -->",
        )
        .replace(
            "<!-- curio:managed:end -->\n",
            "<!-- curio:managed:end -->\n\n## Companion notes\n\nThese lines are sacred.\n",
        );
    std::fs::write(&note_path, &edited).unwrap();

    // Re-export with changed content AND changed machine metadata.
    let mut updated = input(id, "Annotated (revised)", "replacement body");
    updated.tags.push("annotated".to_owned());
    let outcome = export_note(&dest, &updated).unwrap();
    assert_eq!(outcome.disposition, ExportDisposition::Updated);

    let rewritten = std::fs::read_to_string(&note_path).unwrap();
    // Managed surfaces replaced…
    assert!(rewritten.contains("title: Annotated (revised)"));
    assert!(rewritten.contains("replacement body"));
    assert!(!rewritten.contains("original body"));
    // …user content preserved byte-for-byte…
    assert!(rewritten.contains("My preamble stays.\n\n<!-- curio:managed:begin v1 -->"));
    assert!(rewritten.ends_with(
        "<!-- curio:managed:end -->\n\n## Companion notes\n\nThese lines are sacred.\n"
    ));
    // …and unknown frontmatter keys survive.
    assert!(rewritten.contains("kp_enriched: true"));
    assert!(rewritten.contains("rating: 5"));
}

#[test]
fn distinct_articles_with_the_same_title_get_distinct_paths() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let a = CurioId::new();
    let b = CurioId::new();

    let first = export_note(&dest, &input(a, "Same Title", "body a")).unwrap();
    let second = export_note(&dest, &input(b, "Same Title", "body b")).unwrap();

    assert_ne!(first.path, second.path);
    assert!(second.path.starts_with("curio/same-title-"));
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes.len(), 2);
}

#[test]
fn manifest_keys_are_sorted_and_git_mergeable() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    for i in 0..4 {
        export_note(&dest, &input(CurioId::new(), &format!("Note {i}"), "x")).unwrap();
    }
    let raw = std::fs::read_to_string(dir.path().join(".curio/manifest.json")).unwrap();
    let entry_keys: Vec<&str> = raw
        .lines()
        .filter(|line| line.contains("\"path\":"))
        .map(str::trim_start)
        .collect();
    let mut sorted = entry_keys.clone();
    sorted.sort_unstable();
    assert_eq!(entry_keys.len(), 4);
    // UUIDv7 is time-ordered, so insertion order == sorted order here;
    // the real assertion is the canonical reserialization equality:
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(raw, manifest.to_canonical_json());
    assert_eq!(entry_keys, sorted);
}

/// The staged flow exists so the facade can put the event intent between
/// the note write and the manifest write: after staging, the note bytes
/// are durable but the manifest is untouched; `commit()` lands it.
#[test]
fn staged_export_holds_the_manifest_back_until_commit() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();

    let staged = stage_export_note(&dest, &input(id, "Staged", "body")).unwrap();
    assert_eq!(staged.outcome().disposition, ExportDisposition::Created);
    let note_path = dir.path().join(&staged.outcome().path);
    assert!(note_path.is_file(), "note is written at stage time");
    assert!(
        load_manifest(dir.path()).unwrap().notes.is_empty(),
        "manifest write must wait for commit"
    );

    let outcome = staged.commit().unwrap();
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes.get(&id).unwrap().checksum, outcome.checksum);

    // A staged-but-never-committed export (crash before the manifest
    // write) re-converges: the next export finds the article's own note,
    // re-runs as Updated (so an intent the crash lost is staged again —
    // never suppressed as Unchanged) and heals the manifest.
    let id2 = CurioId::new();
    drop(stage_export_note(&dest, &input(id2, "Crashy", "body")).unwrap());
    assert!(!load_manifest(dir.path()).unwrap().notes.contains_key(&id2));
    let retried = export_note(&dest, &input(id2, "Crashy", "body")).unwrap();
    assert_eq!(retried.disposition, ExportDisposition::Updated);
    assert!(load_manifest(dir.path()).unwrap().notes.contains_key(&id2));
}

#[test]
fn a_deleted_note_is_healed_without_a_new_event_disposition() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    let article = input(id, "Healed", "body");

    let first = export_note(&dest, &article).unwrap();
    std::fs::remove_file(dir.path().join(&first.path)).unwrap();

    let second = export_note(&dest, &article).unwrap();
    assert_eq!(
        second.disposition,
        ExportDisposition::Unchanged,
        "the event stream already told this story"
    );
    assert!(dir.path().join(&second.path).is_file(), "note restored");
}

/// Feed content can carry the literal marker bytes (a code fence quoting
/// Curio's own contract survives sanitize + htmd verbatim). Unneutralized,
/// the first export would embed a fake end marker inside the region; the
/// next re-export's first-match split would truncate there, fold the
/// attacker's tail into the "user-owned" suffix forever, and the
/// frontmatter checksum would stop matching the bytes a consumer delimits
/// between the markers.
#[test]
fn marker_literals_in_the_body_cannot_break_the_managed_region() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    let hostile = format!(
        "quoting the contract:\n\n```\n{MANAGED_REGION_END_V1}\nINJECTED\n{MANAGED_REGION_BEGIN_V1}\n```"
    );

    let first = export_note(&dest, &input(id, "Hostile", &hostile)).unwrap();
    let note_path = dir.path().join(&first.path);
    let note = std::fs::read_to_string(&note_path).unwrap();
    // Exactly one real marker pair — the smuggled literals are defanged.
    assert_eq!(note.matches(MANAGED_REGION_BEGIN_V1).count(), 1);
    assert_eq!(note.matches(MANAGED_REGION_END_V1).count(), 1);
    assert!(note.contains("INJECTED"), "content itself survives");

    // The checksum covers exactly the on-disk region bytes: a consumer
    // hashing what it finds between the markers reproduces it.
    let begin = note.find(MANAGED_REGION_BEGIN_V1).unwrap() + MANAGED_REGION_BEGIN_V1.len();
    let end = note.find(MANAGED_REGION_END_V1).unwrap();
    let region = note[begin..end]
        .strip_prefix('\n')
        .unwrap()
        .strip_suffix('\n')
        .unwrap();
    assert_eq!(region_checksum(region), first.checksum);

    // Re-export is a clean idempotency hit, not a NoteParse or a
    // region truncation.
    let second = export_note(&dest, &input(id, "Hostile", &hostile)).unwrap();
    assert_eq!(second.disposition, ExportDisposition::Unchanged);

    // User companion text below the region survives an update that still
    // carries hostile markers — nothing accretes, nothing is truncated.
    let annotated = note.replace(
        "<!-- curio:managed:end -->\n",
        "<!-- curio:managed:end -->\n\n## My notes\n\nSacred.\n",
    );
    std::fs::write(&note_path, &annotated).unwrap();
    let updated_body = format!("{hostile}\n\nrevised");
    let third = export_note(&dest, &input(id, "Hostile", &updated_body)).unwrap();
    assert_eq!(third.disposition, ExportDisposition::Updated);
    let rewritten = std::fs::read_to_string(&note_path).unwrap();
    assert_eq!(rewritten.matches(MANAGED_REGION_END_V1).count(), 1);
    assert!(rewritten.ends_with("<!-- curio:managed:end -->\n\n## My notes\n\nSacred.\n"));
    assert!(rewritten.contains("revised"));
}

/// Lexical containment alone is fooled by a symlinked subdirectory
/// planted inside the destination root: `<root>/curio -> /elsewhere`
/// would redirect the atomic write outside the root. The write path
/// canonicalizes and re-asserts containment — the guarantee documented
/// on `Destination::root`.
#[test]
#[cfg(unix)]
fn a_symlinked_subdirectory_cannot_redirect_writes_outside_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), dir.path().join("curio")).unwrap();

    let dest = destination(dir.path());
    let err = export_note(&dest, &input(CurioId::new(), "Escape", "body")).unwrap_err();
    assert!(
        matches!(err, curio_core::export::ExportError::Containment { .. }),
        "expected containment refusal, got: {err}"
    );
    assert_eq!(
        std::fs::read_dir(outside.path()).unwrap().count(),
        0,
        "nothing may land outside the destination root"
    );
}

#[test]
fn hostile_titles_cannot_escape_the_destination() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let outcome = export_note(
        &dest,
        &input(CurioId::new(), "../../../../etc/passwd", "body"),
    )
    .unwrap();
    assert_eq!(outcome.path, "curio/etc-passwd.md");
    assert!(dir.path().join(&outcome.path).is_file());
}

/// A power loss under the old fsync-less write could leave the note
/// zero-length (or NUL-filled) while the manifest already recorded its
/// checksum; the idempotency fast path must not take that on trust.
#[test]
fn a_note_truncated_to_zero_bytes_is_repaired_on_re_export() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    let article = input(id, "Truncated", "# Body\n\nkept");

    let first = export_note(&dest, &article).unwrap();
    let note_path = dir.path().join(&first.path);
    let original = std::fs::read_to_string(&note_path).unwrap();

    for blank in ["", "\0\0\0\0\0\0\0\0"] {
        std::fs::write(&note_path, blank).unwrap();
        let again = export_note(&dest, &article).unwrap();
        assert_eq!(
            again.disposition,
            ExportDisposition::Unchanged,
            "a repair is not a new event"
        );
        assert_eq!(again.path, first.path);
        assert_eq!(
            std::fs::read_to_string(&note_path).unwrap(),
            original,
            "{blank:?} must be rewritten to the exported note"
        );
    }

    // A blank note is rewritten fresh (not refused as unparseable) when
    // the content changed meanwhile, too.
    std::fs::write(&note_path, "").unwrap();
    let updated = export_note(&dest, &input(id, "Truncated", "revised")).unwrap();
    assert_eq!(updated.disposition, ExportDisposition::Updated);
    assert!(
        std::fs::read_to_string(&note_path)
            .unwrap()
            .contains("revised")
    );
}

/// A non-blank note that no longer parses carries bytes Curio cannot
/// account for: the fast path reports it instead of claiming the export
/// is intact, and never rewrites it.
#[test]
fn an_unparseable_note_is_reported_not_silently_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let article = input(CurioId::new(), "Garbled", "body");

    let first = export_note(&dest, &article).unwrap();
    let note_path = dir.path().join(&first.path);
    std::fs::write(&note_path, "my own notes, markers gone\n").unwrap();

    let err = export_note(&dest, &article).unwrap_err();
    assert!(
        matches!(err, curio_core::export::ExportError::NoteParse { .. }),
        "{err}"
    );
    assert_eq!(
        std::fs::read_to_string(&note_path).unwrap(),
        "my own notes, markers gone\n"
    );
}

/// Directories squatting on note names through the public export path.
/// The ownership check sees them before any write (the injected rename
/// failure itself is `export::tests::a_failed_rename_leaves_no_temp_file`):
/// one squatter is skipped for the next candidate; with every candidate
/// taken the export fails — and either way no temp file is left for a
/// sync tool to pick up, and a failed export records nothing.
#[test]
fn squatted_note_names_leave_no_temp_file_and_no_manifest_entry() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    std::fs::create_dir_all(dir.path().join("curio/squatted.md/inside")).unwrap();

    let skipped = export_note(&dest, &input(CurioId::new(), "Squatted", "body")).unwrap();
    assert!(
        skipped.path.starts_with("curio/squatted-"),
        "{}",
        skipped.path
    );
    assert!(dir.path().join("curio/squatted.md/inside").is_dir());
    assert_eq!(tmp_leftovers(dir.path()), Vec::<String>::new());

    let id = CurioId::new();
    let full = id.to_string();
    for name in [
        "blocked",
        &format!("blocked-{}", &full[..8]),
        &format!("blocked-{full}"),
    ] {
        std::fs::create_dir_all(dir.path().join(format!("curio/{name}.md/inside"))).unwrap();
    }
    let err = export_note(&dest, &input(id, "Blocked", "body")).unwrap_err();
    assert!(
        matches!(err, curio_core::export::ExportError::PathsTaken { .. }),
        "{err}"
    );
    assert_eq!(tmp_leftovers(dir.path()), Vec::<String>::new());
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes.len(), 1);
    assert!(!manifest.notes.contains_key(&id));
}

/// NFC and NFD spellings of one title name the same file on APFS/HFS+,
/// so they must map to the same slug: the second article then takes the
/// deduplicated path instead of overwriting the first one's note.
#[test]
fn nfc_and_nfd_titles_share_a_slug_so_neither_overwrites_the_other() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let nfc = export_note(&dest, &input(CurioId::new(), "Caf\u{e9}", "first")).unwrap();
    let nfd = export_note(&dest, &input(CurioId::new(), "Cafe\u{301}", "second")).unwrap();

    assert_eq!(nfc.path, "curio/caf\u{e9}.md");
    assert_ne!(nfd.path, nfc.path);
    assert!(nfd.path.starts_with("curio/caf\u{e9}-"), "{}", nfd.path);
    let first = std::fs::read_to_string(dir.path().join(&nfc.path)).unwrap();
    assert!(first.contains("first") && !first.contains("second"));
}

#[test]
fn long_cjk_titles_fit_name_max_even_with_the_dedup_suffix() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let title = "長い題名".repeat(50); // 200 chars, 600 bytes
    let a = export_note(&dest, &input(CurioId::new(), &title, "a")).unwrap();
    let b = export_note(&dest, &input(CurioId::new(), &title, "b")).unwrap();
    for outcome in [&a, &b] {
        let name = outcome.path.strip_prefix("curio/").unwrap();
        assert!(name.len() <= 255, "{} bytes: {name}", name.len());
        assert!(dir.path().join(&outcome.path).is_file());
    }
    assert_ne!(a.path, b.path);
}

#[test]
fn windows_reserved_titles_never_produce_a_device_name() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    for title in ["CON", "nul", "Com1", "LPT9"] {
        let outcome = export_note(&dest, &input(CurioId::new(), title, "body")).unwrap();
        assert_eq!(
            outcome.path,
            format!("curio/{}-note.md", title.to_lowercase())
        );
    }
}

/// A manifest written before slugs were NFC-normalized can still hold an
/// NFD path (`cafe\u{301}.md`) that names the same file on APFS/HFS+ as
/// the NFC path a new "Café" article now slugs to. The manifest
/// comparison is normalization-aware, so the new article takes the
/// deduplicated path instead of the old article's note.
#[test]
fn a_legacy_nfd_manifest_path_still_owns_its_name() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let old_id = CurioId::new();
    let old = export_note(&dest, &input(old_id, "Caf\u{e9}", "old")).unwrap();

    // Rewind to the pre-normalization layout: NFD path in the manifest
    // and on disk.
    let legacy = "curio/cafe\u{301}.md";
    std::fs::rename(dir.path().join(&old.path), dir.path().join(legacy)).unwrap();
    let mut manifest = load_manifest(dir.path()).unwrap();
    manifest.notes.get_mut(&old_id).unwrap().path = legacy.to_owned();
    curio_core::export::write_manifest(dir.path(), &manifest).unwrap();

    let new = export_note(&dest, &input(CurioId::new(), "Caf\u{e9}", "new")).unwrap();
    assert_ne!(new.path, "curio/caf\u{e9}.md");
    assert!(new.path.starts_with("curio/caf\u{e9}-"), "{}", new.path);
    let kept = std::fs::read_to_string(dir.path().join(legacy)).unwrap();
    assert!(kept.contains(&format!("curio_id: {old_id}")));
    assert!(kept.contains("old") && !kept.contains("new"));
}

/// The manifest is not the only witness of ownership: when it has lost
/// an entry (deleted, or rolled back by a sync tool) — or when two
/// byte-distinct names fold to one file — the note on disk still names
/// its article. A new article never takes over a note that parses with
/// another `curio_id`; the owner itself re-adopts its note in place.
#[test]
fn a_note_owned_by_another_article_on_disk_is_never_taken_over() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let owner = CurioId::new();
    let first = export_note(&dest, &input(owner, "Shared Title", "owner's")).unwrap();
    assert_eq!(first.path, "curio/shared-title.md");
    std::fs::remove_file(dir.path().join(".curio/manifest.json")).unwrap();

    let newcomer =
        export_note(&dest, &input(CurioId::new(), "Shared Title", "newcomer's")).unwrap();
    assert_eq!(newcomer.disposition, ExportDisposition::Created);
    assert_ne!(newcomer.path, first.path);
    let kept = std::fs::read_to_string(dir.path().join(&first.path)).unwrap();
    assert!(kept.contains(&format!("curio_id: {owner}")));
    assert!(kept.contains("owner's") && !kept.contains("newcomer's"));

    // The owner, exported again, re-adopts its own note at its own path.
    let readopted = export_note(&dest, &input(owner, "Shared Title", "owner's")).unwrap();
    assert_eq!(readopted.path, first.path);
}

/// A crash can leave stale, non-UTF-8 blocks where a note was: that is
/// an unparseable note (the user should look at it), not an I/O fault.
#[test]
fn a_non_utf8_note_is_reported_as_unparseable() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let article = input(CurioId::new(), "Stale Blocks", "body");
    let first = export_note(&dest, &article).unwrap();
    std::fs::write(dir.path().join(&first.path), b"---\n\xff\xfe\x00garbage").unwrap();

    let err = export_note(&dest, &article).unwrap_err();
    assert!(
        matches!(err, curio_core::export::ExportError::NoteParse { .. }),
        "{err}"
    );
}

// ---- #116: a lost, stale or forked manifest must never cost a note ----

fn remove_manifest(root: &Path) {
    std::fs::remove_file(root.join(".curio/manifest.json")).unwrap();
}

fn tmp_leftovers(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let entry = entry.unwrap();
            if entry.file_type().unwrap().is_dir() {
                stack.push(entry.path());
            } else if entry.file_name().to_string_lossy().contains(".curio-tmp") {
                found.push(entry.path().display().to_string());
            }
        }
    }
    found
}

/// Acceptance: manifest deleted → exporting a same-titled second article
/// never changes the first note's bytes.
#[test]
fn a_lost_manifest_never_lets_a_same_titled_article_overwrite_a_note() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let first = export_note(&dest, &input(CurioId::new(), "Weekly links", "first")).unwrap();
    let first_abs = dir.path().join(&first.path);
    // The user annotates the first note.
    let annotated = format!(
        "{}\n## My notes\n\nOnly about the first article.\n",
        std::fs::read_to_string(&first_abs).unwrap()
    );
    std::fs::write(&first_abs, &annotated).unwrap();

    remove_manifest(dir.path());
    let second_id = CurioId::new();
    let second = export_note(&dest, &input(second_id, "Weekly links", "second")).unwrap();

    assert_eq!(second.disposition, ExportDisposition::Created);
    assert_ne!(
        second.path, first.path,
        "the slug path belongs to the first article"
    );
    assert_eq!(
        std::fs::read_to_string(&first_abs).unwrap(),
        annotated,
        "the first note's bytes must not change"
    );
    let second_note = std::fs::read_to_string(dir.path().join(&second.path)).unwrap();
    assert!(second_note.contains(&format!("curio_id: {second_id}")));
    assert!(second_note.contains("\nsecond\n"));
}

/// A lost manifest + a re-export of the *same* article reconciles onto its
/// own note (ownership proven by the note's `curio_id`), keeping the
/// user's companion text — no duplicate note.
#[test]
fn a_lost_manifest_reconciles_an_article_onto_its_own_note() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    let first = export_note(&dest, &input(id, "Mine", "v1")).unwrap();
    let abs = dir.path().join(&first.path);
    let annotated = format!("{}\nkept\n", std::fs::read_to_string(&abs).unwrap());
    std::fs::write(&abs, &annotated).unwrap();

    remove_manifest(dir.path());
    let again = export_note(&dest, &input(id, "Mine", "v2")).unwrap();
    assert_eq!(again.path, first.path);
    assert_eq!(
        again.disposition,
        ExportDisposition::Updated,
        "the article was saved before: never a second article.saved"
    );
    assert_eq!(
        load_manifest(dir.path()).unwrap().notes[&id].path,
        first.path
    );
    let note = std::fs::read_to_string(&abs).unwrap();
    assert!(
        note.contains("\nv2\n") && note.ends_with("\nkept\n"),
        "{note}"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("curio")).unwrap().count(),
        1
    );
}

/// A lost manifest + an *unchanged* re-export: the note's bytes stay as
/// they are, the export is `Updated` (never a second `article.saved`, and
/// never a silent `Unchanged` that could swallow a crashed export's
/// event), and the manifest records the note again.
#[test]
fn a_lost_manifest_and_an_unchanged_article_keep_the_note_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    let first = export_note(&dest, &input(id, "Mine", "same")).unwrap();
    let abs = dir.path().join(&first.path);
    let annotated = format!("{}\nkept\n", std::fs::read_to_string(&abs).unwrap());
    std::fs::write(&abs, &annotated).unwrap();

    remove_manifest(dir.path());
    let again = export_note(&dest, &input(id, "Mine", "same")).unwrap();
    assert_eq!(again.disposition, ExportDisposition::Updated);
    assert_eq!(again.path, first.path);
    assert_eq!(std::fs::read_to_string(&abs).unwrap(), annotated);
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes[&id].path, first.path);
    assert_eq!(manifest.notes[&id].checksum, first.checksum);

    // A note whose region was edited by hand is healed.
    remove_manifest(dir.path());
    let tampered = annotated.replace("\nsame\n", "\nhand edit\n");
    assert_ne!(tampered, annotated);
    std::fs::write(&abs, &tampered).unwrap();
    let healed = export_note(&dest, &input(id, "Mine", "same")).unwrap();
    assert_eq!(healed.disposition, ExportDisposition::Updated);
    assert!(std::fs::read_to_string(&abs).unwrap().contains("\nsame\n"));
}

/// The idempotency hit writes nothing, so it takes no lock: it creates
/// no lock file, and works on a destination it could not lock.
#[test]
fn an_unchanged_export_takes_no_lock() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    export_note(&dest, &input(id, "Steady", "body")).unwrap();
    let lock = dir.path().join(".curio/manifest.lock");
    std::fs::remove_file(&lock).unwrap();

    let again = export_note(&dest, &input(id, "Steady", "body")).unwrap();
    assert_eq!(again.disposition, ExportDisposition::Unchanged);
    assert!(!lock.exists(), "the idempotency hit must not take the lock");

    // A changed article does take it.
    export_note(&dest, &input(id, "Steady", "new body")).unwrap();
    assert!(lock.is_file());
}

/// A stale manifest (restored from git, or reset by a sync tool) that
/// records a path now holding *another* article's note: the export moves
/// to a free path instead of taking that note over.
#[test]
fn a_stale_manifest_path_held_by_another_article_is_never_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let a = CurioId::new();
    let b = CurioId::new();
    let a_first = export_note(&dest, &input(a, "Shared", "a v1")).unwrap();
    let stale = std::fs::read(dir.path().join(".curio/manifest.json")).unwrap();

    // A's note goes away and B is exported into the freed slug path.
    std::fs::remove_file(dir.path().join(&a_first.path)).unwrap();
    remove_manifest(dir.path());
    let b_out = export_note(&dest, &input(b, "Shared", "b v1")).unwrap();
    assert_eq!(b_out.path, a_first.path);
    let b_bytes = std::fs::read(dir.path().join(&b_out.path)).unwrap();

    // The stale manifest comes back (it only knows A, at that path).
    std::fs::write(dir.path().join(".curio/manifest.json"), stale).unwrap();
    let a_again = export_note(&dest, &input(a, "Shared", "a v2")).unwrap();

    assert_eq!(a_again.disposition, ExportDisposition::Updated);
    assert_ne!(a_again.path, b_out.path);
    assert_eq!(
        std::fs::read(dir.path().join(&b_out.path)).unwrap(),
        b_bytes
    );
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes[&a].path, a_again.path);
}

/// A file at the slug path that is not a Curio note at all (the user's
/// own `curio/reading-list.md`) is skipped, not rewritten or refused.
#[test]
fn a_foreign_file_at_the_slug_path_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("curio")).unwrap();
    let mine = dir.path().join("curio/reading-list.md");
    std::fs::write(&mine, "# My own list\n").unwrap();

    let out = export_note(
        &destination(dir.path()),
        &input(CurioId::new(), "Reading list", "body"),
    )
    .unwrap();
    assert_ne!(out.path, "curio/reading-list.md");
    assert!(out.path.starts_with("curio/reading-list-"));
    assert_eq!(std::fs::read_to_string(&mine).unwrap(), "# My own list\n");
}

/// Acceptance: 8 threads exporting 8 articles at once → 8 manifest
/// entries, no leftover `.curio-tmp` files.
#[test]
fn concurrent_exports_to_one_destination_lose_no_manifest_entries() {
    let dir = tempfile::tempdir().unwrap();
    let barrier = std::sync::Barrier::new(8);
    let ids: Vec<CurioId> = (0..8).map(|_| CurioId::new()).collect();
    std::thread::scope(|scope| {
        for (i, id) in ids.iter().enumerate() {
            let barrier = &barrier;
            let dest = destination(dir.path());
            scope.spawn(move || {
                barrier.wait();
                export_note(&dest, &input(*id, &format!("Article {i}"), "body")).unwrap();
            });
        }
    });

    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes.len(), 8);
    for id in &ids {
        assert!(dir.path().join(&manifest.notes[id].path).is_file());
    }
    assert_eq!(tmp_leftovers(dir.path()), Vec::<String>::new());
    assert!(dir.path().join(".curio/manifest.lock").is_file());
    let gitignore = std::fs::read_to_string(dir.path().join(".curio/.gitignore")).unwrap();
    assert!(gitignore.lines().any(|l| l == "manifest.lock"));
}

/// Acceptance: removing the destination dir then exporting →
/// `DestinationMissing`, no recreated tree.
#[test]
fn a_removed_destination_is_destination_missing_and_never_recreated() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("vault");
    std::fs::create_dir(&root).unwrap();
    let dest = destination(&root);
    export_note(&dest, &input(CurioId::new(), "Before", "body")).unwrap();

    std::fs::remove_dir_all(&root).unwrap();
    let err = export_note(&dest, &input(CurioId::new(), "After", "body")).unwrap_err();
    assert!(
        matches!(
            err,
            curio_core::export::ExportError::DestinationMissing { .. }
        ),
        "{err}"
    );
    assert!(!root.exists(), "the destination tree must not be recreated");
}

/// Acceptance: the conflict-copy fixture is detected — and its entries
/// are merged (union, newest `exported_at` wins), then the copies go.
#[test]
fn manifest_conflict_copies_are_detected_and_merged() {
    let fixture = common::manifest_dir().join("../../fixtures/export/manifest-conflict");
    let dir = tempfile::tempdir().unwrap();
    let curio = dir.path().join(".curio");
    std::fs::create_dir_all(&curio).unwrap();
    for name in [
        "manifest.json",
        "manifest (conflicted copy).json",
        "manifest 2.json",
    ] {
        std::fs::copy(fixture.join(name), curio.join(name)).unwrap();
    }

    let copies = curio_core::export::manifest_conflict_copies(dir.path()).unwrap();
    assert_eq!(
        copies,
        vec![
            curio.join("manifest (conflicted copy).json"),
            curio.join("manifest 2.json"),
        ]
    );

    let fresh = CurioId::new();
    export_note(&destination(dir.path()), &input(fresh, "Fresh", "body")).unwrap();

    let manifest = load_manifest(dir.path()).unwrap();
    let id = |s: &str| -> CurioId { s.parse().unwrap() };
    let a = &manifest.notes[&id("0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6a")];
    assert_eq!(
        a.exported_at,
        "2026-07-05T09:00:00.000Z".parse().unwrap(),
        "the newer record from the conflict copy wins"
    );
    let b = &manifest.notes[&id("0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6b")];
    assert_eq!(b.path, "curio/article-b.md", "an older copy never wins");
    assert!(
        manifest
            .notes
            .contains_key(&id("0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6c"))
    );
    assert!(
        manifest
            .notes
            .contains_key(&id("0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d"))
    );
    assert!(manifest.notes.contains_key(&fresh));
    assert_eq!(manifest.notes.len(), 5);
    assert_eq!(
        curio_core::export::manifest_conflict_copies(dir.path()).unwrap(),
        Vec::<std::path::PathBuf>::new(),
        "merged copies are removed once the merged manifest is durable"
    );
}

/// Two devices each exported a different same-titled article to one path
/// and the sync tool forked the manifest: the merged manifest keeps only
/// the article whose note actually sits at that path; the other is
/// re-homed by its next export instead of falsely claiming the note.
#[test]
fn a_merged_copy_never_leaves_two_articles_claiming_one_path() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let a = CurioId::new();
    let b = CurioId::new();
    let a_out = export_note(&dest, &input(a, "Weekly links", "a")).unwrap();
    // Device 2 had exported B to the same path; its manifest is the copy.
    let mut copy = curio_types::ManifestV1::new();
    copy.notes.insert(
        b,
        curio_types::ManifestEntry {
            path: a_out.path.clone(),
            checksum: region_checksum("b"),
            exported_at: "2026-07-02T09:00:00.000Z".parse().unwrap(),
        },
    );
    std::fs::write(
        dir.path().join(".curio/manifest (conflicted copy).json"),
        copy.to_canonical_json(),
    )
    .unwrap();
    let a_bytes = std::fs::read(dir.path().join(&a_out.path)).unwrap();

    // Any export merges the copy (here: an unrelated article).
    export_note(&dest, &input(CurioId::new(), "Other", "x")).unwrap();
    let manifest = load_manifest(dir.path()).unwrap();
    assert_eq!(manifest.notes[&a].path, a_out.path);
    assert!(
        !manifest.notes.contains_key(&b),
        "B does not own the note at that path"
    );

    // B's next export finds a path of its own; A's note is untouched.
    let b_out = export_note(&dest, &input(b, "Weekly links", "b")).unwrap();
    assert_ne!(b_out.path, a_out.path);
    assert_eq!(b_out.disposition, ExportDisposition::Created);
    assert_eq!(
        std::fs::read(dir.path().join(&a_out.path)).unwrap(),
        a_bytes
    );
}

/// A user's own `manifest.backup.json` is not a sync conflict copy: it is
/// neither merged nor deleted.
#[test]
fn a_user_backup_of_the_manifest_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    export_note(&dest, &input(CurioId::new(), "First", "body")).unwrap();
    let backup = dir.path().join(".curio/manifest.backup.json");
    std::fs::copy(dir.path().join(".curio/manifest.json"), &backup).unwrap();
    let before = std::fs::read(&backup).unwrap();

    export_note(&dest, &input(CurioId::new(), "Second", "body")).unwrap();
    assert_eq!(std::fs::read(&backup).unwrap(), before);
    assert_eq!(load_manifest(dir.path()).unwrap().notes.len(), 2);
}

/// The merge persists even when the export itself is an idempotency hit.
#[test]
fn conflict_copies_are_merged_on_an_unchanged_export_too() {
    let dir = tempfile::tempdir().unwrap();
    let dest = destination(dir.path());
    let id = CurioId::new();
    export_note(&dest, &input(id, "Steady", "body")).unwrap();

    let other: CurioId = "0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6c".parse().unwrap();
    let mut copy = curio_types::ManifestV1::new();
    copy.notes.insert(
        other,
        curio_types::ManifestEntry {
            path: "curio/other.md".to_owned(),
            checksum: region_checksum("other"),
            exported_at: "2026-07-02T09:00:00.000Z".parse().unwrap(),
        },
    );
    let copy_path = dir.path().join(".curio/manifest 2.json");
    std::fs::write(&copy_path, copy.to_canonical_json()).unwrap();

    let again = export_note(&dest, &input(id, "Steady", "body")).unwrap();
    assert_eq!(again.disposition, ExportDisposition::Unchanged);
    assert!(
        load_manifest(dir.path())
            .unwrap()
            .notes
            .contains_key(&other)
    );
    assert!(!copy_path.exists());
}
