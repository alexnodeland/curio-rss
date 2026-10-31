//! The dedicated writer thread.
//!
//! One thread owns the sole read-write connection. Mutations arrive as
//! boxed jobs over an mpsc channel; each job carries its own oneshot reply
//! channel, so callers get typed results back while the channel itself
//! stays monomorphic. Dropping the [`WriterHandle`] closes the channel;
//! the thread drains what is queued, then exits and is joined.
//!
//! A panicking job never takes the thread down: each job runs under
//! `catch_unwind`, its caller gets [`StorageError::JobPanicked`], any
//! transaction the job left open is rolled back, and the loop carries on
//! with the next job — one bug in one mutation must not turn every later
//! write into [`StorageError::WriterGone`] until the app restarts.

use std::any::Any;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::mpsc;
use std::thread::JoinHandle;

use rusqlite::Connection;

use super::StorageError;

/// A unit of work for the writer thread.
type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// Handle to the writer thread. Cloneless by design — it lives inside
/// [`super::Storage`], which is the shared handle.
pub(crate) struct WriterHandle {
    tx: Option<mpsc::Sender<Job>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for WriterHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterHandle")
            .field("alive", &self.thread.is_some())
            .finish_non_exhaustive()
    }
}

impl WriterHandle {
    /// Moves `conn` onto a fresh writer thread and returns the handle.
    pub(crate) fn spawn(mut conn: Connection) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("curio-storage-writer".to_owned())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    // `run` already catches panics inside the job body;
                    // this outer guard is defense in depth so nothing a
                    // job does (a panicking reply send, a future job
                    // shape) can end the loop.
                    let unwound = catch_unwind(AssertUnwindSafe(|| job(&mut conn))).is_err();
                    if unwound {
                        tracing::error!("a storage writer job panicked outside its reply guard");
                    }
                    rollback_if_open(&conn);
                }
                if let Err((_conn, err)) = conn.close() {
                    tracing::warn!(error = %err, "closing the writer connection failed");
                }
                tracing::debug!("storage writer thread exited");
            });
        match thread {
            Ok(handle) => Self {
                tx: Some(tx),
                thread: Some(handle),
            },
            Err(err) => {
                // Out-of-threads at open time: leave a handle whose every
                // call reports WriterGone instead of aborting the process.
                tracing::error!(error = %err, "failed to spawn the storage writer thread");
                Self {
                    tx: None,
                    thread: None,
                }
            }
        }
    }

    /// Sends a job and waits for its typed reply. A job that panics
    /// replies [`StorageError::JobPanicked`]; the writer keeps serving.
    pub(crate) fn run<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    {
        let tx = self.tx.as_ref().ok_or(StorageError::WriterGone)?;
        let (reply_tx, reply_rx) = mpsc::channel::<Result<T, StorageError>>();
        let job: Job = Box::new(move |conn| {
            // AssertUnwindSafe: the only state the closure can leave
            // inconsistent is the connection's transaction, and the loop
            // rolls back any transaction still open after every job.
            let result = catch_unwind(AssertUnwindSafe(|| f(conn))).unwrap_or_else(|payload| {
                let message = panic_message(payload.as_ref());
                tracing::error!(%message, "storage writer job panicked; the writer keeps running");
                Err(StorageError::JobPanicked { message })
            });
            // A dropped receiver means the caller gave up; nothing to do.
            let _ = reply_tx.send(result);
        });
        tx.send(job).map_err(|_| StorageError::WriterGone)?;
        reply_rx.recv().map_err(|_| StorageError::WriterGone)?
    }
}

impl Drop for WriterHandle {
    fn drop(&mut self) {
        // Close the channel first so the thread's recv loop ends, then join.
        drop(self.tx.take());
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("storage writer thread panicked");
        }
    }
}

/// Rolls back a transaction a job left open (it panicked mid-way, or
/// issued a bare `BEGIN` it never closed), so the next job starts from
/// autocommit instead of inheriting half a write.
pub(super) fn rollback_if_open(conn: &Connection) {
    if conn.is_autocommit() {
        return;
    }
    if let Err(err) = conn.execute_batch("ROLLBACK") {
        tracing::error!(error = %err, "rolling back a transaction left open by a storage job failed");
    } else {
        tracing::warn!("rolled back a transaction left open by a storage job");
    }
}

/// The human-readable part of a panic payload (`panic!("…")` carries a
/// `&str` or a `String`; anything else is opaque).
pub(super) fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use super::*;

    fn writer() -> WriterHandle {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t (v INTEGER NOT NULL)")
            .unwrap();
        WriterHandle::spawn(conn)
    }

    fn count(writer: &WriterHandle) -> i64 {
        writer
            .run(|conn| Ok(conn.query_row("SELECT COUNT(*) FROM t", [], |row| row.get(0))?))
            .unwrap()
    }

    #[test]
    fn a_panicking_job_is_followed_by_a_successful_write() {
        let writer = writer();
        let err = writer
            .run(|_conn| -> Result<(), StorageError> { panic!("boom in a job") })
            .unwrap_err();
        match err {
            StorageError::JobPanicked { message } => assert_eq!(message, "boom in a job"),
            other => panic!("expected JobPanicked, got {other:?}"),
        }
        writer
            .run(|conn| {
                conn.execute("INSERT INTO t (v) VALUES (1)", [])?;
                Ok(())
            })
            .unwrap();
        assert_eq!(count(&writer), 1);
    }

    #[test]
    fn a_job_panicking_mid_transaction_is_rolled_back() {
        let writer = writer();
        let err = writer
            .run(|conn| -> Result<(), StorageError> {
                let tx = conn.transaction()?;
                tx.execute("INSERT INTO t (v) VALUES (1)", [])?;
                let owned = String::from("panic with a String payload");
                std::panic::panic_any(owned)
            })
            .unwrap_err();
        assert!(
            matches!(&err, StorageError::JobPanicked { message } if message.contains("String payload")),
            "{err:?}"
        );
        assert_eq!(count(&writer), 0, "the half-done insert never committed");
    }

    #[test]
    fn a_transaction_left_open_without_a_panic_is_rolled_back_too() {
        let writer = writer();
        writer
            .run(|conn| {
                conn.execute_batch("BEGIN; INSERT INTO t (v) VALUES (7);")?;
                Ok(())
            })
            .unwrap();
        // The next job starts in autocommit: the dangling BEGIN is gone.
        assert!(writer.run(|conn| Ok(conn.is_autocommit())).unwrap());
        assert_eq!(count(&writer), 0);
    }

    #[test]
    fn opaque_panic_payloads_still_report() {
        let writer = writer();
        let err = writer
            .run(|_conn| -> Result<(), StorageError> { std::panic::panic_any(42_u8) })
            .unwrap_err();
        assert!(
            matches!(&err, StorageError::JobPanicked { message } if message == "non-string panic payload"),
            "{err:?}"
        );
        assert_eq!(count(&writer), 0);
    }
}
