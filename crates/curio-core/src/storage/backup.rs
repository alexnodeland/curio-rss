//! The automatic pre-migration backup.
//!
//! Before any migration touches an existing database, the open path
//! snapshots it with `VACUUM INTO` to `<db>.pre-v{N}.bak` next to the
//! database, where `N` is the schema version the upgrade is heading for.
//! The snapshot is a consistent, self-contained copy (no `-wal` sidecar),
//! written to a temporary name, fsynced, then renamed into place — so a
//! crash mid-backup never leaves a truncated `.bak` that looks complete.
//! Only the [`KEEP`] newest backups are kept.
//!
//! Restoring one by hand: quit Curio, delete `curio.db-wal` and
//! `curio.db-shm` if present, and copy the `.bak` over `curio.db`.

use std::path::{Path, PathBuf};

use rusqlite::Connection;

use super::OpenError;

/// How many pre-migration backups survive pruning.
pub(crate) const KEEP: usize = 2;

/// The backup path for an upgrade of `db_path` to schema version `to`.
pub(crate) fn backup_path(db_path: &Path, to: i64) -> PathBuf {
    let mut name = db_path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".pre-v{to}.bak"));
    db_path.with_file_name(name)
}

/// Snapshots the database behind `conn` (at `db_path`) ahead of an
/// upgrade to version `to`, then prunes older backups. Returns the
/// backup's path.
pub(crate) fn write(conn: &Connection, db_path: &Path, to: i64) -> Result<PathBuf, OpenError> {
    let path = backup_path(db_path, to);
    let fail = |detail: String| OpenError::Backup {
        path: path.clone(),
        detail,
    };
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    // VACUUM INTO refuses an existing target; a leftover temp file is a
    // previous attempt that crashed before its rename.
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(fail(format!("removing stale {}: {err}", tmp.display()))),
    }
    let tmp_str = tmp
        .to_str()
        .ok_or_else(|| fail("the profile path is not valid UTF-8".to_owned()))?;
    conn.execute("VACUUM INTO ?1", [tmp_str])
        .map_err(|err| fail(err.to_string()))?;
    // Write access: Windows' FlushFileBuffers refuses a read-only handle.
    std::fs::OpenOptions::new()
        .write(true)
        .open(&tmp)
        .and_then(|file| file.sync_all())
        .map_err(|err| fail(format!("syncing {}: {err}", tmp.display())))?;
    // Atomic replace: a retry after a failed migration overwrites the
    // earlier (identical-content) backup of the same upgrade.
    std::fs::rename(&tmp, &path).map_err(|err| fail(format!("renaming into place: {err}")))?;
    tracing::info!(backup = %path.display(), "wrote pre-migration database backup");
    prune(db_path);
    Ok(path)
}

/// A pre-migration backup found next to a database: what a head offers to
/// restore when the database itself will not open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreMigrationBackup {
    /// The backup file (`<db>.pre-v{N}.bak`).
    pub path: PathBuf,
    /// `N`: the schema version the upgrade that wrote it was heading for.
    pub upgrade_to: i64,
    /// The schema version stored *inside* the backup, read from its file
    /// header; `None` when the file is not a readable `SQLite` database.
    pub schema_version: Option<i64>,
}

impl PreMigrationBackup {
    /// Whether this build can open the backup: it is a `SQLite` file whose
    /// schema is not newer than
    /// [`Storage::schema_version`](super::Storage::schema_version).
    #[must_use]
    pub fn opens_in_this_build(&self) -> bool {
        self.schema_version
            .is_some_and(|version| version <= super::migrations::SCHEMA_VERSION)
    }
}

/// Every pre-migration backup of the database at `db_path`, newest upgrade
/// first. Reading a backup's schema version only reads its file header:
/// nothing is opened, locked or created next to it. An unreadable
/// directory lists as empty.
#[must_use]
pub fn pre_migration_backups(db_path: &Path) -> Vec<PreMigrationBackup> {
    list_by_version(db_path)
        .unwrap_or_default()
        .into_iter()
        .map(|(upgrade_to, path)| PreMigrationBackup {
            schema_version: header_user_version(&path),
            path,
            upgrade_to,
        })
        .collect()
}

/// The `user_version` field of a `SQLite` file header (a big-endian `u32`
/// at offset 60), or `None` when `path` does not start with the `SQLite`
/// magic string.
fn header_user_version(path: &Path) -> Option<i64> {
    use std::io::Read as _;

    let mut header = [0_u8; 64];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .ok()?;
    if !header.starts_with(b"SQLite format 3\0") {
        return None;
    }
    let field: [u8; 4] = header[60..64].try_into().ok()?;
    Some(i64::from(u32::from_be_bytes(field)))
}

/// The backups of `db_path` as `(version, path)`, highest version first.
fn list_by_version(db_path: &Path) -> std::io::Result<Vec<(i64, PathBuf)>> {
    let dir = match db_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let Some(stem) = db_path.file_name().and_then(|n| n.to_str()) else {
        return Ok(Vec::new());
    };
    let prefix = format!("{stem}.pre-v");
    let mut backups: Vec<(i64, PathBuf)> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let version = name
                .to_str()?
                .strip_prefix(&prefix)?
                .strip_suffix(".bak")?
                .parse::<i64>()
                .ok()?;
            Some((version, entry.path()))
        })
        .collect();
    backups.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    Ok(backups)
}

