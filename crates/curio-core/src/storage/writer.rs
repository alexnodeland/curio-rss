//! The dedicated writer thread.
//!
//! One thread owns the sole read-write connection. Mutations arrive as
//! boxed jobs over an mpsc channel; each job carries its own oneshot reply
//! channel, so callers get typed results back while the channel itself
//! stays monomorphic. Dropping the [`WriterHandle`] closes the channel;
//! the thread drains what is queued, then exits and is joined.

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
                    job(&mut conn);
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

    /// Sends a job and waits for its typed reply.
    pub(crate) fn run<T, F>(&self, f: F) -> Result<T, StorageError>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> Result<T, StorageError> + Send + 'static,
    {
        let tx = self.tx.as_ref().ok_or(StorageError::WriterGone)?;
        let (reply_tx, reply_rx) = mpsc::channel::<Result<T, StorageError>>();
        let job: Job = Box::new(move |conn| {
            // A dropped receiver means the caller gave up; nothing to do.
            let _ = reply_tx.send(f(conn));
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
