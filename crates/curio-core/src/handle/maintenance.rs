//! Event-log maintenance: the retention sweep the facade owns.

use std::path::PathBuf;
use std::sync::PoisonError;

use curio_types::Timestamp;

use crate::events::EventsError;

use super::{CoreError, CoreHandle};

impl CoreHandle {
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
