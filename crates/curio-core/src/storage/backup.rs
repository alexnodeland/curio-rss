//! The automatic pre-migration backup.
//!
//! Before any migration touches an existing database, the open path
//! snapshots it with `VACUUM INTO` to `<db>.pre-v{N}.bak` next to the
//! database, where `N` is the schema version the upgrade is heading for.
//! The snapshot is a consistent, self-contained copy (no `-wal` sidecar),
//! written to a temporary name unique to this write (pid + a per-process
//! sequence, so two processes upgrading the same file at once never share
//! or delete each other's temp file), fsynced, then renamed into place —
//! so a crash mid-backup never leaves a truncated `.bak` that looks
//! complete. Pruning keeps the backup just written plus the
//! highest-versioned other one ([`KEEP`] in all).
//!
//! Restoring one by hand: quit Curio, delete `curio.db-wal` and
//! `curio.db-shm` if present, and copy the `.bak` over `curio.db`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use rusqlite::Connection;

use super::OpenError;

/// How many pre-migration backups survive pruning (the one just written
/// included).
pub(crate) const KEEP: usize = 2;

/// A backup temp file older than this is a crashed attempt's leftover and
/// is swept by [`prune`]. Far longer than any `VACUUM INTO` of a profile
/// takes, so a concurrent writer's in-flight temp file is never touched.
const STALE_TEMP_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Per-process sequence for temp-file names: the pid alone collides
/// between two backups written by one process.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// The backup path for an upgrade of `db_path` to schema version `to`.
pub(crate) fn backup_path(db_path: &Path, to: i64) -> PathBuf {
    let mut name = db_path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".pre-v{to}.bak"));
    db_path.with_file_name(name)
}

/// A temp name for one write of `path`, unique across processes (pid) and
/// within one (sequence): `<backup>.<pid>-<seq>.tmp`.
fn temp_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
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
    let tmp = temp_path(&path);
    // VACUUM INTO refuses an existing target. A file already on this
    // name can only be a crashed process's leftover whose pid was reused
    // (a live process never reuses its own sequence number).
    match std::fs::remove_file(&tmp) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(fail(format!("removing stale {}: {err}", tmp.display()))),
    }
    let tmp_str = tmp
        .to_str()
        .ok_or_else(|| fail("the profile path is not valid UTF-8".to_owned()))?;
    let landed = conn
        .execute("VACUUM INTO ?1", [tmp_str])
        .map_err(|err| err.to_string())
        .and_then(|_| {
            // Write access: Windows' FlushFileBuffers refuses a read-only
            // handle.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&tmp)
                .and_then(|file| file.sync_all())
                .map_err(|err| format!("syncing {}: {err}", tmp.display()))
        })
        .and_then(|()| {
            // Atomic replace: a retry after a failed migration (or a
            // concurrent upgrade of the same file) overwrites an earlier,
            // complete backup of the same upgrade. Windows can refuse the
            // replace while a concurrent writer's rename onto the same name
            // is in flight; the backup that writer landed is just as
            // complete, so it serves this open too.
            std::fs::rename(&tmp, &path).or_else(|err| {
                if path.is_file() {
                    discard_temp(&tmp);
                    Ok(())
                } else {
                    Err(format!("renaming into place: {err}"))
                }
            })
        });
    if let Err(detail) = landed {
        discard_temp(&tmp);
        return Err(fail(detail));
    }
    tracing::info!(backup = %path.display(), "wrote pre-migration database backup");
    prune(db_path, &path);
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
/// nothing is opened, locked or created next to it. In-flight backup temp
/// files are not backups and are skipped. An unreadable directory lists as
/// empty.
#[must_use]
pub fn pre_migration_backups(db_path: &Path) -> Vec<PreMigrationBackup> {
    let dir = match db_path.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir,
        _ => Path::new("."),
    };
    let Some(stem) = db_path.file_name().and_then(|n| n.to_str()) else {
        return Vec::new();
    };
    let prefix = format!("{stem}.pre-v");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut backups: Vec<PreMigrationBackup> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name();
            let upgrade_to = name
                .to_str()?
                .strip_prefix(&prefix)?
                .strip_suffix(".bak")?
                .parse::<i64>()
                .ok()?;
            let path = entry.path();
            Some(PreMigrationBackup {
                schema_version: header_user_version(&path),
                path,
                upgrade_to,
            })
        })
        .collect();
    backups.sort_by_key(|backup| std::cmp::Reverse(backup.upgrade_to));
    backups
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

/// Best-effort removal of this write's temp file after a failure: the
/// original error is what the caller reports.
fn discard_temp(tmp: &Path) {
    if let Err(err) = std::fs::remove_file(tmp)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %tmp.display(), %err, "could not remove a backup temp file");
    }
}

/// Deletes old backups of `db_path`, keeping `just_written` plus the
/// highest-versioned others up to [`KEEP`] in all — the backup this open
/// relies on is never pruned, even when a newer build left
/// higher-numbered ones behind (a manual rollback). Also sweeps backup
/// temp files older than [`STALE_TEMP_AGE`]. Best-effort: a failure here
/// only costs disk space, so it is logged, never surfaced.
fn prune(db_path: &Path, just_written: &Path) {
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
    let keep_name = just_written.file_name();
    let mut backups: Vec<(i64, PathBuf)> = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let name = entry.file_name();
        let Some(rest) = name.to_str().and_then(|n| n.strip_prefix(&prefix)) else {
            continue;
        };
        // Our own temp names: `<db>.pre-v<N>.bak.<pid>-<seq>.tmp`.
        if Path::new(rest).extension().is_some_and(|ext| ext == "tmp") && rest.contains(".bak.") {
            if is_stale(&entry) {
                remove_logged(&entry.path());
            }
            continue;
        }
        if Some(name.as_os_str()) == keep_name {
            continue;
        }
        if let Some(version) = rest
            .strip_suffix(".bak")
            .and_then(|v| v.parse::<i64>().ok())
        {
            backups.push((version, entry.path()));
        }
    }
    backups.sort_by_key(|(version, _)| std::cmp::Reverse(*version));
    for (_, stale) in backups.into_iter().skip(KEEP.saturating_sub(1)) {
        remove_logged(&stale);
    }
}

