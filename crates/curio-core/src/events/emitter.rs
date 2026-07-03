//! The storage-coupled emitter: intents → JSONL, crash-recoverably.
//!
//! Ordering contract (the "transactional-ish" pair):
//!
//! 1. the DB state change commits first, *with* its write-ahead intent row
//!    in the same transaction (the storage repo does this);
//! 2. the emitter appends the staged envelope(s) to the JSONL log;
//! 3. the emitter fsyncs the log ([`EventLog::flush`]);
//! 4. only then are the intent rows deleted.
//!
//! Crash between 1 and 4 → the intent survives and is replayed by
//! [`EventEmitter::new`] at the next startup. Crash between 3 and 4 → the
//! replay appends a duplicate line, which consumers drop by `event_id`
//! dedupe. No window loses an event.

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
    /// intents a previous process staged but never emitted.
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

    /// Appends every staged intent to the log (oldest first), fsyncs,
    /// then deletes the intents. Returns how many events were emitted.
    ///
    /// # Errors
    ///
    /// Storage or log errors; on failure the intents stay staged and the
    /// next call (or startup) retries them.
    pub fn emit_pending(&mut self) -> Result<usize, EventsError> {
        let pending = self.storage.pending_intents()?;
        if pending.is_empty() {
            return Ok(0);
        }
        for intent in &pending {
            self.log.append(&intent.envelope)?;
        }
        // Durability point: the fsync happens BEFORE the intents are
        // deleted, so a crash can only ever duplicate, never lose.
        self.log.flush()?;
        self.storage
            .mark_intents_emitted(pending.iter().map(|p| p.intent_id).collect())?;
        Ok(pending.len())
    }

    /// The underlying log (rotation config, retention sweeps).
    pub fn log_mut(&mut self) -> &mut EventLog {
        &mut self.log
    }
}
