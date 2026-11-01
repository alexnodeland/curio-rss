//! Maintenance: the database health probes and housekeeping the doctor
//! panels run (schema version, integrity checks, `VACUUM`, staged
//! intents), and the event-log retention sweep the facade owns.

use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use curio_types::Timestamp;

use crate::events::EventsError;

use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Path of the profile's `curio.db`.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        self.storage.path()
    }

    /// The schema version recorded in the open database (compare with
    /// [`crate::storage::Storage::schema_version`], the version this build
    /// writes).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn db_schema_version(&self) -> Result<i64, CoreError> {
        Ok(self.storage.db_schema_version()?)
    }

    /// Runs `PRAGMA integrity_check` over the database.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `IntegrityFailed` with the verdict
    /// when the check does not pass; storage errors.
    pub fn integrity_check(&self) -> Result<(), CoreError> {
        Ok(self.storage.integrity_check()?)
    }

    /// Verifies the FTS5 index matches the `articles` table.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `StorageError::Sqlite` with
    /// `SQLITE_CORRUPT_VTAB` when the index has drifted (rebuildable — never
    /// `DatabaseCorrupt`); other storage errors.
    pub fn fts_integrity_check(&self) -> Result<(), CoreError> {
        Ok(self.storage.fts_integrity_check()?)
    }

    /// Compacts the database file (`VACUUM`).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn vacuum(&self) -> Result<(), CoreError> {
        Ok(self.storage.vacuum()?)
    }

    /// How many event intents are staged in the database but not yet
    /// flushed to the JSONL log. Zero in a healthy profile: every
    /// operation flushes before it returns, and `open` replays leftovers;
    /// non-zero after a crash or a failed flush (full disk, unwritable
    /// events dir — see [`CoreHandle::has_pending_events`]).
    ///
    /// A pure read (the doctor probe): it counts every staged row, an
    /// undecodable one included, and never quarantines — that only
    /// happens when intents are drained.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn pending_intent_count(&self) -> Result<usize, CoreError> {
        let count = self.storage.pending_intent_count()?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// How many staged event intents were quarantined because their
    /// envelope no longer decoded. Zero in a healthy profile; anything
    /// else means those events never reached the JSONL log (the raw rows
    /// are kept in the database's `event_intents_dead` table).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn quarantined_intent_count(&self) -> Result<u64, CoreError> {
        Ok(self.storage.quarantined_intent_count()?)
    }

    /// The "pending events" health flag: `true` once a flush to the JSONL
    /// log failed and its intents are still staged — or the log could not
    /// be opened at all (an unwritable `.curio/events`), so nothing can be
    /// flushed yet; cleared by the next successful drain. A cheap in-memory check — heads poll it to decide
    /// whether to call [`CoreHandle::flush_pending_events`].
    #[must_use]
    pub fn has_pending_events(&self) -> bool {
        self.events_backlog.load(Ordering::Acquire)
    }

    /// Drains every staged event intent to the JSONL log now — the retry
    /// for a flush that failed earlier (mutations treat their own flush
    /// as non-fatal). Returns how many events were emitted, and updates
    /// the [`CoreHandle::has_pending_events`] flag either way.
    ///
    /// # Errors
    ///
    /// Storage or event-log errors; the intents stay staged and the flag
    /// stays raised.
    pub fn flush_pending_events(&self) -> Result<usize, CoreError> {
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        // A log that could not be opened yet is retried here, so a
        // repaired events directory drains on the next call.
        let drained = events
            .emitter()
            .and_then(|emitter| Ok(emitter.emit_pending()?));
        // Under the sink lock, so concurrent drains set the flag in the
        // order they ran.
        self.events_backlog
            .store(drained.is_err(), Ordering::Release);
        drop(events);
        drained
    }

    /// Deletes event-log files older than the retention window (≥ 90
    /// days — the `curio.events.v1` floor), measured from today (UTC).
    /// Returns the removed paths.
    ///
    /// Facade-owned on purpose: the handle owns this process's log
    /// writer (other processes on the profile are fenced off by the
    /// events lock), so heads must never open their own
    /// [`crate::events::EventLog`] to sweep.
    ///
    /// # Errors
    ///
    /// [`crate::events::EventLogError::ProfileLocked`] while another
    /// process holds the events lock; filesystem errors scanning or
    /// deleting log files.
    pub fn sweep_event_retention(&self) -> Result<Vec<PathBuf>, CoreError> {
        let today = Timestamp::now().as_datetime().date_naive();
        let removed = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .emitter()?
            .log_mut()
            .sweep_retention(today)
            .map_err(EventsError::Log)?;
        Ok(removed)
    }
}
