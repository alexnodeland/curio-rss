//! The append-only JSONL writer: rotation, retention, fsync-on-flush,
//! torn-tail crash recovery (see [`heal_torn_tail`]), and the
//! cross-process events lock that lets several heads share one log.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;
use curio_types::EventEnvelope;

use super::{EventLogError, file_name, parse_file_name};
use crate::lock::{FileLock, LockError};

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
        if let Some(current) = &mut self.current {
            current.file.write_all(line.as_bytes())?;
            current.bytes = current.bytes.saturating_add(line_bytes);
        }
        Ok(())
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

/// Backwards-scan chunk size used by [`heal_torn_tail`].
const HEAL_SCAN_CHUNK: u64 = 8192;

/// Repairs a torn final line — the artifact of a crash (or a partial
/// write on a full disk) mid-append — by truncating the file back to its
/// last complete line.
///
/// Only ever called with the events lock held, so the "torn" bytes can
/// never be another live process's append in flight.
///
/// Safe by the emitter's ordering contract: a line can only be torn if
/// its fsync never ran, and the fsync always happens *before* the staged
/// write-ahead intent is deleted — so the envelope on the torn line
/// still exists as an intent and is replayed in full at the next
/// startup. Without this heal, the replayed line would be glued onto the
/// torn bytes, permanently corrupting the file for every line-oriented
/// consumer (including `read_all`, which refuses invalid lines).
fn heal_torn_tail(path: &Path) -> Result<(), EventLogError> {
    let mut file = match OpenOptions::new().read(true).write(true).open(path) {
        Ok(file) => file,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err.into()),
    };
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    if last[0] == b'\n' {
        return Ok(());
    }
    // Scan backwards for the last newline; everything after it is the
    // torn line. Chunked so a large file never loads whole.
    let mut buf = vec![0u8; usize::try_from(HEAL_SCAN_CHUNK).unwrap_or(8192)];
    let mut end = len;
    let keep = loop {
        if end == 0 {
            break 0;
        }
        let start = end.saturating_sub(HEAL_SCAN_CHUNK);
        let n = usize::try_from(end - start).unwrap_or(buf.len());
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut buf[..n])?;
        if let Some(idx) = buf[..n].iter().rposition(|&b| b == b'\n') {
            break start + idx as u64 + 1;
        }
        end = start;
    };
    file.set_len(keep)?;
    file.sync_all()?;
    tracing::warn!(
        path = %path.display(),
        torn_bytes = len - keep,
        "healed torn event-log tail left by a previous crash"
    );
    Ok(())
}

