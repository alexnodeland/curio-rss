//! Maintenance: the database health probes and housekeeping the doctor
//! panels run (schema version, integrity checks, `VACUUM`, staged
//! intents), and the event-log retention sweep the facade owns.

use std::path::{Path, PathBuf};
use std::sync::PoisonError;

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
    /// [`CoreError::Storage`] wrapping `IntegrityFailed` when the index has
    /// drifted; storage errors.
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
    /// operation flushes before it returns, and `open` replays leftovers.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn pending_intent_count(&self) -> Result<usize, CoreError> {
        Ok(self.storage.pending_intents()?.len())
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

    /// Deletes event-log files older than the retention window (≥ 90
    /// days — the `curio.events.v1` floor), measured from today (UTC).
    /// Returns the removed paths.
    ///
    /// Facade-owned on purpose: the event log is single-writer and the
    /// handle owns that writer, so heads must never open their own
    /// [`crate::events::EventLog`] to sweep.
    ///
    /// # Errors
    ///
    /// Filesystem errors scanning or deleting log files.
    pub fn sweep_event_retention(&self) -> Result<Vec<PathBuf>, CoreError> {
        let today = Timestamp::now().as_datetime().date_naive();
        let removed = self
            .emitter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .log_mut()
            .sweep_retention(today)
            .map_err(EventsError::Log)?;
        Ok(removed)
    }
}
