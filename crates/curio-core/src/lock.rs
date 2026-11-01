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
//!
//! **A deleted lock file is re-created, never trusted.** The lock lives on
//! the file a handle has open, not on the path: if the file is deleted (a
//! `git clean -X`, a hand clean-up) while a long-lived handle keeps it
//! open, a newcomer creates a fresh file at the path and locks *that* —
//! two holders that no longer exclude each other. So every
//! [`FileLock::acquire`] checks, once it holds the lock, that its handle is
//! still the file at the path; if not, it lets go, reopens the path and
//! locks again. A deletion can therefore overlap at most the one critical
//! section in flight when it happened.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use same_file::Handle;

/// Poll interval bounds while waiting for a contended lock.
const MIN_BACKOFF: Duration = Duration::from_millis(1);
const MAX_BACKOFF: Duration = Duration::from_millis(10);

/// [`FileLock::acquire_async`] waits are for whole critical sections that
/// span network I/O (seconds, not microseconds), so it polls less often.
const ASYNC_MAX_BACKOFF: Duration = Duration::from_millis(50);

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
    Io(io::Error),
}

/// An exclusive advisory lock on one lock file.
#[derive(Debug)]
pub(crate) struct FileLock {
    path: PathBuf,
    /// The open lock file, as a `same_file` handle so its identity can be
    /// compared with whatever file is at `path` now. Swapped for a fresh
    /// handle when the file at the path turns out to have been replaced.
    handle: Mutex<Arc<Handle>>,
}

