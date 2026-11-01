//! The read-only connection pool.
//!
//! A fixed set of `SQLITE_OPEN_READ_ONLY` connections checked out one at a
//! time. Under WAL, readers never block the writer and the writer never
//! blocks readers — the pool exists only so N queries can run in parallel.
//! The `Mutex` here guards the *free list*, never a connection in use;
//! this is exactly what the banned `Mutex<Connection>` pattern was not.
//!
//! A checkout always comes back: the connection rejoins the free list on
//! success, on error, and when the read closure panics (the panic is
//! caught, the connection returned, then the panic resumed) — so a bug in
//! one query can never shrink the pool. Waiting for a free connection is bounded by the pool timeout and
//! fails with [`StorageError::PoolTimeout`] instead of hanging forever.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::path::Path;
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};

use super::StorageError;

/// How long a read waits for a free pooled connection before giving up
/// with [`StorageError::PoolTimeout`]. Reads take milliseconds; a wait
/// this long means every connection is stuck behind something else.
pub(crate) const DEFAULT_POOL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) struct ReadPool {
    free: Mutex<Vec<Connection>>,
    available: Condvar,
    timeout: Duration,
}

impl std::fmt::Debug for ReadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadPool")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl ReadPool {
    /// Opens `size` read-only connections against the (already migrated)
    /// database at `path`.
    pub(crate) fn open(path: &Path, size: usize, timeout: Duration) -> Result<Self, StorageError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let mut free = Vec::with_capacity(size);
        for _ in 0..size {
            let conn = Connection::open_with_flags(path, flags)?;
            conn.busy_timeout(super::BUSY_TIMEOUT)?;
            // Defense in depth on top of the read-only open flag.
            conn.pragma_update(None, "query_only", true)?;
            free.push(conn);
        }
        Ok(Self {
            free: Mutex::new(free),
            available: Condvar::new(),
            timeout,
        })
    }

    /// Checks a connection out, runs `f`, and returns the connection to
    /// the pool — also on error and when `f` panics (the panic is then
    /// resumed on the caller's thread, where it belongs).
    ///
    /// # Errors
    ///
    /// [`StorageError::PoolTimeout`] when no connection frees up within
    /// the pool timeout; otherwise whatever `f` returns.
    pub(crate) fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let conn = self.checkout()?;
        // AssertUnwindSafe: the connection is the only state `f` can
        // touch, and it is reset (any open transaction rolled back)
        // before it rejoins the free list.
        let out = catch_unwind(AssertUnwindSafe(|| f(&conn)));
        // A closure that issued a bare BEGIN (or panicked inside one)
        // must not hand the next reader a stale snapshot.
        super::writer::rollback_if_open(&conn);
        self.checkin(conn);
        out.unwrap_or_else(|payload| resume_unwind(payload))
    }

    fn checkout(&self) -> Result<Connection, StorageError> {
        let free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        let (mut free, _wait) = self
            .available
            .wait_timeout_while(free, self.timeout, |free| free.is_empty())
            .unwrap_or_else(PoisonError::into_inner);
        free.pop().ok_or_else(|| {
            tracing::error!(
                timeout_ms = duration_ms(self.timeout),
                "no pooled read connection freed up in time"
            );
            StorageError::PoolTimeout {
                waited_ms: duration_ms(self.timeout),
            }
        })
    }

    fn checkin(&self, conn: Connection) {
        self.free
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(conn);
        self.available.notify_one();
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::Arc;
    use std::sync::mpsc;
    use std::time::Instant;

    use super::*;

    fn pool(dir: &Path, size: usize, timeout: Duration) -> ReadPool {
        let path = dir.join("pool.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch("CREATE TABLE t (v INTEGER); INSERT INTO t VALUES (41);")
            .unwrap();
        drop(conn);
        ReadPool::open(&path, size, timeout).unwrap()
    }

    fn value(pool: &ReadPool) -> i64 {
        pool.with(|conn| Ok(conn.query_row("SELECT v + 1 FROM t", [], |row| row.get(0))?))
            .unwrap()
    }

    #[test]
    fn ten_panicking_reads_then_a_normal_read_returns_within_a_second() {
        let dir = tempfile::tempdir().unwrap();
        // Pool size 4 (the default): before the RAII guard, the 4th panic
        // emptied the pool and every later read waited forever.
        let pool = pool(dir.path(), 4, DEFAULT_POOL_TIMEOUT);
        for i in 0..10 {
            let unwound = catch_unwind(AssertUnwindSafe(|| {
                pool.with(|_conn| -> Result<(), StorageError> { panic!("read {i} blew up") })
            }));
            assert!(unwound.is_err());
        }
        let started = Instant::now();
        assert_eq!(value(&pool), 42);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(pool.free.lock().unwrap().len(), 4, "no connection leaked");
    }

    #[test]
    fn an_erroring_read_returns_its_connection() {
        let dir = tempfile::tempdir().unwrap();
        let pool = pool(dir.path(), 1, Duration::from_millis(50));
        for _ in 0..3 {
            let err = pool
                .with(|_conn| -> Result<(), StorageError> { Err(StorageError::InvalidTag) })
                .unwrap_err();
            assert!(matches!(err, StorageError::InvalidTag));
        }
        assert_eq!(value(&pool), 42);
    }

    #[test]
    fn an_exhausted_pool_times_out_instead_of_hanging() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(pool(dir.path(), 1, Duration::from_millis(100)));
        let (held_tx, held_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let holder = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                pool.with(|_conn| {
                    held_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok(())
                })
                .unwrap();
            })
        };
        held_rx.recv().unwrap();
        let started = Instant::now();
        let err = pool.with(|_conn| Ok(())).unwrap_err();
        assert!(
            matches!(err, StorageError::PoolTimeout { waited_ms: 100 }),
            "{err:?}"
        );
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(err.to_string().contains("100 ms"), "{err}");
        release_tx.send(()).unwrap();
        holder.join().unwrap();
        assert_eq!(value(&pool), 42, "the released connection serves again");
    }

    #[test]
    fn a_waiting_read_is_woken_by_a_checkin() {
        let dir = tempfile::tempdir().unwrap();
        let pool = Arc::new(pool(dir.path(), 1, Duration::from_secs(5)));
        let (held_tx, held_rx) = mpsc::channel();
        let holder = {
            let pool = Arc::clone(&pool);
            std::thread::spawn(move || {
                pool.with(|_conn| {
                    held_tx.send(()).unwrap();
                    std::thread::sleep(Duration::from_millis(50));
                    Ok(())
                })
                .unwrap();
            })
        };
        held_rx.recv().unwrap();
        assert_eq!(value(&pool), 42);
        holder.join().unwrap();
    }

    #[test]
    fn a_read_left_in_a_transaction_is_rolled_back_on_checkin() {
        let dir = tempfile::tempdir().unwrap();
        let pool = pool(dir.path(), 1, DEFAULT_POOL_TIMEOUT);
        pool.with(|conn| {
            conn.execute_batch("BEGIN")?;
            Ok(())
        })
        .unwrap();
        assert!(pool.with(|conn| Ok(conn.is_autocommit())).unwrap());
    }
}
