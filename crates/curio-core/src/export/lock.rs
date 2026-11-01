//! Per-destination export serialization.
//!
//! The export flow is load manifest → write note → write manifest. Two
//! exports racing on one destination (a bulk export and a single save in
//! the desktop's blocking pool, or the CLI next to the desktop app) would
//! each write a manifest built from the same stale snapshot, and the
//! second rename would silently drop the first export's entry. The
//! [`DestinationLock`] spans the whole flow, from the manifest load to the
//! manifest commit, at two levels:
//!
//! - **in-process**: a process-wide set of held destination roots plus a
//!   condvar — threads of one process queue up per destination;
//! - **cross-process**: an advisory exclusive lock (`flock` /
//!   `LockFileEx`) on `.curio/manifest.lock`, which the OS releases when
//!   the holder exits, so a crashed export never wedges the destination.
//!   A filesystem without advisory locks (some NFS, SMB and FUSE mounts
//!   answer `ENOLCK`/`ENOTSUP`/`ENOSYS`) falls back to the in-process
//!   level alone, with a warning: exporting there still works, it is only
//!   unserialized against *other processes*.
//!
//! The lock file is never deleted (deleting a lock file reopens the very
//! race it guards) and carries no data; `.curio/.gitignore` keeps it out
//! of git.

use std::collections::BTreeSet;
use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Condvar, Mutex, MutexGuard, PoisonError};

use super::ExportError;

/// File name of the advisory lock inside a destination's `.curio/`.
pub(super) const LOCK_FILE: &str = "manifest.lock";

/// Canonical roots currently locked by some thread of this process.
static HELD: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());
/// Signalled whenever a root leaves [`HELD`].
static RELEASED: Condvar = Condvar::new();

/// Lock files whose filesystem refused an advisory lock — warned about
/// once per process, not once per exported note.
static UNLOCKABLE: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// OS error codes meaning "this filesystem has no advisory locks" (as
/// opposed to a real failure, which stays a hard error).
#[cfg(unix)]
const NO_ADVISORY_LOCKS: &[i32] = &[libc::ENOLCK, libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS];
/// `ERROR_INVALID_FUNCTION` and `ERROR_NOT_SUPPORTED` — what `LockFileEx`
/// answers on a share or filesystem driver without byte-range locks.
#[cfg(windows)]
const NO_ADVISORY_LOCKS: &[i32] = &[1, 50];
#[cfg(not(any(unix, windows)))]
const NO_ADVISORY_LOCKS: &[i32] = &[];

fn advisory_locks_unsupported(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::Unsupported
        || err
            .raw_os_error()
            .is_some_and(|code| NO_ADVISORY_LOCKS.contains(&code))
}

/// Takes the cross-process lock with `lock`. On a filesystem without
/// advisory locks the export carries on under the in-process lock alone
/// (warned once per lock file); any other failure is returned.
fn lock_advisory(
    file: &File,
    lock_path: &Path,
    lock: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<()> {
    match lock(file) {
        Ok(()) => Ok(()),
        Err(err) if advisory_locks_unsupported(&err) => {
            let first = UNLOCKABLE
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(lock_path.to_path_buf());
            if first {
                tracing::warn!(
                    path = %lock_path.display(),
                    %err,
                    "the destination's filesystem has no advisory locks; exports to it are \
                     serialized within this process only"
                );
            }
            Ok(())
        }
        Err(err) => Err(err),
    }
}

fn held() -> MutexGuard<'static, BTreeSet<PathBuf>> {
    // The set is only ever mutated by a single insert/remove — a
    // panicking holder cannot leave it half-updated.
    HELD.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Exclusive ownership of one destination's manifest for the duration of
/// an export. Released on drop: the OS lock first, then the in-process
/// slot (so a queued thread never blocks on the OS lock we still hold).
#[derive(Debug)]
pub(crate) struct DestinationLock {
    file: Option<File>,
    key: PathBuf,
}

impl DestinationLock {
    /// Blocks until this process holds the destination exclusively.
    ///
    /// Never creates the destination root: a missing root is
    /// [`ExportError::DestinationMissing`] (a moved or unmounted vault
    /// must not be recreated empty at the old path). `.curio/` itself is
    /// created when absent, inside the existing root.
    pub(crate) fn acquire(root: &Path) -> Result<Self, ExportError> {
        if !root.is_dir() {
            return Err(ExportError::DestinationMissing {
                root: root.to_path_buf(),
            });
        }
        let key = root
            .canonicalize()
            .map_err(|source| ExportError::io(root, source))?;
        {
            let mut set = held();
            while set.contains(&key) {
                set = RELEASED.wait(set).unwrap_or_else(PoisonError::into_inner);
            }
            set.insert(key.clone());
        }
        // From here on, `guard` releases the in-process slot on every
        // error path.
        let mut guard = Self { file: None, key };
        let marker = root.join(".curio");
        match std::fs::create_dir(&marker) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(source) => return Err(ExportError::io(marker, source)),
        }
        let lock_path = marker.join(LOCK_FILE);
        let io_err = |source| ExportError::io(&lock_path, source);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .map_err(io_err)?;
        // Fully qualified: from Rust 1.89 std has an inherent `File::lock`
        // that would shadow the trait method on newer toolchains only.
        lock_advisory(&file, &lock_path, <File as fs4::FileExt>::lock).map_err(io_err)?;
        guard.file = Some(file);
        Ok(guard)
    }
}

