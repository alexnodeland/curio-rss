//! Upgrade tests against databases written by every released Curio.
//!
//! Each `fixtures/db/vX.Y.Z.db` was produced by that release tag's own
//! storage code (generator: `fixtures/db/generate.rs`), so these tests
//! prove that today's build opens what users actually have on disk:
//! the pre-migration backup is written (when there is anything to
//! migrate), every row survives, the FTS index still matches its content
//! table, and the intents the old build staged (but never emitted) still
//! decode and replay into the event log.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

mod common;

use curio_core::CoreHandle;
use curio_core::model::FeedStatus;
use curio_core::storage::Storage;

/// Row counts the generator writes — identical for every release.
const EXPECTED: &[(&str, i64)] = &[
    ("feeds", 3),
    ("articles", 10),
    ("articles_fts", 10),
    ("article_state", 6),
    ("tags", 2),
    ("article_tags", 3),
    ("fetch_log", 3),
    ("settings", 1),
    ("event_intents", 10),
];

fn fixture(version: &str) -> PathBuf {
    common::manifest_dir()
        .join("../../fixtures/db")
        .join(format!("{version}.db"))
}

/// Copies a fixture into a fresh profile directory as `curio.db` — the
/// committed file itself is never opened read-write.
fn profile_with(version: &str) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("curio.db");
    std::fs::copy(fixture(version), &db).unwrap();
    (dir, db)
}

fn raw_counts(path: &Path) -> Vec<(&'static str, i64)> {
    let conn = rusqlite::Connection::open(path).unwrap();
    EXPECTED
        .iter()
        .map(|(table, _)| {
            let n = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{table}\""), [], |row| {
                    row.get(0)
                })
                .unwrap();
            (*table, n)
        })
        .collect()
}

fn user_version(path: &Path) -> i64 {
    rusqlite::Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn assert_upgrades(version: &str, written_at_schema: i64) {
    let (dir, db) = profile_with(version);
    assert_eq!(
        user_version(&db),
        written_at_schema,
        "{version}: fixture schema"
    );
    assert_eq!(raw_counts(&db), EXPECTED, "{version}: fixture contents");

    let storage = Storage::open(&db).unwrap();
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version(),
        "{version}: upgraded to the current schema"
    );

    // The backup holds the release's database exactly as it was. A
    // release already at today's schema has nothing to migrate, so it is
    // opened in place with no backup — until the next migration lands,
    // when the same fixture becomes an upgrade case and the backup check
    // applies to it too.
    assert!(
        written_at_schema <= Storage::schema_version(),
        "{version}: fixture schema {written_at_schema} is newer than this build's"
    );
    let migrated = written_at_schema < Storage::schema_version();
    let backup = dir
        .path()
        .join(format!("curio.db.pre-v{}.bak", Storage::schema_version()));
    if migrated {
        assert_eq!(
            user_version(&backup),
            written_at_schema,
            "{version}: backup"
        );
        assert_eq!(raw_counts(&backup), EXPECTED, "{version}: backup contents");
    } else {
        assert!(!backup.exists(), "{version}: no backup without a migration");
    }

    // Integrity: page-level and the FTS external-content mapping.
    storage.integrity_check().unwrap();
    storage.fts_integrity_check().unwrap();

    // Every row survived, read back through today's repo code.
    assert_eq!(raw_counts(&db), EXPECTED, "{version}: upgraded contents");
    assert_eq!(storage.count_articles().unwrap(), 10);
    let feeds = storage.list_feeds().unwrap();
    assert_eq!(feeds.len(), 3);
    let rust = feeds
        .iter()
        .find(|f| f.url == "https://rust.example/feed.xml")
        .unwrap();
    assert_eq!(rust.tags, ["tech", "tech/rust"]);
    assert_eq!(rust.etag.as_deref(), Some("\"etag-1\""));
    let news = feeds
        .iter()
        .find(|f| f.url == "https://news.example/atom")
        .unwrap();
    assert_eq!(news.title.as_deref(), Some("Daily News"));
    assert_eq!(news.status, FeedStatus::Paused);
    assert_eq!(storage.search_articles("borrowck", 50).unwrap().len(), 4);
    assert_eq!(storage.search_articles("zebra", 50).unwrap().len(), 4);
    assert_eq!(storage.search_articles("quixotic", 50).unwrap().len(), 1);
    assert_eq!(storage.list_tags().unwrap(), ["keep", "rust"]);
    assert_eq!(
        storage.get_setting("fixture.probe").unwrap().as_deref(),
        Some("survives-upgrade")
    );
    // The staged intents the old build left behind all still decode.
    assert_eq!(storage.pending_intents().unwrap().len(), 10);
    assert_eq!(storage.quarantined_intent_count().unwrap(), 0);
    drop(storage);

    // Re-opening the now-current database is a no-op: no new backup.
    drop(Storage::open(&db).unwrap());
    let backups = std::fs::read_dir(dir.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".bak")
        })
        .count();
    assert_eq!(
        backups,
        usize::from(migrated),
        "{version}: one backup per migration, none on re-open"
    );
}

/// The whole engine boots on the release's profile and replays the
/// intents it left staged into the event log.
fn assert_engine_replays(version: &str) {
    let (dir, _db) = profile_with(version);
    let core = CoreHandle::open(dir.path()).unwrap();
    assert_eq!(
        core.pending_intent_count().unwrap(),
        0,
        "{version}: replayed"
    );
    drop(core);
    let events_dir = dir.path().join(".curio").join("events");
    let lines: usize = std::fs::read_dir(&events_dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "jsonl"))
        .map(|p| std::fs::read_to_string(p).unwrap().lines().count())
        .sum();
    assert_eq!(lines, 10, "{version}: one log line per staged intent");
}

#[test]
fn upgrades_a_v0_1_0_database() {
    assert_upgrades("v0.1.0", 5);
    assert_engine_replays("v0.1.0");
}

#[test]
fn upgrades_a_v0_2_0_database() {
    assert_upgrades("v0.2.0", 5);
    assert_engine_replays("v0.2.0");
}

#[test]
fn upgrades_a_v0_3_0_database() {
    assert_upgrades("v0.3.0", 5);
    assert_engine_replays("v0.3.0");
}

#[test]
fn upgrades_a_v0_4_0_database() {
    assert_upgrades("v0.4.0", 6);
    assert_engine_replays("v0.4.0");
}

/// v0.6.0 (v0.5.0 was never released) shipped schema 10, today's: it opens
/// in place with no backup, and becomes an upgrade case with the next
/// migration.
#[test]
fn opens_a_v0_6_0_database() {
    assert_upgrades("v0.6.0", 10);
    assert_engine_replays("v0.6.0");
}
