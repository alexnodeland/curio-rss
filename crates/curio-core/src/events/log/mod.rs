//! The append-only JSONL writer: rotation, retention, fsync-on-flush,
//! torn-tail crash recovery (see [`heal_torn_tail`]), and the
//! cross-process events lock that lets several heads share one log.

mod file;

use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;
use curio_types::EventEnvelope;

use super::{EventLogError, file_name, parse_file_name};
use crate::lock::{FileLock, LockError};
use file::{heal_torn_tail, scan_latest, truncate_and_sync, write_line};

/// Contract rotation threshold: 50 MB.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Contract retention floor: the producer retains files at least 90 days.
pub const MIN_RETENTION_DAYS: u32 = 90;

/// How long a log operation waits for another process to release the
/// events lock before failing with [`EventLogError::ProfileLocked`].
/// Holders keep it for one drain (a few appends, one fsync, one intent
/// delete), so a wait this long means the holder is stuck.
pub const DEFAULT_LOCK_TIMEOUT: Duration = Duration::from_secs(15);

/// Append-only writer for a `.curio/events/` directory.
///
/// Files are `events-YYYYMMDD.jsonl`, named after the UTC date of their
/// first event; the writer rotates when an event's UTC date passes the
/// current file's (midnight rotation) or when the size cap would be
/// exceeded (`-2`, `-3`, … suffix). Files are append-only and never
/// rewritten. [`EventLog::flush`] is the durability point: it fsyncs
/// (`sync_all`) the current file.
///
/// **Several processes may share one directory** (the desktop app and a
/// CLI run on one profile). Every operation runs under an exclusive
/// advisory lock on a lock file the caller names at open — the profile
/// keeps it at `.curio/events.lock`, *beside* the contract directory so
/// consumers listing it never see it — and starts by re-syncing with
/// whatever the other writers did meanwhile:
/// a torn tail is healed, a file another process rotated to is adopted,
/// and the size counter is re-read from disk. Lines from different
/// processes therefore never interleave, never land in a stale `-N`
/// file, and a heal never truncates an append in flight.
#[derive(Debug)]
pub struct EventLog {
    dir: PathBuf,
    max_file_bytes: u64,
    retention_days: u32,
    lock: FileLock,
    lock_timeout: Duration,
    current: Option<OpenFile>,
    /// A failed append closed `current` without managing to truncate and
    /// fsync it, so the whole lines appended before the failure are not
    /// known durable: [`EventLog::flush`] fails until a reopen (whose
    /// next flush covers them) clears this.
    unsynced_close: bool,
    /// Test-only fault injection: `(appends to let through, bytes)` — the
    /// append after that many successes writes only `bytes` bytes of its
    /// line, then fails as a full disk would.
    #[cfg(test)]
    short_write: Option<(usize, usize)>,
}

/// An [`EventLog`] whose cross-process lock is held: the only way to
/// append or fsync. Obtained through `EventLog::locked`, which keeps
/// the lock for the whole closure so a caller can claim, append, fsync
/// and retire a batch as one critical section.
#[derive(Debug)]
pub(crate) struct LockedLog<'log> {
    log: &'log mut EventLog,
}

impl LockedLog<'_> {
    /// [`EventLog::append`] inside the held lock.
    pub(crate) fn append(&mut self, envelope: &EventEnvelope) -> Result<(), EventLogError> {
        self.log.append_held(envelope)
    }

    /// [`EventLog::flush`] inside the held lock.
    pub(crate) fn flush(&mut self) -> Result<(), EventLogError> {
        self.log.flush_held()
    }
}

#[derive(Debug)]
struct OpenFile {
    date: NaiveDate,
    suffix: u32,
    path: PathBuf,
    file: File,
    bytes: u64,
}