impl Drop for DestinationLock {
    fn drop(&mut self) {
        // Closing the handle releases the advisory lock.
        drop(self.file.take());
        held().remove(&self.key);
        RELEASED.notify_all();
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    #[test]
    fn missing_root_is_destination_missing_and_nothing_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        let err = DestinationLock::acquire(&root).unwrap_err();
        assert!(
            matches!(err, ExportError::DestinationMissing { .. }),
            "{err}"
        );
        assert!(!root.exists(), "the root must never be recreated");
    }

    #[test]
    fn a_regular_file_root_is_destination_missing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("vault");
        std::fs::write(&root, "not a directory").unwrap();
        assert!(matches!(
            DestinationLock::acquire(&root),
            Err(ExportError::DestinationMissing { .. })
        ));
    }

    #[test]
    fn acquire_creates_the_marker_and_lock_file_and_release_frees_the_slot() {
        let dir = tempfile::tempdir().unwrap();
        let lock = DestinationLock::acquire(dir.path()).unwrap();
        assert!(dir.path().join(".curio").join(LOCK_FILE).is_file());
        drop(lock);
        // Re-acquiring on the same thread would deadlock if drop leaked
        // the in-process slot.
        drop(DestinationLock::acquire(dir.path()).unwrap());
    }

    /// The advisory lock is what serializes separate processes (CLI next
    /// to the desktop app). A handle that bypasses the in-process set —
    /// exactly what another process looks like — must hold exporters off
    /// until it lets go.
    #[test]
    fn a_foreign_advisory_lock_holder_blocks_acquisition_until_released() {
        let dir = tempfile::tempdir().unwrap();
        drop(DestinationLock::acquire(dir.path()).unwrap());
        let foreign = OpenOptions::new()
            .write(true)
            .open(dir.path().join(".curio").join(LOCK_FILE))
            .unwrap();
        fs4::FileExt::lock(&foreign).unwrap();

        let (tx, rx) = mpsc::channel();
        let root = dir.path().to_path_buf();
        let waiter = std::thread::spawn(move || {
            let lock = DestinationLock::acquire(&root).unwrap();
            tx.send(()).unwrap();
            drop(lock);
        });
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "acquired while another handle held the advisory lock"
        );
        drop(foreign);
        rx.recv_timeout(Duration::from_secs(30)).unwrap();
        waiter.join().unwrap();
    }

    #[test]
    fn a_filesystem_without_advisory_locks_falls_back_to_the_in_process_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let file = File::create(&path).unwrap();
        // Twice each: the second refusal takes the already-warned path.
        for _ in 0..2 {
            lock_advisory(&file, &path, |_| {
                Err(io::Error::from(io::ErrorKind::Unsupported))
            })
            .unwrap();
            for code in NO_ADVISORY_LOCKS {
                lock_advisory(&file, &path, |_| Err(io::Error::from_raw_os_error(*code))).unwrap();
            }
        }
    }

    #[test]
    fn a_real_lock_failure_stays_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let file = File::create(&path).unwrap();
        let err = lock_advisory(&file, &path, |_| {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn threads_of_one_process_queue_per_destination() {
        let dir = tempfile::tempdir().unwrap();
        let first = DestinationLock::acquire(dir.path()).unwrap();

        let (tx, rx) = mpsc::channel();
        let root = dir.path().to_path_buf();
        let waiter = std::thread::spawn(move || {
            let lock = DestinationLock::acquire(&root).unwrap();
            tx.send(()).unwrap();
            drop(lock);
        });
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());

        // A different destination is independent of the held one.
        let other = tempfile::tempdir().unwrap();
        drop(DestinationLock::acquire(other.path()).unwrap());

        drop(first);
        rx.recv_timeout(Duration::from_secs(30)).unwrap();
        waiter.join().unwrap();
    }
}
