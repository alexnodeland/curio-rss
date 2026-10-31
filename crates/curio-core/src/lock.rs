//! Advisory cross-process file locks.
//!
//! Two heads on one profile — the desktop app and a CLI cron job, or two
//! cron jobs — are separate processes with separate in-memory state, so an
//! in-process `Mutex` serializes nothing between them. A [`FileLock`] is an
//! exclusive advisory lock on a small lock file (`flock(2)` on Unix,
//! `LockFileEx` on Windows, via `fs4`), held only for short critical
//! sections. The kernel releases it when the holder exits, so a crashed
//! process never leaves a stale lock behind.
//!
//! Each [`FileLock`] opens its own handle, so two `FileLock`s on one path
//! exclude each other even inside a single process (flock locks belong to
//! the open file description, `LockFileEx` locks to the handle). A single
//! `FileLock` is not re-entrant: callers keep at most one guard alive.

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Poll interval bounds while waiting for a contended lock.
const MIN_BACKOFF: Duration = Duration::from_millis(1);
const MAX_BACKOFF: Duration = Duration::from_millis(10);

/// After this long waiting, say so in the log — the wait is real.
const WAIT_NOTICE_AFTER: Duration = Duration::from_secs(1);

/// Why a lock could not be taken.
#[derive(Debug)]
pub(crate) enum LockError {
    /// Another holder kept the lock for the whole timeout.
    TimedOut {
        /// How long we waited.
        waited: Duration,
    },
    /// The lock file could not be opened or locked.
    Io(std::io::Error),
}

/// An exclusive advisory lock on one lock file.
#[derive(Debug)]
pub(crate) struct FileLock {
    path: PathBuf,
    file: File,
}

impl FileLock {
    /// Opens (creating if absent) the lock file at `path`. Opening does
    /// not lock; [`FileLock::acquire`] does.
    pub(crate) fn open(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        Ok(Self { path, file })
    }

    /// The lock file's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the lock, waiting up to `timeout` for another holder to let
    /// go. The returned guard releases it on drop.
    pub(crate) fn acquire(self: &Arc<Self>, timeout: Duration) -> Result<FileLockGuard, LockError> {
        let started = Instant::now();
        let mut backoff = MIN_BACKOFF;
        let mut noticed = false;
        loop {
            // Fully qualified: on toolchains >= 1.89 `File` has inherent
            // lock methods that would shadow the trait (and break the
            // 1.88 MSRV build).
            match <File as fs4::FileExt>::try_lock(&self.file) {
                Ok(()) => {
                    return Ok(FileLockGuard {
                        lock: Arc::clone(self),
                    });
                }
                Err(fs4::TryLockError::WouldBlock) => {}
                Err(fs4::TryLockError::Error(err)) => return Err(LockError::Io(err)),
            }
            let waited = started.elapsed();
            if waited >= timeout {
                return Err(LockError::TimedOut { waited });
            }
            if !noticed && waited >= WAIT_NOTICE_AFTER {
                noticed = true;
                tracing::info!(
                    path = %self.path.display(),
                    "waiting for another Curio process to release the profile lock"
                );
            }
            std::thread::sleep(backoff.min(timeout.saturating_sub(waited)));
            backoff = (backoff * 2).min(MAX_BACKOFF);
        }
    }
}

/// Proof the lock is held; releases it on drop.
#[derive(Debug)]
pub(crate) struct FileLockGuard {
    lock: Arc<FileLock>,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        if let Err(err) = <File as fs4::FileExt>::unlock(&self.lock.file) {
            // Closing the handle would release it too; the lock file
            // stays open for the next critical section, so report it.
            tracing::error!(path = %self.lock.path.display(), error = %err, "releasing a file lock failed");
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    #[test]
    fn a_second_handle_waits_then_times_out_while_the_first_holds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let first = Arc::new(FileLock::open(&path).unwrap());
        let second = Arc::new(FileLock::open(&path).unwrap());
        assert_eq!(second.path(), path);

        let guard = first.acquire(Duration::from_secs(1)).unwrap();
        let started = Instant::now();
        match second.acquire(Duration::from_millis(60)) {
            Err(LockError::TimedOut { waited }) => assert!(waited >= Duration::from_millis(60)),
            other => panic!("expected a timeout, got {other:?}"),
        }
        assert!(started.elapsed() >= Duration::from_millis(60));

        drop(guard);
        let _second = second.acquire(Duration::from_millis(60)).unwrap();
        // ...and now the first is the one locked out.
        assert!(matches!(
            first.acquire(Duration::ZERO),
            Err(LockError::TimedOut { .. })
        ));
    }

    #[test]
    fn a_waiter_gets_the_lock_once_the_holder_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let first = Arc::new(FileLock::open(&path).unwrap());
        let second = Arc::new(FileLock::open(&path).unwrap());
        let guard = first.acquire(Duration::from_secs(1)).unwrap();
        let waiter = std::thread::spawn(move || second.acquire(Duration::from_secs(10)).is_ok());
        std::thread::sleep(Duration::from_millis(30));
        drop(guard);
        assert!(waiter.join().unwrap());
    }

    #[test]
    fn opening_an_unwritable_path_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FileLock::open(dir.path().join("missing").join("x.lock")).is_err());
    }
}
