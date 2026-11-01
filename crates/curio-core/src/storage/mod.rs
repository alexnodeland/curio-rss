//! WAL `SQLite` behind a single-writer, pooled-reader facade.
//!
//! The concurrency model matches `SQLite`'s own: exactly **one writer** — a
//! dedicated thread owning the sole read-write connection, fed typed jobs
//! over an mpsc channel with oneshot replies — plus an N-connection
//! read-only pool for queries. The sketch's `Mutex<Connection>` pattern
//! (which serialized every reader behind every writer and blocked async
//! executors) is banned by design; nothing above this module ever sees a
//! `rusqlite` type or a line of SQL.
//!
//! Schema lives in the private `migrations` module; every query goes through
//! the typed repo functions in the private `repo` module.

mod backup;
mod migrations;
mod netfs;
mod open;
mod pool;
mod repo;
mod writer;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use backup::{PreMigrationBackup, pre_migration_backups};
pub use netfs::{
    ALLOW_NETWORK_FS_ENV, NetworkFsKind, detect_network_fs, network_fs_override_from_env,
};
pub use open::OpenError;
pub(crate) use repo::SettingsTx;
pub use repo::{FETCH_LOG_KEEP, FeedSchedule, ListArticles, PendingIntent, UpsertOutcome};
pub(crate) use writer::panic_message;

use pool::ReadPool;
use writer::WriterHandle;

/// Default number of read-only connections in the pool.
pub const DEFAULT_READ_POOL_SIZE: usize = 4;

