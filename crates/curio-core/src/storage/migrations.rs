//! Versioned, embedded, forward-only schema migrations.
//!
//! Each migration is a numbered SQL file under `migrations/`, embedded via
//! `include_str!`. The applied version is tracked in `SQLite`'s
//! `user_version` pragma. An upgrade is safe by construction:
//!
//! 1. an existing database is first snapshotted to a pre-migration backup
//!    (see the `backup` module) — no backup, no migration;
//! 2. every pending step then runs inside **one** `IMMEDIATE` transaction,
//!    with `user_version` bumped in that same transaction — so a failing
//!    step rolls the whole upgrade back and the database stays at the
//!    version it was opened at, never half-way.
//!
//! Running the set is idempotent: already-applied migrations are skipped,
//! and a database from a *newer* build is refused rather than guessed at.

use std::path::{Path, PathBuf};

use rusqlite::{Connection, TransactionBehavior};

use super::open::{OpenError, classifier};
use super::{StorageError, backup};

/// One embedded migration step.
#[derive(Debug, Clone, Copy)]
struct Migration {
    /// The `user_version` this migration brings the database to.
    version: i64,
    /// The SQL batch, embedded at compile time.
    sql: &'static str,
}

/// Every migration, in order. Version `N` lives at index `N - 1`.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: include_str!("migrations/0001_init.sql"),
    },
    Migration {
        version: 2,
        sql: include_str!("migrations/0002_feed_scoped_dedupe.sql"),
    },
    Migration {
        version: 3,
        sql: include_str!("migrations/0003_feed_tags.sql"),
    },
    Migration {
        version: 4,
        sql: include_str!("migrations/0004_article_lead_image.sql"),
    },
    Migration {
        version: 5,
        sql: include_str!("migrations/0005_feed_sort_order.sql"),
    },
    Migration {
        version: 6,
        sql: include_str!("migrations/0006_feed_full_text.sql"),
    },
    Migration {
        version: 7,
        sql: include_str!("migrations/0007_article_content_origin.sql"),
    },
    Migration {
        version: 8,
        sql: include_str!("migrations/0008_event_intents_dead.sql"),
    },
];

/// The schema version this build writes and expects.
pub(crate) const SCHEMA_VERSION: i64 = MIGRATIONS[MIGRATIONS.len() - 1].version;