impl FileLock {
    /// Opens (creating if absent) the lock file at `path`. Opening does
    /// not lock; [`FileLock::acquire`] does.
    pub(crate) fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let handle = Mutex::new(Arc::new(open_lock_file(&path)?));
        Ok(Self { path, handle })
    }

    /// The lock file's path.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// [`FileLock::acquire`] for async callers: waits up to `timeout` with
    /// `tokio` sleeps between attempts, so a wait never blocks the
    /// runtime thread (other tasks on it, possibly the holder, keep
    /// running). Each attempt itself never blocks.
    pub(crate) async fn acquire_async(
        &self,
        timeout: Duration,
    ) -> Result<FileLockGuard, LockError> {
        let started = Instant::now();
        let mut backoff = MIN_BACKOFF;
        let mut noticed = false;
        loop {
            match self.acquire(Duration::ZERO) {
                Err(LockError::TimedOut { .. }) => {}
                acquired => return acquired,
            }
            let waited = started.elapsed();
            if waited >= timeout {
                return Err(LockError::TimedOut { waited });
            }
            if !noticed && waited >= WAIT_NOTICE_AFTER {
                noticed = true;
                tracing::info!(
                    path = %self.path.display(),
                    "waiting for another Curio process to release a lock"
                );
            }
            tokio::time::sleep(backoff.min(timeout.saturating_sub(waited))).await;
            backoff = (backoff * 2).min(ASYNC_MAX_BACKOFF);
        }
    }

    /// Takes the lock, waiting up to `timeout` for another holder to let
    /// go. The returned guard releases it on drop.
    pub(crate) fn acquire(&self, timeout: Duration) -> Result<FileLockGuard, LockError> {
        let started = Instant::now();
        let mut backoff = MIN_BACKOFF;
        let mut noticed = false;
        loop {
            let handle = Arc::clone(&self.handle.lock().unwrap_or_else(PoisonError::into_inner));
            // Fully qualified: on toolchains >= 1.89 `File` has inherent
            // lock methods that would shadow the trait (and break the
            // 1.88 MSRV build).
            match <File as fs4::FileExt>::try_lock(handle.as_file()) {
                Ok(()) => {
                    let guard = FileLockGuard {
                        path: self.path.clone(),
                        handle,
                    };
                    if self.is_current(&guard.handle).map_err(LockError::Io)? {
                        return Ok(guard);
                    }
                    // Our file was deleted or replaced under us: holding
                    // its lock excludes nobody. Release it, adopt the
                    // file now at the path and go again.
                    drop(guard);
                    self.reopen().map_err(LockError::Io)?;
                    continue;
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

    /// Whether `handle` is still the file at the lock path — it is not
    /// once the path was deleted, or deleted and re-created by another
    /// process. (Opening the path for the comparison is harmless: flock
    /// and `LockFileEx` locks belong to our own handle, and closing a
    /// different one releases nothing.)
    fn is_current(&self, handle: &Handle) -> io::Result<bool> {
        match Handle::from_path(&self.path) {
            Ok(at_path) => Ok(at_path == *handle),
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(err) => Err(err),
        }
    }

    /// Replaces the handle with one on the file now at the path (creating
    /// it if the path is gone).
    fn reopen(&self) -> io::Result<()> {
        let fresh = Arc::new(open_lock_file(&self.path)?);
        tracing::warn!(
            path = %self.path.display(),
            "the lock file was deleted or replaced while in use; re-opened it"
        );
        *self.handle.lock().unwrap_or_else(PoisonError::into_inner) = fresh;
        Ok(())
    }
}

/// Opens (creating if absent) a lock file for reading and writing.
fn open_lock_file(path: &Path) -> io::Result<Handle> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    Handle::from_file(file)
}

/// Proof the lock is held; releases it on drop.
#[derive(Debug)]
pub(crate) struct FileLockGuard {
    path: PathBuf,
    handle: Arc<Handle>,
}

impl Drop for FileLockGuard {
    fn drop(&mut self) {
        if let Err(err) = <File as fs4::FileExt>::unlock(self.handle.as_file()) {
            // Closing the handle would release it too; the lock file
            // stays open for the next critical section, so report it.
            tracing::error!(path = %self.path.display(), error = %err, "releasing a file lock failed");
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
        let first = FileLock::open(&path).unwrap();
        let second = FileLock::open(&path).unwrap();
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
        let first = FileLock::open(&path).unwrap();
        let second = FileLock::open(&path).unwrap();
        let guard = first.acquire(Duration::from_secs(1)).unwrap();
        let waiter = std::thread::spawn(move || second.acquire(Duration::from_secs(10)).is_ok());
        std::thread::sleep(Duration::from_millis(30));
        drop(guard);
        assert!(waiter.join().unwrap());
    }

    /// The lock file deleted, then re-created by a newcomer, under a
    /// long-lived handle: the long-lived handle's next acquire must move
    /// to the file now at the path, so the two still exclude each other.
    /// Unix only: whether Windows lets a file that is open elsewhere be
    /// deleted and its name reused at once depends on the OS version and
    /// filesystem; the code path under test is the same on both.
    #[cfg(unix)]
    #[test]
    fn a_deleted_lock_file_is_re_opened_and_still_excludes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let long_lived = FileLock::open(&path).unwrap();
        drop(long_lived.acquire(Duration::from_secs(1)).unwrap());

        std::fs::remove_file(&path).unwrap();
        let newcomer = FileLock::open(&path).unwrap();
        let held = newcomer.acquire(Duration::from_secs(1)).unwrap();
        // Without the identity check this would lock the unlinked file
        // and succeed: two holders at once.
        assert!(matches!(
            long_lived.acquire(Duration::from_millis(50)),
            Err(LockError::TimedOut { .. })
        ));
        drop(held);
        let _ours = long_lived.acquire(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            newcomer.acquire(Duration::ZERO),
            Err(LockError::TimedOut { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_lock_file_deleted_with_nobody_else_around_is_re_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let lock = FileLock::open(&path).unwrap();
        std::fs::remove_file(&path).unwrap();
        let _guard = lock.acquire(Duration::from_secs(1)).unwrap();
        assert!(path.exists(), "the acquire re-created the lock file");
        // ...and the re-created file is the one now held.
        let other = FileLock::open(&path).unwrap();
        assert!(matches!(
            other.acquire(Duration::ZERO),
            Err(LockError::TimedOut { .. })
        ));
    }

    #[test]
    fn opening_an_unwritable_path_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(FileLock::open(dir.path().join("missing").join("x.lock")).is_err());
    }

    /// The async wait runs on a current-thread runtime, so it must yield
    /// between attempts: the task that releases the lock shares the thread.
    #[tokio::test]
    async fn an_async_waiter_yields_until_the_holder_releases() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let holder = FileLock::open(&path).unwrap();
        let waiter = FileLock::open(&path).unwrap();
        let guard = holder.acquire(Duration::from_secs(1)).unwrap();
        let release = async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            drop(guard);
        };
        let (acquired, ()) = tokio::join!(waiter.acquire_async(Duration::from_secs(10)), release);
        let _held = acquired.unwrap();
        assert!(matches!(
            holder.acquire(Duration::ZERO),
            Err(LockError::TimedOut { .. })
        ));
    }

    #[tokio::test]
    async fn an_async_waiter_times_out_while_the_holder_keeps_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("x.lock");
        let holder = FileLock::open(&path).unwrap();
        let waiter = FileLock::open(&path).unwrap();
        let _guard = holder.acquire(Duration::from_secs(1)).unwrap();
        match waiter.acquire_async(Duration::from_millis(60)).await {
            Err(LockError::TimedOut { waited }) => assert!(waited >= Duration::from_millis(60)),
            other => panic!("expected a timeout, got {other:?}"),
        }
    }
}
