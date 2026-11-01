//! Contained, durable, atomic note and manifest writes: lexical
//! containment of a relative note path, then temp-file + fsync + atomic
//! rename with realpath containment under the destination root.
//! [`write_file_atomic`] is the same temp + rename without the containment,
//! for the heads' single-file exports (OPML, themes) to a user-picked path.

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
    let io_err = |source| ExportError::io(path, source);
    let dir = path.parent().ok_or_else(|| ExportError::Containment {
        path: path.display().to_string(),
    })?;
    // The deepest directory that already exists: everything below it is
    // created here, and each new entry needs its parent flushed too.
    let existing_ancestor = dir.ancestors().find(|a| a.is_dir()).map(Path::to_path_buf);
    std::fs::create_dir_all(dir).map_err(io_err)?;
    let canonical_root = root
        .canonicalize()
        .map_err(|source| ExportError::io(root, source))?;
    let canonical_dir = dir.canonicalize().map_err(io_err)?;
    if !canonical_dir.starts_with(&canonical_root) {
        return Err(ExportError::Containment {
            path: path.display().to_string(),
        });
    }
    land_via_temp(&temp_path(dir, path), path, bytes)?;
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

/// Atomically replaces the file at `path` with `bytes`: they go to a
/// fresh temp file in the same directory, are `fsync`ed, and the temp file
/// is renamed over `path` (then the directory is flushed). A crash or a
/// failed write at any point leaves the previous file exactly as it was —
/// never truncated — and no temp file behind. Unlike the destination
/// writes there is no containment check: `path` is one the user picked.
/// The parent directory must already exist. A symlink at `path` is
/// replaced, not written through.
///
/// # Errors
///
/// The I/O error of the failed step (temp write, rename, or flush).
pub fn write_file_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    write_file_atomic_with(path, |file| file.write_all(bytes))
}

/// [`write_file_atomic`] with the temp file's contents produced by `fill` —
/// the seam the injected-failure tests use to fail mid-write.
fn write_file_atomic_with(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let dir = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    land_with(&temp_path(dir, path), path, fill).map_err(|(_, source)| source)?;
    sync_dir(dir)
}