/// How long any connection waits on another connection's lock (in this
/// process or another head sharing the profile) before the statement
/// fails with [`StorageError::Busy`].
pub(crate) const BUSY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Storage-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// An underlying `SQLite` error.
    #[error("sqlite error: {0}")]
    Sqlite(#[source] rusqlite::Error),
    /// The database stayed locked by another connection for longer than
    /// the busy timeout — typically another Curio process (the desktop
    /// app and a CLI run) writing to the same profile. Transient: the
    /// same operation succeeds once the other writer finishes.
    #[error(
        "the profile database is busy — another Curio process (the desktop app or a CLI run) \
         is writing to it; try again in a moment"
    )]
    Busy(#[source] rusqlite::Error),
    /// A write collided with a UNIQUE constraint — the row already exists
    /// (subscribing to a feed URL that is already subscribed, for one).
    #[error("{entity} already exists")]
    Duplicate {
        /// What already exists (`"feed"`, `"article"`, `"tag"`, …; `"row"`
        /// for a table with no friendlier name).
        entity: &'static str,
        /// The constraint failure `SQLite` reported.
        #[source]
        source: rusqlite::Error,
    },
    /// The disk holding the profile is full (`SQLITE_FULL`). Nothing was
    /// written; the operation succeeds once space is freed.
    #[error("the disk holding the profile database is full — free some space and try again")]
    DiskFull(#[source] rusqlite::Error),
    /// The profile database cannot be written (`SQLITE_READONLY` /
    /// `SQLITE_PERM`): the file, its folder or the volume is read-only, or
    /// permissions changed under the running app.
    #[error(
        "the profile database is read-only — check the permissions of the profile folder \
         and the volume it is on"
    )]
    ReadOnly(#[source] rusqlite::Error),
    /// `SQLite` found the database file corrupt mid-session
    /// (`SQLITE_CORRUPT` / `SQLITE_NOTADB`) — distinct from
    /// [`StorageError::Corrupt`], a single stored value that failed to
    /// parse.
    #[error("the profile database is corrupt: {0}")]
    DatabaseCorrupt(#[source] rusqlite::Error),
    /// The writer thread has shut down and can no longer accept
    /// commands.
    #[error("the storage writer thread is gone")]
    WriterGone,
    /// A write job panicked. The writer rolled back whatever the job left
    /// open and keeps serving later writes; only this one failed.
    #[error("a storage write panicked: {message}")]
    JobPanicked {
        /// The panic message.
        message: String,
    },
    /// No pooled read connection freed up in time — every connection is
    /// stuck behind a long-running read.
    #[error("no database read connection became free within {waited_ms} ms")]
    PoolTimeout {
        /// How long the read waited.
        waited_ms: u64,
    },
    /// The database refused to enter WAL mode.
    #[error("could not enable WAL journal mode (got {mode:?})")]
    WalUnavailable {
        /// The journal mode the database reported instead.
        mode: String,
    },
    /// A row referenced by the caller does not exist.
    #[error("{entity} not found")]
    NotFound {
        /// What was looked up.
        entity: &'static str,
    },
    /// A stored value failed to parse back into its domain type — the
    /// database has been tampered with or corrupted.
    #[error("corrupt value in {column}: {message}")]
    Corrupt {
        /// The column holding the bad value.
        column: &'static str,
        /// Parse-failure detail.
        message: String,
    },
    /// `PRAGMA integrity_check` reported problems.
    #[error("database integrity check failed: {detail}")]
    IntegrityFailed {
        /// The first problem `SQLite` reported.
        detail: String,
    },
    /// Event-envelope (de)serialization failed.
    #[error("event envelope serialization: {0}")]
    Envelope(#[from] serde_json::Error),
    /// A tag failed validation. The published contracts require tags to
    /// be non-empty (`minLength: 1` in both `curio.events.v1` and
    /// `curio.frontmatter.v1`), so empty or whitespace-only tags are
    /// refused at the producer boundary — an accepted one would emit
    /// events and frontmatter that fail the schemas Curio publishes.
    #[error("invalid tag: tags must contain non-whitespace characters")]
    InvalidTag,
}

impl From<rusqlite::Error> for StorageError {
    /// Sorts a `SQLite` failure by its (extended) result code into the
    /// class a head can act on: lock contention (`SQLITE_BUSY` /
    /// `SQLITE_LOCKED` after the busy timeout) → [`StorageError::Busy`], a
    /// UNIQUE / PRIMARY KEY collision → [`StorageError::Duplicate`],
    /// `SQLITE_FULL` → [`StorageError::DiskFull`], `SQLITE_READONLY` /
    /// `SQLITE_PERM` → [`StorageError::ReadOnly`], `SQLITE_CORRUPT` /
    /// `SQLITE_NOTADB` → [`StorageError::DatabaseCorrupt`]. Every other
    /// failure stays [`StorageError::Sqlite`] — including
    /// `SQLITE_CORRUPT_VTAB`, a search index out of step with its content
    /// table: a rebuild fixes that, so it is no corrupt database.
    fn from(err: rusqlite::Error) -> Self {
        use rusqlite::ErrorCode;
        let (code, extended) = match &err {
            rusqlite::Error::SqliteFailure(failure, _) => (failure.code, failure.extended_code),
            _ => return Self::Sqlite(err),
        };
        match code {
            ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked => Self::Busy(err),
            ErrorCode::ConstraintViolation
                if matches!(
                    extended,
                    rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                        | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY
                ) =>
            {
                Self::Duplicate {
                    entity: duplicate_entity(&err),
                    source: err,
                }
            }
            ErrorCode::DiskFull => Self::DiskFull(err),
            ErrorCode::ReadOnly | ErrorCode::PermissionDenied => Self::ReadOnly(err),
            ErrorCode::DatabaseCorrupt if extended == rusqlite::ffi::SQLITE_CORRUPT_VTAB => {
                Self::Sqlite(err)
            }
            ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => Self::DatabaseCorrupt(err),
            _ => Self::Sqlite(err),
        }
    }
}

/// Names what a UNIQUE / PRIMARY KEY failure collided with, from the table
/// `SQLite` names in its message (`UNIQUE constraint failed: feeds.url`).
fn duplicate_entity(err: &rusqlite::Error) -> &'static str {
    let rusqlite::Error::SqliteFailure(_, Some(message)) = err else {
        return "row";
    };
    let table = message
        .rsplit(": ")
        .next()
        .and_then(|columns| columns.split('.').next())
        .unwrap_or_default();
    match table {
        "feeds" => "feed",
        "articles" => "article",
        "article_state" => "article state",
        "tags" => "tag",
        "article_tags" => "article tag",
        "settings" => "setting",
        _ => "row",
    }
}

impl StorageError {
    /// Whether this is transient contention with another connection or
    /// process ([`StorageError::Busy`]) — retrying later can succeed.
    #[must_use]
    pub fn is_busy(&self) -> bool {
        matches!(self, Self::Busy(_))
    }

    /// The raw `SQLite` error behind a class that came straight from one
    /// (see the `From<rusqlite::Error>` conversion); `Err(self)` for the
    /// storage layer's own failures.
    pub(crate) fn into_sqlite(self) -> Result<rusqlite::Error, Self> {
        match self {
            Self::Sqlite(source)
            | Self::Busy(source)
            | Self::DiskFull(source)
            | Self::ReadOnly(source)
            | Self::DatabaseCorrupt(source)
            | Self::Duplicate { source, .. } => Ok(source),
            other => Err(other),
        }
    }
}

/// The storage handle: one writer thread + a read-only pool over a single
/// WAL-mode `SQLite` database.
///
/// `Storage` is `Send + Sync`; heads share it behind an `Arc`. Dropping the
/// last handle shuts the writer down cleanly (remaining queued jobs finish
/// first).
#[derive(Debug)]
pub struct Storage {
    writer: WriterHandle,
    pool: ReadPool,
    path: PathBuf,
}

impl Storage {
    /// Opens (creating if absent) the database at `path`, applies pending
    /// migrations — after writing a pre-migration backup next to the file
    /// (`curio.db.pre-v{N}.bak`; it and the highest-numbered other are kept) — and spins up the
    /// writer thread plus a read pool of [`DEFAULT_READ_POOL_SIZE`]
    /// connections.
    ///
    /// # Errors
    ///
    /// A typed [`OpenError`]: [`OpenError::Corrupt`] for a file that is
    /// not a (healthy) `SQLite` database, [`OpenError::TooNew`] for one
    /// written by a newer build, [`OpenError::Locked`] when another
    /// process holds it, [`OpenError::Io`] for OS-level failures,
    /// [`OpenError::Backup`] / [`OpenError::MigrationFailed`] for a
    /// refused or rolled-back upgrade.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OpenError> {
        Self::open_with_pool_size(path, DEFAULT_READ_POOL_SIZE)
    }

    /// [`Storage::open`] with an explicit read-pool size (min 1).
    ///
    /// # Errors
    ///
    /// See [`Storage::open`].
    pub fn open_with_pool_size(
        path: impl AsRef<Path>,
        read_pool_size: usize,
    ) -> Result<Self, OpenError> {
        let path = path.as_ref().to_path_buf();
        let classify = open::classifier(&path);
        let mut conn = Connection::open(&path).map_err(|e| classify(e.into()))?;
        // Before the first statement, so a lock held elsewhere is waited
        // out (then reported as `OpenError::Locked`) rather than failing
        // at once.
        conn.busy_timeout(BUSY_TIMEOUT)
            .map_err(|e| classify(e.into()))?;
        // The first statement that reads the file header — a garbage file
        // fails here with SQLITE_NOTADB (→ `Corrupt`) — and a pure read: a
        // too-new database is refused before the WAL switch below could
        // rewrite its header or create sidecar files.
        migrations::refuse_too_new(&conn, &path)?;
        configure_write_connection(&conn).map_err(&classify)?;
        migrations::apply(&mut conn, &path)?;
        let writer = WriterHandle::spawn(conn);
        let pool = ReadPool::open(&path, read_pool_size.max(1), pool::DEFAULT_POOL_TIMEOUT)
            .map_err(classify)?;
        Ok(Self { writer, pool, path })
    }

    /// The filesystem path of the database.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The schema version this build writes and expects.
    #[must_use]
    pub fn schema_version() -> i64 {
        migrations::SCHEMA_VERSION
    }

    /// Runs a mutation on the writer thread and waits for its typed reply.
    pub(crate) fn write<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    {
        self.writer.run(f)
    }

    /// Runs a query on a pooled read-only connection.
    pub(crate) fn read<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        self.pool.with(f)
    }

    /// The `user_version` of the opened database (equal to
    /// [`Storage::schema_version`] after a successful open).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn db_schema_version(&self) -> Result<i64, StorageError> {
        self.read(migrations::current_version)
    }

    /// A deterministic dump of `sqlite_master` — the fresh-database golden
    /// schema test pins the migrated schema to a committed snapshot, and
    /// diagnostics can print it.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn schema_dump(&self) -> Result<String, StorageError> {
        self.read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT type, name, COALESCE(sql, '<internal>') FROM sqlite_master \
                 ORDER BY type, name",
            )?;
            let entries = stmt
                .query_map([], |row| {
                    Ok(format!(
                        "-- {} {}\n{}",
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(entries.join("\n\n"))
        })
    }

    /// Rebuilds the database file (`VACUUM`). Article rowids are an
    /// explicit `INTEGER PRIMARY KEY`, so the FTS external-content mapping
    /// survives — regression-tested against the sketch's corruption bug.
    ///
    /// # Errors
    ///
    /// Returns an error if the underlying `VACUUM` fails.
    pub fn vacuum(&self) -> Result<(), StorageError> {
        self.write(|conn| {
            conn.execute("VACUUM", [])?;
            Ok(())
        })
    }

    /// Runs `PRAGMA integrity_check` over the whole database — the
    /// doctor-surface health probe.
    ///
    /// # Errors
    ///
    /// [`StorageError::IntegrityFailed`] with `SQLite`'s first finding if
    /// the database is damaged; [`StorageError::Sqlite`] if the pragma
    /// itself cannot run.
    pub fn integrity_check(&self) -> Result<(), StorageError> {
        self.read(|conn| {
            let verdict: String = conn.query_row("PRAGMA integrity_check", [], |row| row.get(0))?;
            if verdict.eq_ignore_ascii_case("ok") {
                Ok(())
            } else {
                Err(StorageError::IntegrityFailed { detail: verdict })
            }
        })
    }

    /// Asks FTS5 to verify the search index against the `articles` content
    /// table; errors if they have desynchronized.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] with `SQLITE_CORRUPT_VTAB` if the
    /// index no longer matches the content table (never
    /// [`StorageError::DatabaseCorrupt`]: a desynced index is rebuildable).
    pub fn fts_integrity_check(&self) -> Result<(), StorageError> {
        self.write(|conn| {
            conn.execute(
                "INSERT INTO articles_fts(articles_fts, rank) VALUES ('integrity-check', 1)",
                [],
            )?;
            Ok(())
        })
    }
}

