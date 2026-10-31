//! Contained, durable, atomic note and manifest writes: lexical
//! containment of a relative note path, then temp-file + fsync + atomic
//! rename with realpath containment under the destination root.

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::ExportError;
/// Joins a relative note path under the root, refusing traversal. This
/// is the lexical half of containment; [`write_atomic`] adds the
/// realpath half (symlinked intermediate directories cannot redirect a
/// write outside the root).
pub(super) fn contained_path(root: &Path, rel: &str) -> Result<PathBuf, ExportError> {
    let rel_path = Path::new(rel);
    let safe = rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if !safe || rel_path.is_absolute() {
        return Err(ExportError::Containment {
            path: rel.to_owned(),
        });
    }
    Ok(root.join(rel_path))
}

/// Per-process sequence for temp-file names: the pid alone collides
/// between two writes of one process.
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// Temp-file + atomic-rename write in the target's own directory,
/// with realpath containment under `root`: after the parent directories
/// exist, both they and the root are canonicalized and the write is
/// refused unless the parent still resolves under the root — a symlinked
/// intermediate directory planted inside the destination cannot redirect
/// the write elsewhere. (The final component is not followed either:
/// `rename` replaces a symlink itself rather than writing through it.)
///
/// Durable, not just atomic: the temp file is `fsync`ed before the
/// rename and the directory after it (plus the parent of every directory
/// this call created, up to the first one that already existed), so
/// after a power loss the target holds the old bytes or the new ones,
/// never the zero-length file a journaled rename of unflushed data can
/// leave behind. That is what makes the contract's note-then-manifest
/// ordering hold across a crash. A filesystem that cannot `fsync` at all
/// is written without it (see [`tolerate_unsupported_sync`]). Any failure
/// removes the temp file, so no `.curio-tmp-*` stray is left for a sync
/// tool to pick up. Temp names are unique per write (pid + a per-process
/// sequence) and created exclusively, so concurrent writes never share
/// one.
pub(super) fn write_atomic(root: &Path, path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    let io_err = |source| ExportError::Io {
        path: path.to_path_buf(),
        source,
    };
    let dir = path.parent().ok_or_else(|| ExportError::Containment {
        path: path.display().to_string(),
    })?;
    // The deepest directory that already exists: everything below it is
    // created here, and each new entry needs its parent flushed too.
    let existing_ancestor = dir.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf);
    std::fs::create_dir_all(dir).map_err(io_err)?;
    let canonical_root = root.canonicalize().map_err(|source| ExportError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let canonical_dir = dir.canonicalize().map_err(io_err)?;
    if !canonical_dir.starts_with(&canonical_root) {
        return Err(ExportError::Containment {
            path: path.display().to_string(),
        });
    }
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let tmp = dir.join(format!(
        ".{file_name}.curio-tmp-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    land_via_temp(&tmp, path, bytes)?;
    sync_dir(dir).map_err(io_err)?;
    for created in dir
        .ancestors()
        .take_while(|a| Some(*a) != existing_ancestor.as_deref())
    {
        if let Some(parent) = created.parent() {
            sync_dir(parent).map_err(io_err)?;
        }
    }
    Ok(())
}

/// Writes `bytes` to the fresh temp file `tmp`, flushes it, and renames
/// it over `path`. On failure the temp file is removed again — unless it
/// could not be created because something already sits at that name,
/// which is not ours to delete.
fn land_via_temp(tmp: &Path, path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    if let Err(source) = write_synced(tmp, bytes) {
        if source.kind() != std::io::ErrorKind::AlreadyExists {
            discard_temp(tmp);
        }
        return Err(ExportError::Io {
            path: tmp.to_path_buf(),
            source,
        });
    }
    if let Err(source) = std::fs::rename(tmp, path) {
        discard_temp(tmp);
        return Err(ExportError::Io {
            path: path.to_path_buf(),
            source,
        });
    }
    Ok(())
}

/// Writes `bytes` to a new file at `path` — never one that already
/// exists — and flushes them to stable storage before returning, as far
/// as the filesystem can: one that cannot `fsync` at all is tolerated
/// (see [`tolerate_unsupported_sync`]).
fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    tolerate_unsupported_sync(file.sync_all(), path)
}

/// Whether an `fsync` failure only says the filesystem cannot flush this
/// handle, rather than that the data failed to reach the disk.
///
/// On Apple platforms `sync_all` is `fcntl(F_FULLFSYNC)`, which Apple
/// documents only for HFS, FAT, UDF and APFS: SMB/NFS shares, exFAT
/// drives and FUSE mounts can answer `ENOTSUP`/`EINVAL`. Some network and
/// FUSE mounts answer a directory `fsync` with `EINVAL` on Linux too.
fn is_sync_unsupported(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::InvalidInput | std::io::ErrorKind::Unsupported
    )
}

/// Maps an `fsync` the filesystem does not support to success: such a
/// filesystem offers no stronger guarantee to wait for, and refusing
/// would make every export to it fail. Real I/O errors pass through.
fn tolerate_unsupported_sync(result: std::io::Result<()>, path: &Path) -> std::io::Result<()> {
    match result {
        Err(err) if is_sync_unsupported(&err) => {
            tracing::debug!(path = %path.display(), %err, "fsync unsupported by this filesystem");
            Ok(())
        }
        other => other,
    }
}