/// The newest `(date, suffix)` among existing log files, if any.
fn scan_latest(dir: &Path) -> Result<Option<(NaiveDate, u32)>, EventLogError> {
    let mut latest: Option<(NaiveDate, u32)> = None;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if let Some(parsed) = parse_file_name(name)
            && latest.is_none_or(|best| parsed > best)
        {
            latest = Some(parsed);
        }
    }
    Ok(latest)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::panic)]

    use std::sync::mpsc;

    use curio_types::{CurioId, EventId, EventPayload, EventsSchemaV1Marker};

    use super::*;

    const TORN: &[u8] = b"{\"schema\":\"curio.events.v1\",\"event_id\":\"in-fli";

    fn opened(ts: &str) -> EventEnvelope {
        EventEnvelope {
            schema: EventsSchemaV1Marker,
            event_id: EventId::new(),
            ts: ts.parse().unwrap(),
            event: EventPayload::ArticleOpened {
                curio_id: CurioId::new(),
                dwell_ms: None,
            },
        }
    }

    /// The lock file the tests pair with an events dir: its sibling, as
    /// in the profile layout.
    fn lock_of(events: &Path) -> PathBuf {
        events.with_file_name("events.lock")
    }

    /// One whole line on disk in the day's file; returns its path.
    fn one_whole_line(dir: &Path) -> PathBuf {
        let mut log = EventLog::open(dir, lock_of(dir)).unwrap();
        log.append(&opened("2026-07-03T10:00:00.000Z")).unwrap();
        log.flush().unwrap();
        dir.join("events-20260703.jsonl")
    }

    /// Appends `TORN` raw — what a live process looks like halfway
    /// through an append.
    fn start_an_append(path: &Path) {
        OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap()
            .write_all(TORN)
            .unwrap();
    }

    fn ends_torn(path: &Path) -> bool {
        std::fs::read(path).unwrap().ends_with(TORN)
    }

    #[test]
    fn open_creates_exactly_the_events_dir_and_the_named_lock_file() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("events");
        let lock = dir.path().join("events.lock");
        let log = EventLog::open(&events, &lock).unwrap();
        assert_eq!(log.lock_path(), lock);
        assert!(lock.exists());
        assert_eq!(
            std::fs::read_dir(&events).unwrap().count(),
            0,
            "nothing but event files ever lands in the contract dir"
        );
        let mut created: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        created.sort();
        assert_eq!(created, ["events", "events.lock"]);
    }

    #[test]
    fn a_lock_file_in_a_missing_directory_is_an_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = EventLog::open(
            dir.path().join("events"),
            dir.path().join("nope").join("x.lock"),
        )
        .unwrap_err();
        assert!(matches!(err, EventLogError::Io(_)), "{err:?}");
    }

    /// The events lock file deleted while a long-lived handle (the
    /// desktop app) keeps it open, then a new handle (a CLI run) opened on
    /// the re-created path: the two must still exclude each other, so the
    /// old handle's heal never truncates the newcomer's append in flight.
    #[cfg(unix)]
    #[test]
    fn a_deleted_events_lock_still_serializes_old_and_new_handles() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("events");
        let path = one_whole_line(&events);
        let mut long_lived =
            EventLog::open_with_lock_timeout(&events, lock_of(&events), Duration::from_millis(50))
                .unwrap();
        std::fs::remove_file(lock_of(&events)).unwrap();
        let mut newcomer = EventLog::open(&events, lock_of(&events)).unwrap();
        newcomer
            .locked(|_| {
                start_an_append(&path);
                let err = long_lived
                    .locked(|_| Ok::<_, EventLogError>(()))
                    .unwrap_err();
                assert!(
                    matches!(err, EventLogError::ProfileLocked { .. }),
                    "{err:?}"
                );
                assert!(ends_torn(&path), "no heal under the other holder's lock");
                Ok::<_, EventLogError>(())
            })
            .unwrap();
    }

    /// Acceptance (#114): `heal_torn_tail` never truncates while another
    /// handle holds the lock — the "torn" bytes may be that handle's
    /// append in flight. The opener waits; once the holder lets go (here
    /// without finishing, as if it crashed) the heal runs.
    #[test]
    fn heal_waits_while_another_handle_holds_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("events");
        let path = one_whole_line(&events);
        let mut holder = EventLog::open(&events, lock_of(&events)).unwrap();
        let (opened_tx, opened_rx) = mpsc::channel();
        let opener = holder
            .locked(|_| {
                start_an_append(&path);
                let events = events.clone();
                let opener = std::thread::spawn(move || {
                    let log = EventLog::open_with_lock_timeout(
                        &events,
                        lock_of(&events),
                        Duration::from_secs(30),
                    );
                    opened_tx.send(()).unwrap();
                    log.map(|_| ())
                });
                std::thread::sleep(Duration::from_millis(200));
                assert!(
                    opened_rx.try_recv().is_err(),
                    "the second open must wait for the lock"
                );
                assert!(ends_torn(&path), "no truncation while the lock is held");
                Ok::<_, EventLogError>(opener)
            })
            .unwrap();
        opener.join().unwrap().unwrap();
        assert!(!ends_torn(&path), "healed once the holder let go");
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_lock_held_past_the_timeout_is_a_typed_error_and_touches_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let events = dir.path().join("events");
        let path = one_whole_line(&events);
        let mut holder = EventLog::open(&events, lock_of(&events)).unwrap();
        holder
            .locked(|_| {
                start_an_append(&path);
                let err = EventLog::open_with_lock_timeout(
                    &events,
                    lock_of(&events),
                    Duration::from_millis(50),
                )
                .unwrap_err();
                match &err {
                    EventLogError::ProfileLocked {
                        path: lock,
                        waited_ms,
                    } => {
                        assert_eq!(lock, &dir.path().join("events.lock"));
                        assert!(*waited_ms >= 50);
                    }
                    other => panic!("expected ProfileLocked, got {other:?}"),
                }
                assert!(err.to_string().contains("another Curio process"), "{err}");
                assert!(ends_torn(&path));
                Ok::<_, EventLogError>(())
            })
            .unwrap();
    }

    /// A handle that held a file open while another process rotated past
    /// it adopts the newer file instead of growing the stale one.
    #[test]
    fn a_handle_adopts_the_file_another_handle_rotated_to() {
        let root = tempfile::tempdir().unwrap();
        let events = root.path().join("events");
        let dir = events.as_path();
        let first = opened("2026-07-03T10:00:00.000Z");
        let line = u64::try_from(serde_json::to_string(&first).unwrap().len() + 1).unwrap();
        // Two lines per file.
        let mut a = EventLog::open(dir, lock_of(dir))
            .unwrap()
            .with_max_file_bytes(line * 2);
        let mut b = EventLog::open(dir, lock_of(dir))
            .unwrap()
            .with_max_file_bytes(line * 2);
        let events: Vec<_> = (1..=4)
            .map(|s| opened(&format!("2026-07-03T10:00:0{s}.000Z")))
            .collect();
        a.append(&events[0]).unwrap();
        b.append(&events[1]).unwrap();
        b.append(&events[2]).unwrap(); // rotates to -2
        a.append(&events[3]).unwrap(); // must follow into -2, not overfill the base
        a.flush().unwrap();
        b.flush().unwrap();
        let base = std::fs::read_to_string(dir.join("events-20260703.jsonl")).unwrap();
        assert_eq!(base.lines().count(), 2, "the base file is never overfilled");
        assert_eq!(
            crate::events::read_all(dir).unwrap(),
            events,
            "file order is append order"
        );
    }
}
