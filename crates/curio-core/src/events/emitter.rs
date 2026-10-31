//! The storage-coupled emitter: intents → JSONL, crash-recoverably.
//!
//! Ordering contract (the "transactional-ish" pair):
//!
//! 1. the DB state change commits first, *with* its write-ahead intent row
//!    in the same transaction (the storage repo does this);
//! 2. the emitter takes the cross-process events lock (see [`EventLog`]);
//! 3. under it, the emitter claims the staged intents, appends their
//!    envelopes to the JSONL log, and fsyncs the log;
//! 4. only then — still under the lock — are the intent rows deleted.
//!
//! Crash between 1 and 4 → the intent survives and is replayed by
//! [`EventEmitter::new`] at the next startup. Crash between 3 and 4 → the
//! replay appends a duplicate line, which consumers drop by `event_id`
//! dedupe. No window loses an event.
//!
//! Claim + append + fsync + delete is one critical section, so the claim
//! is atomic across processes: when the desktop app and a CLI run share a
//! profile, an intent one of them staged is drained by one of them at a
//! time — whichever takes the lock first sees it, and the other sees it
//! already deleted. (Deleting at claim time instead, `DELETE … RETURNING`
//! style, would open a crash window that *loses* the event.)
//!
//! Delivery is still **at-least-once**, never exactly-once: the delete in
//! step 4 goes through this process's writer and can fail after step 3
//! made the lines durable — a crash, or [`StorageError::Busy`] when
//! another process holds a database write past the busy timeout. The
//! intents then stay staged and the next drain (in either process)
//! appends them again; consumers drop the duplicate by `event_id`, as the
//! contract requires. The lock is also held while that delete waits
//! behind this process's queued write jobs, so a long one (a vacuum, a
//! big import) delays other processes' emits.
//!
//! [`StorageError::Busy`]: crate::storage::StorageError::Busy

use std::sync::Arc;

use super::{EventLog, EventsError};
use crate::storage::Storage;

/// Drains staged event intents from [`Storage`] into an [`EventLog`].
#[derive(Debug)]
pub struct EventEmitter {
    storage: Arc<Storage>,
    log: EventLog,
}

impl EventEmitter {
    /// Couples a storage handle to a log and immediately replays any
    /// intents staged but never emitted — by a previous process that
    /// crashed, or by a live one about to drain them itself (the lock
    /// makes sure the two never drain the same intents at once).
    ///
    /// # Errors
    ///
    /// Storage or log errors during the recovery replay.
    pub fn new(storage: Arc<Storage>, log: EventLog) -> Result<Self, EventsError> {
        let mut emitter = Self { storage, log };
        let replayed = emitter.emit_pending()?;
        if replayed > 0 {
            tracing::info!(replayed, "replayed unemitted event intents at startup");
        }
        Ok(emitter)
    }

    /// Under the events lock: appends every staged intent to the log
    /// (oldest first), fsyncs, then deletes the intents. Returns how many
    /// events were emitted.
    ///
    /// # Errors
    ///
    /// [`super::EventLogError::ProfileLocked`] when another process holds
    /// the lock past its timeout; storage or log errors. On failure the
    /// intents stay staged and the next call (or startup) retries them.
    pub fn emit_pending(&mut self) -> Result<usize, EventsError> {
        let storage = &self.storage;
        self.log.locked(|log| {
            let pending = storage.pending_intents()?;
            if pending.is_empty() {
                return Ok(0);
            }
            for intent in &pending {
                log.append(&intent.envelope)?;
            }
            // Durability point: the fsync happens BEFORE the intents are
            // deleted, so a crash can only ever duplicate, never lose.
            log.flush()?;
            storage.mark_intents_emitted(pending.iter().map(|p| p.intent_id).collect())?;
            Ok(pending.len())
        })
    }

    /// The underlying log (rotation config, retention sweeps).
    pub fn log_mut(&mut self) -> &mut EventLog {
        &mut self.log
    }
}
