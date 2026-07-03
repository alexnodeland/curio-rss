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
//! Schema lives in [`migrations`]; every query goes through the typed repo
//! functions in [`repo`].

mod migrations;
mod pool;
mod repo;
mod writer;

use std::path::{Path, PathBuf};

use rusqlite::Connection;

pub use repo::{ListArticles, PendingIntent, UpsertOutcome};

use pool::ReadPool;
use writer::WriterHandle;

/// Default number of read-only connections in the pool.
pub const DEFAULT_READ_POOL_SIZE: usize = 4;

/// Storage-layer errors.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// An underlying `SQLite` error.
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// The writer thread has shut down (or panicked) and can no longer
    /// accept commands.
    #[error("the storage writer thread is gone")]
    WriterGone,
    /// The database was written by a newer build of Curio.
    #[error("database schema version {found} is newer than this build supports ({supported})")]
    SchemaTooNew {
        /// The `user_version` found in the database.
        found: i64,
        /// The highest version this build knows.
        supported: i64,
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
    /// Event-envelope (de)serialization failed.
    #[error("event envelope serialization: {0}")]
    Envelope(#[from] serde_json::Error),
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
    /// migrations, and spins up the writer thread plus a read pool of
    /// [`DEFAULT_READ_POOL_SIZE`] connections.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or created, WAL
    /// mode cannot be enabled, a migration fails, or the database was
    /// written by a newer build.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, StorageError> {
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
    ) -> Result<Self, StorageError> {
        let path = path.as_ref().to_path_buf();
        let mut conn = Connection::open(&path)?;
        configure_write_connection(&conn)?;
        migrations::apply(&mut conn)?;
        let writer = WriterHandle::spawn(conn);
        let pool = ReadPool::open(&path, read_pool_size.max(1))?;
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

    /// Asks FTS5 to verify the search index against the `articles` content
    /// table; errors if they have desynchronized.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] with `SQLITE_CORRUPT_VTAB` if the
    /// index no longer matches the content table.
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
fn configure_write_connection(conn: &Connection) -> Result<(), StorageError> {
    let mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(StorageError::WalUnavailable { mode });
    }
    conn.busy_timeout(std::time::Duration::from_secs(5))?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Ok(())
}
