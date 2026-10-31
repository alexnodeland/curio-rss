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

/// Deletes all but the [`KEEP`] highest-versioned backups of `db_path`.
/// Best-effort: a failure here only costs disk space, so it is logged,
/// never surfaced.
fn prune(db_path: &Path) {
    let dir = match db_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let Some(stem) = db_path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let prefix = format!("{stem}.pre-v");
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!(error = %err, "could not list backups for pruning");
            return;
        }
    };
    let mut backups: Vec<(i64, PathBuf)> = entries
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
}