/// A temp-file name next to `path` in `dir`, unique per write (pid + a
/// per-process sequence) and dot-prefixed so file browsers hide it.
fn temp_path(dir: &Path, path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    dir.join(format!(
        ".{file_name}.curio-tmp-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Writes `bytes` to the fresh temp file `tmp`, flushes it, and renames
/// it over `path` (see [`land_with`]).
fn land_via_temp(tmp: &Path, path: &Path, bytes: &[u8]) -> Result<(), ExportError> {
    use std::io::Write as _;
    land_via_temp_with(tmp, path, |file| file.write_all(bytes))
}

/// [`land_via_temp`] with the temp file's contents produced by `fill` —
/// the seam the injected-failure tests use to fail mid-write. The failure
/// is sorted into its class by [`ExportError::io`].
fn land_via_temp_with(
    tmp: &Path,
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<(), ExportError> {
    land_with(tmp, path, fill).map_err(|(path, source)| ExportError::io(path, source))
}

/// Fills the fresh temp file `tmp` via `fill`, flushes it, and renames it
/// over `path`. On failure the temp file is removed again — unless it could
/// not be created because something already sits at that name, which is not
/// ours to delete. The error names the path that failed: the temp file for
/// a failed write, the target for a failed rename.
fn land_with(
    tmp: &Path,
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> Result<(), (PathBuf, std::io::Error)> {
    if let Err(source) = write_synced(tmp, fill) {
        if source.kind() != std::io::ErrorKind::AlreadyExists {
            discard_temp(tmp);
        }
        return Err((tmp.to_path_buf(), source));
    }
    if let Err(source) = std::fs::rename(tmp, path) {
        discard_temp(tmp);
        return Err((path.to_path_buf(), source));
    }
    Ok(())
}

/// Creates a new file at `path` — never one that already exists — fills it
/// via `fill`, and flushes it to stable storage before returning, as far
/// as the filesystem can: one that cannot `fsync` at all is tolerated
/// (see [`tolerate_unsupported_sync`]).
fn write_synced(
    path: &Path,
    fill: impl FnOnce(&mut std::fs::File) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut handle = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    fill(&mut handle)?;
    tolerate_unsupported_sync(handle.sync_all(), path)
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

    /// The path a filesystem-class export error names. A directory
    /// squatting on a name is `EISDIR`/`EEXIST` on unix but "access denied"
    /// on Windows, so those injected failures sort into `Io` or
    /// `PermissionDenied` depending on the OS.
    fn failed_fs_path(err: &ExportError) -> Option<&Path> {
        match err {
            ExportError::Io { path, .. }
            | ExportError::PermissionDenied { path, .. }
            | ExportError::DiskFull { path, .. } => Some(path),
            _ => None,
        }
    }

    #[test]
    fn filesystem_failures_are_sorted_by_what_the_user_can_do() {
        use std::error::Error as _;
        use std::io::{Error, ErrorKind};
        let path = Path::new("/vault/curio/note.md");
        let class = |source: Error| match ExportError::io(path, source) {
            ExportError::PermissionDenied { .. } => "permission",
            ExportError::DiskFull { .. } => "full",
            ExportError::Io { .. } => "io",
            other => panic!("not a filesystem class: {other:?}"),
        };
        for (kind, expected) in [
            (ErrorKind::PermissionDenied, "permission"),
            (ErrorKind::ReadOnlyFilesystem, "permission"),
            (ErrorKind::StorageFull, "full"),
            (ErrorKind::QuotaExceeded, "full"),
            (ErrorKind::NotFound, "io"),
            (ErrorKind::IsADirectory, "io"),
            (ErrorKind::Other, "io"),
        ] {
            assert_eq!(class(Error::from(kind)), expected, "{kind:?}");
        }
        // The codes the OS actually reports sort the same way.
        #[cfg(unix)]
        for (errno, expected) in [
            (1, "permission"),  // EPERM
            (13, "permission"), // EACCES
            (30, "permission"), // EROFS
            (28, "full"),       // ENOSPC
            (5, "io"),          // EIO
        ] {
            assert_eq!(class(Error::from_raw_os_error(errno)), expected, "{errno}");
        }
        #[cfg(windows)]
        for (code, expected) in [
            (5, "permission"), // ERROR_ACCESS_DENIED
            (112, "full"),     // ERROR_DISK_FULL
            (39, "full"),      // ERROR_HANDLE_DISK_FULL
        ] {
            assert_eq!(class(Error::from_raw_os_error(code)), expected, "{code}");
        }

        // The message names the path once and says what to do; the OS
        // error stays reachable as the source.
        for (err, advice) in [
            (
                ExportError::io(path, Error::from(ErrorKind::PermissionDenied)),
                "check the destination folder's permissions",
            ),
            (
                ExportError::io(path, Error::from(ErrorKind::StorageFull)),
                "free some space",
            ),
        ] {
            let message = err.to_string();
            assert!(message.contains(advice), "{message}");
            assert_eq!(
                message.matches("/vault/curio/note.md").count(),
                1,
                "{message}"
            );
            assert!(err.source().is_some(), "{err:?}");
        }
    }

    /// Injected failures half-way through a note write: a full disk and a
    /// permission refusal reach the caller as their typed classes, naming
    /// the temp file, with the previous note intact and no temp stray.
    #[test]
    fn a_full_disk_or_refused_write_mid_note_is_typed() {
        use std::io::{Error, ErrorKind, Write as _};
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("note.md");
        std::fs::write(&target, b"previous note").unwrap();
        for (kind, full) in [
            (ErrorKind::StorageFull, true),
            (ErrorKind::PermissionDenied, false),
        ] {
            let tmp = temp_path(root.path(), &target);
            let err = land_via_temp_with(&tmp, &target, |file| {
                file.write_all(b"half a no")?;
                Err(Error::new(kind, "injected"))
            })
            .unwrap_err();
            let typed = if full {
                matches!(&err, ExportError::DiskFull { path, .. } if *path == tmp)
            } else {
                matches!(&err, ExportError::PermissionDenied { path, .. } if *path == tmp)
            };
            assert!(typed, "{kind:?}: {err:?}");
            assert_eq!(std::fs::read(&target).unwrap(), b"previous note");
            assert_eq!(temp_strays(root.path()), Vec::<String>::new());
        }
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
        assert_eq!(failed_fs_path(&err), Some(target.as_path()), "{err}");
        #[cfg(unix)]
        assert!(matches!(err, ExportError::Io { .. }), "{err:?}");
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
        assert_eq!(failed_fs_path(&err), Some(tmp.as_path()), "{err}");
        #[cfg(unix)]
        assert!(matches!(err, ExportError::Io { .. }), "{err:?}");
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
    fn write_file_atomic_replaces_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("subs.opml");
        std::fs::write(&target, b"old").unwrap();
        write_file_atomic(&target, b"new").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert_eq!(temp_strays(dir.path()), Vec::<String>::new());
    }

    /// Injected failure half-way through the write: the export over an
    /// existing file errors, the previous file is byte-for-byte intact (not
    /// truncated, as a plain `fs::write` would leave it), and the half-written
    /// temp file is cleaned up.
    #[test]
    fn a_failed_write_file_atomic_keeps_the_previous_file() {
        use std::io::Write as _;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("subs.opml");
        std::fs::write(&target, b"<opml>the user's previous export</opml>").unwrap();

        let err = write_file_atomic_with(&target, |file| {
            file.write_all(b"<opml>half")?;
            Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "injected: disk full mid-write",
            ))
        })
        .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::StorageFull);
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"<opml>the user's previous export</opml>"
        );
        assert_eq!(temp_strays(dir.path()), Vec::<String>::new());
    }

    #[test]
    fn write_file_atomic_into_a_missing_directory_fails_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("gone").join("subs.opml");
        let err = write_file_atomic(&target, b"bytes").unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        assert!(!target.exists());
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