impl EventLog {
    /// Opens (creating if absent) the events directory `dir` and the
    /// cross-process lock file `lock_file`, waiting up to
    /// [`DEFAULT_LOCK_TIMEOUT`] for another process holding the lock.
    /// Nothing is created anywhere but those two paths.
    ///
    /// Every handle on one directory must name the same lock file — the
    /// lock is what serializes them. Keep it out of `dir` (consumers list
    /// that) and somewhere that is not cleaned while Curio runs; a lock
    /// file deleted anyway is noticed and re-created at the next
    /// operation. `lock_file`'s directory must exist.
    /// Appending resumes on the newest existing log
    /// file, so restarts continue the day's file instead of abandoning
    /// it. A torn final line left by a crash mid-append is healed here
    /// (truncated back to the last complete line — see `heal_torn_tail`)
    /// and again at the start of every later operation, always under the
    /// lock — so readers and subsequent appends only ever see whole
    /// lines, and a heal never truncates another process's append in
    /// flight.
    ///
    /// # Errors
    ///
    /// [`EventLogError::ProfileLocked`] if another process keeps the lock
    /// past the timeout; filesystem errors creating the directory or the
    /// lock file, or healing.
    pub fn open(
        dir: impl Into<PathBuf>,
        lock_file: impl Into<PathBuf>,
    ) -> Result<Self, EventLogError> {
        Self::open_with_lock_timeout(dir, lock_file, DEFAULT_LOCK_TIMEOUT)
    }

