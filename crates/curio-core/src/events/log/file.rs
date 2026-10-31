//! File-level primitives under the event log: the torn-tail heal, the
//! rollback truncate, the (fault-injectable) line write, and the scan
//! for the newest log file.

use std::fs::{File, OpenOptions};
use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;

use chrono::NaiveDate;

use super::super::{EventLogError, parse_file_name};

/// Truncates `path` to `len` bytes and fsyncs it, through a fresh
/// write handle: the log's own handle is append-only, which on Windows
/// lacks the write-data access a truncate needs (`set_len` on it fails
/// with access denied), so the rollback can't reuse it.
pub(super) fn truncate_and_sync(path: &Path, len: u64) -> std::io::Result<()> {
    let file = OpenOptions::new().write(true).open(path)?;
    file.set_len(len)?;
    file.sync_all()
}

/// Writes one serialized line. `short_write` is the test-only fault
/// injection: write that many bytes, then fail as a full disk would.
pub(super) fn write_line(
    file: &mut File,
    line: &[u8],
    short_write: Option<usize>,
) -> std::io::Result<()> {
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
pub(super) fn heal_torn_tail(path: &Path) -> Result<(), EventLogError> {
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
pub(super) fn scan_latest(dir: &Path) -> Result<Option<(NaiveDate, u32)>, EventLogError> {
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