/// Deletes all but the [`KEEP`] highest-versioned backups of `db_path`.
/// Best-effort: a failure here only costs disk space, so it is logged,
/// never surfaced.
fn prune(db_path: &Path) {
    let backups = match list_by_version(db_path) {
        Ok(backups) => backups,
        Err(err) => {
            tracing::warn!(error = %err, "could not list backups for pruning");
            return;
        }
    };
    for (_, stale) in backups.into_iter().skip(KEEP) {
        if let Err(err) = std::fs::remove_file(&stale) {
            tracing::warn!(backup = %stale.display(), error = %err, "could not prune an old backup");
        }
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed unwrap is the assertion.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn backup_path_sits_next_to_the_database() {
        assert_eq!(
            backup_path(Path::new("/p/curio.db"), 7),
            PathBuf::from("/p/curio.db.pre-v7.bak")
        );
    }

    #[test]
    fn prune_keeps_the_two_highest_versions_and_ignores_strangers() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        for name in [
            "curio.db.pre-v3.bak",
            "curio.db.pre-v10.bak",
            "curio.db.pre-v6.bak",
            "curio.db.pre-vX.bak",
            "other.db.pre-v1.bak",
            "notes.txt",
        ] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        prune(&db);
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "curio.db.pre-v10.bak",
                "curio.db.pre-v6.bak",
                "curio.db.pre-vX.bak",
                "notes.txt",
                "other.db.pre-v1.bak",
            ]
        );
    }

    #[test]
    fn a_stale_temp_file_is_replaced_and_an_existing_backup_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
            .unwrap();
        std::fs::write(dir.path().join("curio.db.pre-v2.bak.tmp"), b"junk").unwrap();
        std::fs::write(dir.path().join("curio.db.pre-v2.bak"), b"old").unwrap();
        let path = write(&conn, &db, 2).unwrap();
        assert!(!dir.path().join("curio.db.pre-v2.bak.tmp").exists());
        let copy = Connection::open(&path).unwrap();
        let x: i64 = copy.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(x, 1);
    }

    #[test]
    fn an_unwritable_target_is_a_typed_backup_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x);").unwrap();
        // A directory squatting on the temp name: neither removable as a
        // file nor writable by VACUUM INTO.
        std::fs::create_dir(dir.path().join("curio.db.pre-v2.bak.tmp")).unwrap();
        let err = write(&conn, &db, 2).unwrap_err();
        assert!(matches!(err, OpenError::Backup { .. }), "{err:?}");
        assert!(err.to_string().contains("not migrated"));
    }

    #[test]
    fn backups_list_newest_first_with_the_schema_version_inside() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x);").unwrap();
        conn.pragma_update(None, "user_version", 3).unwrap();
        write(&conn, &db, 4).unwrap();
        conn.pragma_update(None, "user_version", 4).unwrap();
        write(&conn, &db, 5).unwrap();
        std::fs::write(dir.path().join("curio.db.pre-v9.bak"), b"not sqlite").unwrap();
        std::fs::write(dir.path().join("other.db.pre-v1.bak"), b"x").unwrap();

        let found = pre_migration_backups(&db);
        let summary: Vec<(i64, Option<i64>, bool)> = found
            .iter()
            .map(|b| (b.upgrade_to, b.schema_version, b.opens_in_this_build()))
            .collect();
        assert_eq!(
            summary,
            [(9, None, false), (5, Some(4), true), (4, Some(3), true)]
        );
        assert_eq!(found[1].path, dir.path().join("curio.db.pre-v5.bak"));
        // Reading the header created nothing next to the backup.
        assert!(!dir.path().join("curio.db.pre-v5.bak-shm").exists());
        assert!(!dir.path().join("curio.db.pre-v5.bak-wal").exists());
    }

    #[test]
    fn a_backup_newer_than_this_build_does_not_open_in_it() {
        let newer = PreMigrationBackup {
            path: PathBuf::from("curio.db.pre-v99.bak"),
            upgrade_to: 99,
            schema_version: Some(crate::storage::migrations::SCHEMA_VERSION + 1),
        };
        assert!(!newer.opens_in_this_build());
        let current = PreMigrationBackup {
            schema_version: Some(crate::storage::migrations::SCHEMA_VERSION),
            ..newer
        };
        assert!(current.opens_in_this_build());
    }

    #[test]
    fn a_missing_directory_lists_no_backups() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("gone").join("curio.db");
        assert_eq!(pre_migration_backups(&db), Vec::new());
        assert_eq!(header_user_version(&dir.path().join("absent.bak")), None);
        // A file shorter than a header is not a database either.
        std::fs::write(dir.path().join("short.bak"), b"SQLite format 3\0").unwrap();
        assert_eq!(header_user_version(&dir.path().join("short.bak")), None);
    }
}