/// Whether a directory entry was last modified more than
/// [`STALE_TEMP_AGE`] ago (unknown times count as fresh).
fn is_stale(entry: &std::fs::DirEntry) -> bool {
    entry
        .metadata()
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
        .is_some_and(|age| age > STALE_TEMP_AGE)
}

/// Best-effort delete of one pruned file.
fn remove_logged(path: &Path) {
    if let Err(err) = std::fs::remove_file(path) {
        tracing::warn!(backup = %path.display(), error = %err, "could not prune an old backup");
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed unwrap is the assertion.
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

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
        prune(&db, &dir.path().join("curio.db.pre-v10.bak"));
        assert_eq!(
            listing(dir.path()),
            [
                "curio.db.pre-v10.bak",
                "curio.db.pre-v6.bak",
                "curio.db.pre-vX.bak",
                "notes.txt",
                "other.db.pre-v1.bak",
            ]
        );
    }

    /// A manual rollback: a newer build left pre-v9 and pre-v10 behind,
    /// the user restored an older backup and an older build now upgrades
    /// it to v8. The backup it just wrote must survive pruning.
    #[test]
    fn prune_never_deletes_the_backup_just_written() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        for name in ["curio.db.pre-v9.bak", "curio.db.pre-v10.bak"] {
            std::fs::write(dir.path().join(name), b"x").unwrap();
        }
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x);").unwrap();
        let path = write(&conn, &db, 8).unwrap();
        assert!(path.exists());
        assert_eq!(
            listing(dir.path()),
            ["curio.db", "curio.db.pre-v10.bak", "curio.db.pre-v8.bak"]
        );
    }

    #[test]
    fn prune_sweeps_only_stale_temp_files() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        let stale = dir.path().join("curio.db.pre-v4.bak.111-0.tmp");
        let fresh = dir.path().join("curio.db.pre-v4.bak.222-0.tmp");
        std::fs::write(&stale, b"partial").unwrap();
        std::fs::write(&fresh, b"in flight").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&stale)
            .unwrap()
            .set_modified(SystemTime::now() - STALE_TEMP_AGE - Duration::from_secs(60))
            .unwrap();
        prune(&db, &dir.path().join("curio.db.pre-v4.bak"));
        assert!(!stale.exists(), "a crashed attempt's leftover is swept");
        assert!(
            fresh.exists(),
            "a concurrent writer's temp file is left alone"
        );
    }

    #[test]
    fn temp_names_are_unique_per_write() {
        let path = Path::new("/p/curio.db.pre-v2.bak");
        let (a, b) = (temp_path(path), temp_path(path));
        assert_ne!(a, b);
        let name = a.file_name().unwrap().to_str().unwrap();
        assert!(name.starts_with("curio.db.pre-v2.bak."), "{name}");
        assert_eq!(Path::new(name).extension().unwrap(), "tmp", "{name}");
        assert!(name.contains(&std::process::id().to_string()), "{name}");
    }

    #[test]
    fn an_existing_backup_is_overwritten_and_no_temp_file_is_left() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x); INSERT INTO t VALUES (1);")
            .unwrap();
        std::fs::write(dir.path().join("curio.db.pre-v2.bak"), b"old").unwrap();
        let path = write(&conn, &db, 2).unwrap();
        assert_eq!(listing(dir.path()), ["curio.db", "curio.db.pre-v2.bak"]);
        let copy = Connection::open(&path).unwrap();
        let x: i64 = copy.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(x, 1);
    }

    /// Several writers backing up the same upgrade at once (the desktop
    /// and a cron `curio` both opening an older profile) each land a
    /// complete backup and never trip over each other's temp file.
    #[test]
    fn concurrent_backups_of_one_upgrade_all_succeed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(
                "CREATE TABLE t (x);
                 WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < 2000)
                 INSERT INTO t SELECT randomblob(256) FROM n;",
            )
            .unwrap();
        }
        let barrier = std::sync::Barrier::new(4);
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    let conn = Connection::open(&db).unwrap();
                    barrier.wait();
                    write(&conn, &db, 2).unwrap();
                });
            }
        });
        assert_eq!(listing(dir.path()), ["curio.db", "curio.db.pre-v2.bak"]);
        let copy = Connection::open(dir.path().join("curio.db.pre-v2.bak")).unwrap();
        let n: i64 = copy
            .query_row("SELECT COUNT(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 2000);
    }

    #[test]
    fn an_unwritable_target_is_a_typed_backup_error() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("curio.db");
        let conn = Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE t (x);").unwrap();
        // The backup would land in a directory that does not exist.
        let elsewhere = dir.path().join("missing").join("curio.db");
        let err = write(&conn, &elsewhere, 2).unwrap_err();
        assert!(matches!(err, OpenError::Backup { .. }), "{err:?}");
        assert!(err.to_string().contains("not migrated"));
        assert_eq!(listing(dir.path()), ["curio.db"]);
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
        // An in-flight write's temp file is not a backup.
        std::fs::write(dir.path().join("curio.db.pre-v7.bak.1-0.tmp"), b"x").unwrap();

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
