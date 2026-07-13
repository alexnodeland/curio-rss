//! Versioned, embedded, forward-only schema migrations.
//!
//! Each migration is a numbered SQL file under `migrations/`, embedded via
//! `include_str!` and applied inside its own transaction; the applied
//! version is tracked in `SQLite`'s `user_version` pragma (updated in the
//! same transaction, so a crashed migration leaves no trace). Running the
//! set is idempotent: already-applied migrations are skipped, and a
//! database from a *newer* build is refused rather than guessed at.

use rusqlite::Connection;

use super::StorageError;

/// One embedded migration step.
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
];

/// The schema version this build writes and expects.
pub(crate) const SCHEMA_VERSION: i64 = MIGRATIONS[MIGRATIONS.len() - 1].version;

/// Reads the database's current `user_version`.
pub(crate) fn current_version(conn: &Connection) -> Result<i64, StorageError> {
    let version = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    Ok(version)
}

/// Applies every pending migration. Idempotent; errors on a database whose
/// version is newer than this build supports.
pub(crate) fn apply(conn: &mut Connection) -> Result<(), StorageError> {
    let current = current_version(conn)?;
    if current > SCHEMA_VERSION {
        return Err(StorageError::SchemaTooNew {
            found: current,
            supported: SCHEMA_VERSION,
        });
    }
    for migration in MIGRATIONS {
        if migration.version <= current {
            continue;
        }
        let tx = conn.transaction()?;
        tx.execute_batch(migration.sql)?;
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
        tracing::info!(version = migration.version, "applied schema migration");
    }
    Ok(())
}