/// Best-effort removal of a temp file after a failed write: the
/// original error is what the caller reports, so a failed cleanup is
/// only logged.
fn discard_temp(tmp: &Path) {
    if let Err(err) = std::fs::remove_file(tmp)
        && err.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(path = %tmp.display(), %err, "could not remove export temp file");
    }
}

/// Flushes a directory's entries (the rename that just landed) to
/// stable storage. Filesystems that cannot `fsync` a directory are
/// tolerated (see [`tolerate_unsupported_sync`]). On Windows a directory
/// cannot be opened as a file and NTFS journals the rename itself, so
/// this is a no-op there.
fn sync_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let handle = std::fs::File::open(dir)?;
        tolerate_unsupported_sync(handle.sync_all(), dir)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// Names of every `.curio-tmp-*` stray left in `dir`.
    fn temp_strays(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".curio-tmp-"))
            .collect()
    }

    #[test]
    fn write_atomic_creates_parents_and_lands_the_bytes() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("curio").join("note.md");
        write_atomic(root.path(), &target, b"first").unwrap();
        write_atomic(root.path(), &target, b"second").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"second");
        assert_eq!(
            temp_strays(&root.path().join("curio")),
            Vec::<String>::new()
        );
    }

    /// Injected rename failure: a directory squats on the target name, so
    /// the temp file is fully written and synced but `rename` refuses
    /// (EISDIR on unix, access denied on Windows). The error must name
    /// the target and the temp file must be gone.
    #[test]
    fn a_failed_rename_leaves_no_temp_file() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("curio");
        let target = dir.join("note.md");
        std::fs::create_dir_all(target.join("occupied")).unwrap();

        let err = write_atomic(root.path(), &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == target),
            "{err}"
        );
        assert_eq!(temp_strays(&dir), Vec::<String>::new());
        assert!(
            target.join("occupied").is_dir(),
            "the squatter is untouched"
        );
    }

    /// Injected temp-write failure: the temp name itself is occupied (by a
    /// directory, and by a file), so creating the temp file fails. The
    /// error names the temp path, nothing lands at the target, and the
    /// squatter — not ours — is left exactly as it was.
    #[test]
    fn a_failed_temp_write_reports_the_temp_path() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("curio");
        let target = dir.join("note.md");
        let tmp = dir.join(".note.md.curio-tmp-squatted");
        std::fs::create_dir_all(&tmp).unwrap();

        let err = land_via_temp(&tmp, &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == tmp),
            "{err}"
        );
        assert!(!target.exists());
        assert!(tmp.is_dir(), "the squatting directory is untouched");

        std::fs::remove_dir(&tmp).unwrap();
        std::fs::write(&tmp, b"someone else's").unwrap();
        let err = land_via_temp(&tmp, &target, b"bytes").unwrap_err();
        assert!(
            matches!(&err, ExportError::Io { path, .. } if *path == tmp),
            "{err}"
        );
        assert!(!target.exists());
        assert_eq!(std::fs::read(&tmp).unwrap(), b"someone else's");
    }

    #[test]
    fn write_atomic_creates_several_missing_levels() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("a").join("b").join("c").join("note.md");
        write_atomic(root.path(), &target, b"deep").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"deep");
    }

    #[test]
    fn unsupported_fsync_is_tolerated_but_real_errors_are_not() {
        use std::io::{Error, ErrorKind};
        let path = Path::new("/vault/curio/note.md");
        for kind in [ErrorKind::Unsupported, ErrorKind::InvalidInput] {
            assert!(is_sync_unsupported(&Error::from(kind)), "{kind:?}");
            assert!(tolerate_unsupported_sync(Err(Error::from(kind)), path).is_ok());
        }
        // EINVAL (22 on every unix) as the OS reports it maps the same way.
        #[cfg(unix)]
        assert!(is_sync_unsupported(&Error::from_raw_os_error(22)));
        for kind in [
            ErrorKind::PermissionDenied,
            ErrorKind::StorageFull,
            ErrorKind::Other,
        ] {
            assert!(!is_sync_unsupported(&Error::from(kind)), "{kind:?}");
            let err = tolerate_unsupported_sync(Err(Error::from(kind)), path).unwrap_err();
            assert_eq!(err.kind(), kind);
        }
        assert!(tolerate_unsupported_sync(Ok(()), path).is_ok());
    }

    #[test]
    fn contained_path_refuses_traversal() {
        let root = Path::new("/tmp/dest");
        assert!(contained_path(root, "curio/note.md").is_ok());
        assert!(matches!(
            contained_path(root, "../escape.md"),
            Err(ExportError::Containment { .. })
        ));
        assert!(matches!(
            contained_path(root, "/abs/path.md"),
            Err(ExportError::Containment { .. })
        ));
        assert!(matches!(
            contained_path(root, "a/../../b.md"),
            Err(ExportError::Containment { .. })
        ));
    }

    #[test]
    fn a_failed_atomic_write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        // The rename target is a non-empty directory: the rename fails
        // after the temp file was written.
        let target = dir.path().join("note.md");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("keep"), "x").unwrap();
        assert!(write_atomic(dir.path(), &target, b"bytes").is_err());
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().contains(".curio-tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }
}