    /// [`EventLog::open`] with an explicit lock timeout, which also
    /// bounds every later operation's wait for the lock.
    ///
    /// # Errors
    ///
    /// See [`EventLog::open`].
    pub fn open_with_lock_timeout(
        dir: impl Into<PathBuf>,
        lock_file: impl Into<PathBuf>,
        lock_timeout: Duration,
    ) -> Result<Self, EventLogError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        let lock = FileLock::open(lock_file)?;
        let mut log = Self {
            dir,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            retention_days: MIN_RETENTION_DAYS,
            lock,
            lock_timeout,
            current: None,
            #[cfg(test)]
            short_write: None,
            unsynced_close: false,
        };
        // The catch-up every locked section starts with heals the tail.
        log.locked(|_| Ok::<_, EventLogError>(()))?;
        Ok(log)
    }

    /// The cross-process lock file guarding this directory.
    #[must_use]
    pub fn lock_path(&self) -> &Path {
        self.lock.path()
    }

    /// Runs `f` with the cross-process lock held, after re-syncing with
    /// what other processes wrote since this handle last held it.
    pub(crate) fn locked<T, E>(
        &mut self,
        f: impl FnOnce(&mut LockedLog<'_>) -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: From<EventLogError>,
    {
        let _guard = self
            .lock
            .acquire(self.lock_timeout)
            .map_err(|err| E::from(self.lock_error(err)))?;
        self.resync()?;
        f(&mut LockedLog { log: self })
    }

    /// Overrides the size-rotation threshold (min 1 byte). The contract
    /// value is [`DEFAULT_MAX_FILE_BYTES`]; tests use small caps.
    #[must_use]
    pub fn with_max_file_bytes(mut self, max: u64) -> Self {
        self.max_file_bytes = max.max(1);
        self
    }

    /// Sets the retention window, clamped to the contract floor of
    /// [`MIN_RETENTION_DAYS`] — a shorter window is not offered.
    #[must_use]
    pub fn with_retention_days(mut self, days: u32) -> Self {
        self.retention_days = days.max(MIN_RETENTION_DAYS);
        self
    }

    /// The events directory.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Appends one envelope as one JSONL line (under the lock), rotating
    /// first if the envelope's UTC date has passed the current file's or
    /// the size cap would be exceeded. Buffered durability: call
    /// [`EventLog::flush`] to fsync.
    ///
    /// All-or-nothing: a write that fails part-way (a full disk, an I/O
    /// error) is rolled back — the file is truncated to its last known
    /// whole-line size and closed, so the next append reopens it through
    /// the torn-tail heal. A retry therefore never glues a line onto torn
    /// bytes, and the file only ever holds whole lines.
    ///
    /// # Errors
    ///
    /// [`EventLogError::ProfileLocked`]; serialization or filesystem
    /// errors.
    pub fn append(&mut self, envelope: &EventEnvelope) -> Result<(), EventLogError> {
        self.locked(|log| log.append(envelope))
    }

    fn append_held(&mut self, envelope: &EventEnvelope) -> Result<(), EventLogError> {
        let mut line = serde_json::to_string(envelope)?;
        line.push('\n');
        let line_bytes = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let date = envelope.ts.as_datetime().date_naive();
        self.ensure_file(date, line_bytes)?;
        let short_write = self.take_short_write();
        if let Some(current) = &mut self.current {
            if let Err(err) = write_line(&mut current.file, line.as_bytes(), short_write) {
                self.roll_back_failed_append();
                return Err(err.into());
            }
            current.bytes = current.bytes.saturating_add(line_bytes);
        }
        Ok(())
    }

    /// Undoes a part-written line: closes the current file, truncates it
    /// back to its last known size (always a whole-line boundary) and
    /// fsyncs it — so the whole lines appended before the failure stay
    /// flushable. If that fails too, closing is still enough for the torn
    /// bytes (the reopen runs `heal_torn_tail`, which cuts back to the
    /// last newline), and `flush` refuses until the reopen.
    fn roll_back_failed_append(&mut self) {
        let Some(current) = self.current.take() else {
            return;
        };
        let OpenFile {
            path, file, bytes, ..
        } = current;
        drop(file);
        let rolled_back = truncate_and_sync(&path, bytes);
        self.unsynced_close = rolled_back.is_err();
        match rolled_back {
            Ok(()) => tracing::warn!(
                path = %path.display(),
                bytes,
                "event-log append failed; rolled the file back to its last whole line"
            ),
            Err(err) => tracing::warn!(
                path = %path.display(),
                error = %err,
                "event-log append failed and the rollback truncate/fsync failed; \
                 the torn tail heals when the file is reopened"
            ),
        }
    }

    #[cfg(not(test))]
    #[allow(clippy::unused_self)] // mirrors the cfg(test) fault-injection twin
    fn take_short_write(&mut self) -> Option<usize> {
        None
    }

    #[cfg(test)]
    fn take_short_write(&mut self) -> Option<usize> {
        match self.short_write.take()? {
            (0, bytes) => Some(bytes),
            (after, bytes) => {
                self.short_write = Some((after - 1, bytes));
                None
            }
        }
    }

    /// Test-only fault injection: after `after_appends` more successful
    /// appends, the next [`EventLog::append`] writes only `bytes` bytes
    /// of its line and then fails with `StorageFull` (a full disk).
    #[cfg(test)]
    pub(crate) fn inject_short_write(&mut self, after_appends: usize, bytes: usize) {
        self.short_write = Some((after_appends, bytes));
    }

    /// Fsyncs the current file (`sync_all`) — the durability point the
    /// emitter requires before an intent may be marked emitted.
    ///
    /// # Errors
    ///
    /// [`EventLogError::ProfileLocked`]; filesystem errors.
    pub fn flush(&mut self) -> Result<(), EventLogError> {
        self.locked(|log| log.log.flush_held())
    }

    fn flush_held(&mut self) -> Result<(), EventLogError> {
        if let Some(current) = &mut self.current {
            current.file.sync_all()?;
        } else if self.unsynced_close {
            return Err(std::io::Error::other(
                "the event-log file closed by a failed append was never fsynced",
            )
            .into());
        }
        Ok(())
    }

    /// Deletes log files older than the retention window, measured from
    /// `today` (UTC), under the lock. The currently open file is never
    /// swept. Returns the removed paths.
    ///
    /// # Errors
    ///
    /// [`EventLogError::ProfileLocked`]; filesystem errors scanning or
    /// deleting.
    pub fn sweep_retention(&mut self, today: NaiveDate) -> Result<Vec<PathBuf>, EventLogError> {
        self.locked(|log| log.log.sweep_held(today))
    }

    fn sweep_held(&mut self, today: NaiveDate) -> Result<Vec<PathBuf>, EventLogError> {
        let mut removed = Vec::new();
        for entry in std::fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some((date, _suffix)) = parse_file_name(name) else {
                continue;
            };
            let age_days = (today - date).num_days();
            if age_days <= i64::from(self.retention_days) {
                continue;
            }
            let path = entry.path();
            if self.current.as_ref().is_some_and(|c| c.path == path) {
                continue;
            }
            std::fs::remove_file(&path)?;
            tracing::debug!(path = %path.display(), age_days, "swept expired event log file");
            removed.push(path);
        }
        Ok(removed)
    }

    /// Catches up with other writers before a locked operation: heals a
    /// torn tail on the newest file, adopts a file another process
    /// rotated to (fsyncing ours first — it may hold unflushed lines of
    /// an earlier [`EventLog::append`]), and re-reads the size counter.
    fn resync(&mut self) -> Result<(), EventLogError> {
        let latest = scan_latest(&self.dir)?;
        if let Some((date, suffix)) = latest {
            heal_torn_tail(&self.dir.join(file_name(date, suffix)))?;
        }
        let same_file = match (&self.current, latest) {
            (Some(current), Some(latest)) => (current.date, current.suffix) == latest,
            (Some(_), None) => false,
            (None, _) => return Ok(()),
        };
        if same_file {
            if let Some(current) = &mut self.current {
                current.bytes = current.file.metadata()?.len();
            }
        } else {
            self.flush_held()?;
            self.current = None;
        }
        Ok(())
    }

    fn lock_error(&self, err: LockError) -> EventLogError {
        match err {
            LockError::TimedOut { waited } => EventLogError::ProfileLocked {
                path: self.lock.path().to_path_buf(),
                waited_ms: u64::try_from(waited.as_millis()).unwrap_or(u64::MAX),
            },
            LockError::Io(err) => EventLogError::Io(err),
        }
    }

    /// Makes sure `current` points at the right file for an event dated
    /// `date` whose serialized line is `line_bytes` long.
    fn ensure_file(&mut self, date: NaiveDate, line_bytes: u64) -> Result<(), EventLogError> {
        if self.current.is_none() {
            // Resume on the newest existing file, if any; otherwise start
            // the file for this event's date.
            let (date, suffix) = scan_latest(&self.dir)?.unwrap_or((date, 1));
            self.open_file(date, suffix)?;
        }

        // UTC-midnight rotation: the event's date has passed the file's.
        // (An *older* date — clock skew, crash-replayed intents — keeps
        // appending to the current file: files are named after their
        // first event and never rewritten.)
        if let Some(current) = &self.current
            && date > current.date
        {
            self.flush_held()?;
            self.open_file(date, 1)?;
        }

        // Size rotation: never split a line; a line larger than the cap
        // still goes out whole into a fresh file.
        while let Some(current) = &self.current {
            if current.bytes == 0 || current.bytes.saturating_add(line_bytes) <= self.max_file_bytes
            {
                break;
            }
            let (date, suffix) = (current.date, current.suffix + 1);
            self.flush_held()?;
            self.open_file(date, suffix)?;
        }
        Ok(())
    }

    fn open_file(&mut self, date: NaiveDate, suffix: u32) -> Result<(), EventLogError> {
        let path = self.dir.join(file_name(date, suffix));
        // Belt and suspenders: never glue an append onto a torn tail.
        heal_torn_tail(&path)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes = file.metadata()?.len();
        tracing::debug!(path = %path.display(), bytes, "event log file opened");
        // A file closed unsynced by a failed append was the newest, so it
        // is the one reopened here (rotation only ever moves forward from
        // it, flushing it first) — its next fsync covers the earlier lines.
        self.unsynced_close = false;
        self.current = Some(OpenFile {
            date,
            suffix,
            path,
            file,
            bytes,
        });
        Ok(())
    }
}

#[cfg(test)]
mod tests;