/// Pragmas for the single read-write connection.
///
/// `synchronous = FULL` is deliberate (#118): every transaction that
/// stages an event intent must be **durable** before the emitter fsyncs
/// that event to the JSONL log. Under WAL + `NORMAL`, a commit that has
/// returned can still roll back on power loss — the log would then hold
/// an `article.starred` whose state change (and intent) the database
/// forgot, and no negation would ever follow. `FULL` fsyncs the WAL on
/// every commit; the writer is single-threaded and commits are batched
/// per operation, so the cost is one fsync per mutation.
fn configure_write_connection(conn: &Connection) -> Result<(), StorageError> {
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::WalUnavailable { mode });
    }
    conn.pragma_update(None, "synchronous", "FULL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::time::Duration;

    use super::*;

    /// A real lock conflict: another connection (standing in for another
    /// process) holds an exclusive transaction past our busy timeout.
    #[test]
    fn lock_contention_is_the_typed_busy_error() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().join("curio.db")).unwrap();
        let other = Connection::open(storage.path()).unwrap();
        other.execute_batch("BEGIN EXCLUSIVE").unwrap();

        let ours = Connection::open(storage.path()).unwrap();
        ours.busy_timeout(Duration::from_millis(20)).unwrap();
        let err = StorageError::from(ours.execute_batch("BEGIN IMMEDIATE; COMMIT;").unwrap_err());
        assert!(err.is_busy(), "{err:?}");
        assert!(err.to_string().contains("another Curio process"), "{err}");
        assert!(std::error::Error::source(&err).is_some());
        other.execute_batch("COMMIT").unwrap();
    }

    #[test]
    fn other_sqlite_failures_stay_generic() {
        let conn = Connection::open_in_memory().unwrap();
        let err = StorageError::from(conn.execute_batch("NOT SQL").unwrap_err());
        assert!(matches!(err, StorageError::Sqlite(_)), "{err:?}");
        assert!(!err.is_busy());
        let locked = StorageError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_LOCKED),
            None,
        ));
        assert!(locked.is_busy(), "SQLITE_LOCKED is contention too");
    }

    fn sqlite(code: i32) -> StorageError {
        StorageError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ))
    }

    #[test]
    fn result_codes_map_to_distinct_classes() {
        assert!(matches!(
            sqlite(rusqlite::ffi::SQLITE_BUSY),
            StorageError::Busy(_)
        ));
        assert!(matches!(
            sqlite(rusqlite::ffi::SQLITE_FULL),
            StorageError::DiskFull(_)
        ));
        assert!(!sqlite(rusqlite::ffi::SQLITE_FULL).is_busy());
        for code in [rusqlite::ffi::SQLITE_READONLY, rusqlite::ffi::SQLITE_PERM] {
            assert!(
                matches!(sqlite(code), StorageError::ReadOnly(_)),
                "code {code}"
            );
        }
        for code in [rusqlite::ffi::SQLITE_CORRUPT, rusqlite::ffi::SQLITE_NOTADB] {
            assert!(
                matches!(sqlite(code), StorageError::DatabaseCorrupt(_)),
                "code {code}"
            );
        }
        // A desynced FTS index is rebuildable — no corrupt database.
        assert!(matches!(
            sqlite(rusqlite::ffi::SQLITE_CORRUPT_VTAB),
            StorageError::Sqlite(_)
        ));
        // Another constraint (CHECK, NOT NULL, FOREIGN KEY) is no duplicate.
        assert!(matches!(
            sqlite(rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY),
            StorageError::Sqlite(_)
        ));
        assert!(matches!(
            sqlite(rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY),
            StorageError::Duplicate { entity: "row", .. }
        ));
        // Every class keeps the raw error as its source.
        for err in [
            sqlite(rusqlite::ffi::SQLITE_FULL),
            sqlite(rusqlite::ffi::SQLITE_READONLY),
            sqlite(rusqlite::ffi::SQLITE_CORRUPT),
        ] {
            assert!(std::error::Error::source(&err).is_some(), "{err:?}");
            assert!(err.into_sqlite().is_ok());
        }
        assert!(StorageError::WriterGone.into_sqlite().is_err());
    }

    /// A real `SQLITE_FULL`: the database may not grow past its page cap.
    #[test]
    fn a_full_database_is_disk_full() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (x BLOB)").unwrap();
        let pages: i64 = conn
            .query_row("PRAGMA page_count", [], |row| row.get(0))
            .unwrap();
        conn.pragma_update(None, "max_page_count", pages).unwrap();
        let err = StorageError::from(
            conn.execute("INSERT INTO t VALUES (zeroblob(1000000))", [])
                .unwrap_err(),
        );
        assert!(matches!(err, StorageError::DiskFull(_)), "{err:?}");
        assert!(err.to_string().contains("disk"), "{err}");
    }

    /// A real UNIQUE collision names the table's entity.
    #[test]
    fn a_unique_collision_is_a_duplicate_of_its_entity() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE feeds (url TEXT UNIQUE); CREATE TABLE odd (k TEXT UNIQUE);
             INSERT INTO feeds VALUES ('a'); INSERT INTO odd VALUES ('a');",
        )
        .unwrap();
        let feed = StorageError::from(
            conn.execute("INSERT INTO feeds VALUES ('a')", [])
                .unwrap_err(),
        );
        assert!(
            matches!(feed, StorageError::Duplicate { entity: "feed", .. }),
            "{feed:?}"
        );
        assert_eq!(feed.to_string(), "feed already exists");
        let other = StorageError::from(
            conn.execute("INSERT INTO odd VALUES ('a')", [])
                .unwrap_err(),
        );
        assert!(
            matches!(other, StorageError::Duplicate { entity: "row", .. }),
            "{other:?}"
        );
    }

    #[test]
    fn a_panicking_write_through_storage_is_followed_by_a_successful_write() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().join("curio.db")).unwrap();
        let err = storage
            .write(|_conn| -> Result<(), StorageError> { panic!("bug in a mutation") })
            .unwrap_err();
        assert!(matches!(err, StorageError::JobPanicked { .. }), "{err:?}");
        assert!(err.to_string().contains("bug in a mutation"), "{err}");
        // The writer survived: a real mutation goes through.
        storage.vacuum().unwrap();
        assert_eq!(
            storage.db_schema_version().unwrap(),
            Storage::schema_version()
        );
    }

    /// #118: the writer's commits must be durable before the emitter
    /// fsyncs their events — `synchronous = FULL` (2), not `NORMAL` (1).
    #[test]
    fn the_writer_connection_commits_durably() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().join("curio.db")).unwrap();
        let level: i64 = storage
            .write(|conn| Ok(conn.query_row("PRAGMA synchronous", [], |row| row.get(0))?))
            .unwrap();
        assert_eq!(level, 2, "PRAGMA synchronous on the writer must be FULL");
        let mode: String = storage
            .write(|conn| Ok(conn.query_row("PRAGMA journal_mode", [], |row| row.get(0))?))
            .unwrap();
        assert_eq!(mode, "wal");
    }
}
