//! The read-only connection pool.
//!
//! A fixed set of `SQLITE_OPEN_READ_ONLY` connections checked out one at a
//! time. Under WAL, readers never block the writer and the writer never
//! blocks readers — the pool exists only so N queries can run in parallel.
//! The `Mutex` here guards the *free list*, never a connection in use;
//! this is exactly what the banned `Mutex<Connection>` pattern was not.

use std::path::Path;
use std::sync::{Condvar, Mutex, PoisonError};

use rusqlite::{Connection, OpenFlags};

use super::StorageError;

pub(crate) struct ReadPool {
    free: Mutex<Vec<Connection>>,
    available: Condvar,
}

impl std::fmt::Debug for ReadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReadPool").finish_non_exhaustive()
    }
}

impl ReadPool {
    /// Opens `size` read-only connections against the (already migrated)
    /// database at `path`.
    pub(crate) fn open(path: &Path, size: usize) -> Result<Self, StorageError> {
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI;
        let mut free = Vec::with_capacity(size);
        for _ in 0..size {
            let conn = Connection::open_with_flags(path, flags)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            // Defense in depth on top of the read-only open flag.
            conn.pragma_update(None, "query_only", true)?;
            free.push(conn);
        }
        Ok(Self {
            free: Mutex::new(free),
            available: Condvar::new(),
        })
    }

    /// Checks a connection out, runs `f`, and returns the connection to
    /// the pool (also on error).
    pub(crate) fn with<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, StorageError>,
    ) -> Result<T, StorageError> {
        let conn = self.checkout();
        let out = f(&conn);
        self.checkin(conn);
        out
    }

    fn checkout(&self) -> Connection {
        let mut free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        loop {
            if let Some(conn) = free.pop() {
                return conn;
            }
            free = self
                .available
                .wait(free)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    fn checkin(&self, conn: Connection) {
        self.free
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(conn);
        self.available.notify_one();
    }
}
