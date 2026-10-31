//! The handle's event sink: the [`EventEmitter`], attached lazily.
//!
//! An events directory that cannot be opened (a full or read-only disk, a
//! file squatting `.curio/events`, a `.curio/` the user cannot write) is
//! an *emission* failure, not an open failure: the database is fine, so
//! the profile opens, every mutation still commits with its intent
//! staged, and the handle raises its backlog flag. Each later drain
//! retries the attach, so once the directory is repaired the next drain
//! opens the log and replays the backlog in order.
//!
//! Contention is the one log failure `open` still reports: another
//! process holding the events lock past its timeout
//! ([`crate::events::EventLogError::ProfileLocked`]) is retryable, and
//! heads already turn it into "busy, try again" (see
//! [`super::CoreError::is_profile_busy`]).

use std::path::PathBuf;
use std::sync::Arc;

use crate::events::{EventEmitter, EventLog, EventsError};
use crate::storage::Storage;

use super::{CoreError, EVENTS_LOCK_FILE, ensure_gitignore};

/// The emitter once its log is open, plus what it takes to open it.
#[derive(Debug)]
pub(super) struct EventSink {
    storage: Arc<Storage>,
    /// The profile's `.curio/` directory (events dir, lock, gitignore).
    state_dir: PathBuf,
    emitter: Option<EventEmitter>,
}

impl EventSink {
    /// A sink with nothing opened yet.
    pub(super) fn detached(storage: Arc<Storage>, state_dir: PathBuf) -> Self {
        Self {
            storage,
            state_dir,
            emitter: None,
        }
    }

    /// The emitter, opening the events log first if no earlier call
    /// managed to.
    ///
    /// # Errors
    ///
    /// Whatever kept the log from opening: the events lock held by
    /// another process, or a filesystem error creating the directory,
    /// the lock file or `.curio/.gitignore`. The next call tries again.
    pub(super) fn emitter(&mut self) -> Result<&mut EventEmitter, CoreError> {
        let emitter = match self.emitter.take() {
            Some(emitter) => emitter,
            None => self.attach()?,
        };
        Ok(self.emitter.insert(emitter))
    }

    fn attach(&self) -> Result<EventEmitter, CoreError> {
        let log = EventLog::open(
            self.state_dir.join("events"),
            self.state_dir.join(EVENTS_LOCK_FILE),
        )
        .map_err(EventsError::Log)?;
        // Contract: the event log is never committed to git — nor is the
        // cross-process lock file guarding it. No line is appended until
        // the ignore file says so.
        ensure_gitignore(&self.state_dir.join(".gitignore"))?;
        Ok(EventEmitter::attach(Arc::clone(&self.storage), log))
    }
}
