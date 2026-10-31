//! The append-only JSONL writer: rotation, retention, fsync-on-flush,
//! and torn-tail crash recovery (see [`heal_torn_tail`]).

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;
use curio_types::EventEnvelope;

use super::{EventLogError, file_name, parse_file_name};

/// Contract rotation threshold: 50 MB.
pub const DEFAULT_MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;

/// Contract retention floor: the producer retains files at least 90 days.
pub const MIN_RETENTION_DAYS: u32 = 90;

/// Append-only writer for a `.curio/events/` directory.
///
/// Files are `events-YYYYMMDD.jsonl`, named after the UTC date of their
/// first event; the writer rotates when an event's UTC date passes the
/// current file's (midnight rotation) or when the size cap would be
/// exceeded (`-2`, `-3`, … suffix). Files are append-only and never
/// rewritten. [`EventLog::flush`] is the durability point: it fsyncs
/// (`sync_all`) the current file.
#[derive(Debug)]
pub struct EventLog {
    dir: PathBuf,
    max_file_bytes: u64,
    retention_days: u32,
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

#[derive(Debug)]
struct OpenFile {
    date: NaiveDate,
    suffix: u32,
    path: PathBuf,
    file: File,
    bytes: u64,
}

impl EventLog {
    /// Opens (creating if absent) the events directory. Appending resumes
    /// on the newest existing log file, so restarts continue the day's
    /// file instead of abandoning it. A torn final line left by a crash
    /// mid-append is healed here (truncated back to the last complete
    /// line — see `heal_torn_tail`), so readers and subsequent appends
    /// only ever see whole lines.
    ///
    /// # Errors
    ///
    /// Filesystem errors creating or scanning the directory.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, EventLogError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        if let Some((date, suffix)) = scan_latest(&dir)? {
            heal_torn_tail(&dir.join(file_name(date, suffix)))?;
        }
        Ok(Self {
            dir,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            retention_days: MIN_RETENTION_DAYS,
            current: None,
            #[cfg(test)]
            short_write: None,
            unsynced_close: false,
        })
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

    /// Appends one envelope as one JSONL line, rotating first if the
    /// envelope's UTC date has passed the current file's or the size cap
    /// would be exceeded. Buffered durability: call [`EventLog::flush`]
    /// to fsync.
    ///
    /// All-or-nothing: a write that fails part-way (a full disk, an I/O
    /// error) is rolled back — the file is truncated to its last known
    /// whole-line size and closed, so the next append reopens it through
    /// the torn-tail heal. A retry therefore never glues a line onto torn
    /// bytes, and the file only ever holds whole lines.
    ///
    /// # Errors
    ///
    /// Serialization or filesystem errors.
    pub fn append(&mut self, envelope: &EventEnvelope) -> Result<(), EventLogError> {
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
    /// Filesystem errors.
    pub fn flush(&mut self) -> Result<(), EventLogError> {
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
    /// `today` (UTC). The currently open file is never swept. Returns the
    /// removed paths.
    ///
    /// # Errors
    ///
    /// Filesystem errors scanning or deleting.
    pub fn sweep_retention(&mut self, today: NaiveDate) -> Result<Vec<PathBuf>, EventLogError> {
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
            self.flush()?;
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
            self.flush()?;
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

/// Truncates `path` to `len` bytes and fsyncs it, through a fresh
/// write handle: the log's own handle is append-only, which on Windows
/// lacks the write-data access a truncate needs (`set_len` on it fails
/// with access denied), so the rollback can't reuse it.
fn truncate_and_sync(path: &Path, len: u64) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(len)?;
    file.sync_all()
}

/// Writes one serialized line. `short_write` is the test-only fault
/// injection: write that many bytes, then fail as a full disk would.
fn write_line(file: &mut File, line: &[u8], short_write: Option<usize>) -> std::io::Result<()> {
    match short_write {
        None => file.write_all(line),
        Some(n) => {
            file.write_all(&line[..n.min(line.len())])?;
            Err(std::io::Error::new(
                std::io::ErrorKind::StorageFull,
                "injected short write",
            ))
        }
    }
}

/// Backwards-scan chunk size used by [`heal_torn_tail`].
const HEAL_SCAN_CHUNK: u64 = 8192;

/// Repairs a torn final line — the artifact of a crash (or a partial
/// write on a full disk) mid-append — by truncating the file back to its
/// last complete line.
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
    #![allow(clippy::unwrap_used)]

    use curio_types::{CurioId, EventId, EventPayload, EventsSchemaV1Marker};

    use super::*;
    use crate::events::read_all;

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

    /// Asserts the file is a sequence of whole lines — newline-terminated,
    /// every line a parseable envelope — and returns them.
    fn whole_lines(path: &Path) -> Vec<String> {
        let raw = std::fs::read_to_string(path).unwrap();
        assert!(raw.is_empty() || raw.ends_with('\n'), "torn tail:\n{raw}");
        raw.lines()
            .map(|line| {
                serde_json::from_str::<EventEnvelope>(line).unwrap();
                line.to_owned()
            })
            .collect()
    }

    /// #118: an append that dies mid-line (ENOSPC) must not leave torn
    /// bytes behind for the in-process retry to extend.
    #[test]
    fn a_failed_append_is_rolled_back_and_the_retry_writes_one_whole_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events-20260703.jsonl");
        let mut log = EventLog::open(dir.path()).unwrap();
        let first = opened("2026-07-03T10:00:00.000Z");
        log.append(&first).unwrap();
        log.flush().unwrap();

        let second = opened("2026-07-03T10:00:01.000Z");
        log.inject_short_write(0, 17);
        let err = log.append(&second).unwrap_err();
        assert!(
            matches!(&err, EventLogError::Io(io) if io.kind() == std::io::ErrorKind::StorageFull),
            "{err}"
        );
        // Nothing torn survives the failure itself...
        assert_eq!(whole_lines(&path).len(), 1);

        // ...and the retry appends exactly one whole copy.
        log.append(&second).unwrap();
        log.flush().unwrap();
        let lines = whole_lines(&path);
        assert_eq!(lines.len(), 2, "no duplicate, no partial: {lines:?}");
        assert_eq!(read_all(dir.path()).unwrap(), vec![first, second]);
    }

    /// If the rollback itself could not truncate + fsync, the lines that
    /// landed before the failure are not known durable: `flush` must not
    /// report success (the emitter would retire their intents) until the
    /// file is reopened and fsynced again.
    #[test]
    fn flush_refuses_after_an_unsynced_rollback_until_the_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let mut log = EventLog::open(dir.path()).unwrap();
        let first = opened("2026-07-03T10:00:00.000Z");
        log.append(&first).unwrap();
        // The state roll_back_failed_append leaves when set_len/sync_all
        // fail: file closed, durability unknown.
        log.current = None;
        log.unsynced_close = true;
        log.flush().unwrap_err();

        let second = opened("2026-07-03T10:00:01.000Z");
        log.append(&second).unwrap();
        log.flush().unwrap();
        assert_eq!(read_all(dir.path()).unwrap(), vec![first, second]);
    }

    /// A failure on the very first line of a fresh file rolls back to empty.
    #[test]
    fn a_failed_first_append_leaves_an_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("events-20260703.jsonl");
        let mut log = EventLog::open(dir.path()).unwrap();
        let only = opened("2026-07-03T10:00:00.000Z");
        log.inject_short_write(0, 5);
        log.append(&only).unwrap_err();
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

        log.append(&only).unwrap();
        log.flush().unwrap();
        assert_eq!(whole_lines(&path).len(), 1);
        assert_eq!(read_all(dir.path()).unwrap(), vec![only]);
    }
}