/// Reads the database's current `user_version`.
pub(crate) fn current_version(conn: &Connection) -> Result<i64, StorageError> {
    let version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

/// Refuses a database written by a newer build before anything else
/// touches it: a read of the header's `user_version` only, so the file is
/// left exactly as found (no journal-mode switch, no `-wal`/`-shm`
/// sidecars). Also the first statement to read the header, so a garbage
/// file fails here as [`OpenError::Corrupt`].
pub(crate) fn refuse_too_new(conn: &Connection, db_path: &Path) -> Result<(), OpenError> {
    let found = current_version(conn).map_err(classifier(db_path))?;
    if found > SCHEMA_VERSION {
        return Err(OpenError::TooNew {
            found,
            supported: SCHEMA_VERSION,
        });
    }
    Ok(())
}

/// Applies every pending migration to the database at `db_path`, backing
/// it up first. Returns the backup's path if one was written (a fresh
/// database, or one already current, needs none).
pub(crate) fn apply(conn: &mut Connection, db_path: &Path) -> Result<Option<PathBuf>, OpenError> {
    apply_set(conn, db_path, MIGRATIONS)
}

/// [`apply`] over an explicit migration list (tests inject failing steps).
fn apply_set(
    conn: &mut Connection,
    db_path: &Path,
    migrations: &[Migration],
) -> Result<Option<PathBuf>, OpenError> {
    let classify = classifier(db_path);
    let target = migrations.last().map_or(0, |m| m.version);
    let current = current_version(conn).map_err(&classify)?;
    if current > target {
        return Err(OpenError::TooNew {
            found: current,
            supported: target,
        });
    }
    if current == target {
        return Ok(None);
    }
    // Version 0 is a database this build just created: nothing to lose.
    let backup = if current > 0 {
        Some(backup::write(conn, db_path, target)?)
    } else {
        None
    };
    let failed = |source: StorageError| match classify(source) {
        OpenError::Storage(source) => OpenError::MigrationFailed {
            from: current,
            to: target,
            backup: backup.clone(),
            source: Box::new(source),
        },
        typed => typed,
    };
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|e| failed(e.into()))?;
    // Re-read under the write lock: another process may have upgraded the
    // file between the backup and here.
    let locked_at = current_version(&tx).map_err(failed)?;
    if locked_at > target {
        return Err(OpenError::TooNew {
            found: locked_at,
            supported: target,
        });
    }
    for migration in migrations.iter().filter(|m| m.version > locked_at) {
        tx.execute_batch(migration.sql)
            .and_then(|()| tx.pragma_update(None, "user_version", migration.version))
            .map_err(|e| failed(e.into()))?;
        tracing::info!(version = migration.version, "applied schema migration");
    }
    tx.commit().map_err(|e| failed(e.into()))?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    // Test code: a failed unwrap is the assertion.
    #![allow(clippy::unwrap_used)]

    use super::*;

    const BROKEN: Migration = Migration {
        version: SCHEMA_VERSION + 1,
        sql: "CREATE TABLE injected_step (x); SELECT * FROM no_such_table;",
    };

    fn version(path: &Path) -> i64 {
        let conn = Connection::open(path).unwrap();
        current_version(&conn).unwrap()
    }

    fn article_count(path: &Path) -> i64 {
        let conn = Connection::open(path).unwrap();
        conn.query_row("SELECT COUNT(*) FROM articles", [], |r| r.get(0))
            .unwrap()
    }

    /// A database at schema v1 with one article row.
    fn v1_database(path: &Path) -> Connection {
        let mut conn = Connection::open(path).unwrap();
        apply_set(&mut conn, path, &MIGRATIONS[..1]).unwrap();
        conn.execute_batch(
            "INSERT INTO articles (curio_id, dedupe_key, title, source_url, saved_at, modified_at)
             VALUES ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d', 'guid:1', 'Kept',
                     'https://a.example/1', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn a_fresh_database_migrates_without_a_backup() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("curio.db");
        let mut conn = Connection::open(&path).unwrap();
        assert_eq!(apply(&mut conn, &path).unwrap(), None);
        assert_eq!(current_version(&conn).unwrap(), SCHEMA_VERSION);
        // Already current: a no-op, still no backup.
        assert_eq!(apply(&mut conn, &path).unwrap(), None);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn upgrading_writes_the_backup_before_user_version_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("curio.db");
        let mut conn = v1_database(&path);
        let backup = apply(&mut conn, &path).unwrap().unwrap();
        assert_eq!(
            backup,
            dir.path()
                .join(format!("curio.db.pre-v{SCHEMA_VERSION}.bak"))
        );
        // The backup holds the pre-upgrade state; the live file moved on.
        assert_eq!(version(&backup), 1);
        assert_eq!(article_count(&backup), 1);
        assert_eq!(current_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn a_failing_migration_rolls_back_every_step_and_the_backup_restores() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("curio.db");
        let mut conn = v1_database(&path);
        let mut set = MIGRATIONS.to_vec();
        set.push(BROKEN);

        let err = apply_set(&mut conn, &path, &set).unwrap_err();
        let OpenError::MigrationFailed {
            from,
            to,
            backup: Some(backup),
            ..
        } = err
        else {
            panic!("expected MigrationFailed with a backup, got {err:?}");
        };
        assert_eq!((from, to), (1, SCHEMA_VERSION + 1));
        // One transaction: steps 2..=N that *did* succeed rolled back too.
        assert_eq!(current_version(&conn).unwrap(), 1);
        let leftover: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'injected_step'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(leftover, 0);
        drop(conn);

        // Restore the backup over the live file; it opens at v1 with its
        // rows, and a good build then upgrades it normally.
        for sidecar in ["curio.db-wal", "curio.db-shm"] {
            let _ = std::fs::remove_file(dir.path().join(sidecar));
        }
        std::fs::copy(&backup, &path).unwrap();
        assert_eq!(version(&path), 1);
        assert_eq!(article_count(&path), 1);
        let mut conn = Connection::open(&path).unwrap();
        apply(&mut conn, &path).unwrap();
        assert_eq!(current_version(&conn).unwrap(), SCHEMA_VERSION);
        assert_eq!(article_count(&path), 1);
    }

    #[test]
    fn a_newer_database_is_refused_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("curio.db");
        let mut conn = Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        let err = apply(&mut conn, &path).unwrap_err();
        assert!(matches!(
            err,
            OpenError::TooNew { found, supported }
                if found == SCHEMA_VERSION + 1 && supported == SCHEMA_VERSION
        ));
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            1,
            "no backup"
        );
    }

    #[test]
    fn only_the_two_newest_backups_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("curio.db");
        let mut conn = v1_database(&path);
        // Step the database up one version at a time: v2, v3, v4 backups
        // are written in turn; the oldest is pruned.
        for upto in 2..=4 {
            apply_set(&mut conn, &path, &MIGRATIONS[..upto]).unwrap();
        }
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.starts_with("curio.db.pre-v"))
            .collect();
        names.sort();
        assert_eq!(names, ["curio.db.pre-v3.bak", "curio.db.pre-v4.bak"]);
    }
}
