//! Contract-level integration tests for the note exporter: idempotency
//! on `(curio_id, checksum)`, managed-region surgery that preserves
//! user content byte-for-byte, canonical manifest form, and note-first
//! write ordering artifacts.

#![allow(clippy::unwrap_used)]

use std::path::Path;

use curio_core::export::{
    ExportDisposition, ExportInput, export_note, load_manifest, region_checksum,
};
use curio_types::{CurioId, Destination};

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
