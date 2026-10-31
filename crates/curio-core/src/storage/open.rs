//! Typed open failures.
//!
//! Opening is the one storage operation a head has to *act* on rather
//! than just report: a corrupt file wants a restore-from-backup offer, a
//! too-new schema wants "update Curio", a lock wants "quit the other
//! copy". [`OpenError`] gives each of those its own variant so heads match
//! on the failure class instead of parsing `SQLite` messages.

use std::path::{Path, PathBuf};

use rusqlite::ErrorCode;

use super::StorageError;

/// Why [`Storage::open`](super::Storage::open) refused or failed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OpenError {
    /// The file is not a `SQLite` database, or its pages are damaged.
    #[error("{} is not a readable Curio database (corrupt or not SQLite): {source}", path.display())]
    Corrupt {
        /// The database path.
        path: PathBuf,
        /// What `SQLite` reported.
        #[source]
        source: rusqlite::Error,
    },
    /// The database was written by a newer build of Curio. Nothing was
    /// changed; a newer build opens it.
    #[error(
        "database schema version {found} is newer than this build supports ({supported}) — update Curio to open it"
    )]
    TooNew {
        /// The `user_version` found in the database.
        found: i64,
        /// The highest version this build knows.
        supported: i64,
    },
    /// Another process holds the database lock past the busy timeout.
    #[error("{} is locked by another process — close other copies of Curio and retry", path.display())]
    Locked {
        /// The database path.
        path: PathBuf,
        /// What `SQLite` reported.
        #[source]
        source: rusqlite::Error,
    },
    /// The file could not be opened, read or written at the OS level
    /// (missing directory, permissions, read-only media, disk full).
    #[error("cannot open {}: {source}", path.display())]
    Io {
        /// The database path.
        path: PathBuf,
        /// What `SQLite` reported.
        #[source]
        source: rusqlite::Error,
    },
    /// The pre-migration backup could not be written, so no migration was
    /// attempted and the database is untouched.
    #[error("could not write the pre-migration backup {}: {detail} — the database was not migrated", path.display())]
    Backup {
        /// The backup path that was being written.
        path: PathBuf,
        /// What went wrong.
        detail: String,
    },
    /// A migration step failed. The whole upgrade ran in one transaction,
    /// so the database is still at `from`; `backup` is the pre-migration
    /// copy written before the attempt.
    #[error(
        "migrating the database from schema v{from} to v{to} failed and was rolled back{}: {source}",
        backup.as_ref().map(|b| format!(" (pre-migration backup: {})", b.display())).unwrap_or_default()
    )]
    MigrationFailed {
        /// The version the database is still at.
        from: i64,
        /// The version the upgrade was heading for.
        to: i64,
        /// The pre-migration backup, if one was needed (a fresh,
        /// version-0 database has nothing to back up).
        backup: Option<PathBuf>,
        /// The failing step's error.
        #[source]
        source: Box<StorageError>,
    },
    /// Any other storage failure during open (e.g. the file system
    /// refusing WAL mode).
    #[error(transparent)]
    Storage(StorageError),
}

/// Sorts a raw storage failure during open into its typed class. Errors
/// that fit no class stay [`OpenError::Storage`].
pub(crate) fn classify(path: &Path, error: StorageError) -> OpenError {
    let StorageError::Sqlite(source) = error else {
        return OpenError::Storage(error);
    };
    let path = path.to_path_buf();
    match source.sqlite_error_code() {
        Some(ErrorCode::NotADatabase | ErrorCode::DatabaseCorrupt) => {
            OpenError::Corrupt { path, source }
        }
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => {
            OpenError::Locked { path, source }
        }
        Some(
            ErrorCode::CannotOpen
            | ErrorCode::PermissionDenied
            | ErrorCode::ReadOnly
            | ErrorCode::DiskFull
            | ErrorCode::SystemIoFailure
            | ErrorCode::NoLargeFileSupport,
        ) => OpenError::Io { path, source },
        _ => OpenError::Storage(StorageError::Sqlite(source)),
    }
}

/// `classify`, as a `map_err` adapter.
pub(crate) fn classifier(path: &Path) -> impl Fn(StorageError) -> OpenError + '_ {
    move |error| classify(path, error)
}

#[cfg(test)]
mod tests {
    // Test code: a failed unwrap is the assertion.
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn sqlite(code: i32) -> StorageError {
        StorageError::Sqlite(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(code),
            None,
        ))
    }

    #[test]
    fn sqlite_codes_map_to_their_open_class() {
        let path = Path::new("/profile/curio.db");
        assert!(matches!(
            classify(path, sqlite(rusqlite::ffi::SQLITE_NOTADB)),
            OpenError::Corrupt { .. }
        ));
        assert!(matches!(
            classify(path, sqlite(rusqlite::ffi::SQLITE_CORRUPT)),
            OpenError::Corrupt { .. }
        ));
        assert!(matches!(
            classify(path, sqlite(rusqlite::ffi::SQLITE_BUSY)),
            OpenError::Locked { .. }
        ));
        assert!(matches!(
            classify(path, sqlite(rusqlite::ffi::SQLITE_LOCKED)),
            OpenError::Locked { .. }
        ));
        for code in [
            rusqlite::ffi::SQLITE_CANTOPEN,
            rusqlite::ffi::SQLITE_PERM,
            rusqlite::ffi::SQLITE_READONLY,
            rusqlite::ffi::SQLITE_FULL,
            rusqlite::ffi::SQLITE_IOERR,
        ] {
            assert!(
                matches!(classify(path, sqlite(code)), OpenError::Io { .. }),
                "code {code}"
            );
        }
        assert!(matches!(
            classify(path, sqlite(rusqlite::ffi::SQLITE_ERROR)),
            OpenError::Storage(StorageError::Sqlite(_))
        ));
        assert!(matches!(
            classify(path, StorageError::WriterGone),
            OpenError::Storage(StorageError::WriterGone)
        ));
    }

    #[test]
    fn messages_name_the_path_and_the_backup() {
        let path = Path::new("/profile/curio.db");
        let locked = classify(path, sqlite(rusqlite::ffi::SQLITE_BUSY));
        assert!(locked.to_string().contains("/profile/curio.db"));
        let failed = OpenError::MigrationFailed {
            from: 5,
            to: 7,
            backup: Some(PathBuf::from("/profile/curio.db.pre-v7.bak")),
            source: Box::new(StorageError::WriterGone),
        };
        let message = failed.to_string();
        assert!(message.contains("v5 to v7"), "{message}");
        assert!(message.contains("curio.db.pre-v7.bak"), "{message}");
        let fresh = OpenError::MigrationFailed {
            from: 0,
            to: 7,
            backup: None,
            source: Box::new(StorageError::WriterGone),
        };
        assert!(!fresh.to_string().contains("backup"));
    }
}
