//! The `curio.events.v1` append-only behavioral event log.
//!
//! Per profile, `.curio/events/` holds append-only JSONL files named
//! `events-YYYYMMDD.jsonl` (UTC date of the file's first event), rotated at
//! UTC midnight or 50 MB — whichever first, size overflow adding a `-2`,
//! `-3`, … suffix — retained ≥ 90 days, and **never committed to git**.
//! One JSON envelope per line; consumers dedupe by ULID `event_id` and
//! fold negation events (histories are not monotone).
//!
//! Emission is transactional-ish: the storage layer commits each state
//! change together with a write-ahead *intent* row; [`EventEmitter`]
//! appends staged envelopes to the JSONL log, fsyncs, then deletes the
//! intents. A crash between append and delete replays the envelope at the
//! next startup — safe, because consumers dedupe by `event_id`.

mod emitter;
mod fold;
mod log;
mod reader;

use std::path::PathBuf;

use chrono::NaiveDate;

pub use emitter::EventEmitter;
pub use fold::{FeedSubscription, FoldedState};
pub use log::{DEFAULT_MAX_FILE_BYTES, EventLog, MIN_RETENTION_DAYS};
pub use reader::read_all;

use crate::storage::StorageError;

/// Errors of the JSONL log itself (writer and reader).
#[derive(Debug, thiserror::Error)]
pub enum EventLogError {
    /// Filesystem failure.
    #[error("event log io: {0}")]
    Io(#[from] std::io::Error),
    /// An envelope failed to serialize.
    #[error("event serialization: {0}")]
    Serde(#[from] serde_json::Error),
    /// A stored line failed to parse back into an envelope.
    #[error("{}:{line}: invalid event line: {message}", path.display())]
    InvalidLine {
        /// The file holding the bad line.
        path: PathBuf,
        /// 1-indexed line number.
        line: u64,
        /// Parse-failure detail.
        message: String,
    },
}

/// Errors of the storage-coupled emission path.
#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    /// The storage side (intents) failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// The JSONL side failed.
    #[error(transparent)]
    Log(#[from] EventLogError),
}

/// Builds the contract file name for a date + rotation suffix (suffix 1 is
/// the unsuffixed base file).
pub(crate) fn file_name(date: NaiveDate, suffix: u32) -> String {
    let date = date.format("%Y%m%d");
    if suffix <= 1 {
        format!("events-{date}.jsonl")
    } else {
        format!("events-{date}-{suffix}.jsonl")
    }
}

/// Parses a contract file name back into `(date, suffix)`; `None` for
/// foreign files (which the reader and the retention sweep both ignore).
pub(crate) fn parse_file_name(name: &str) -> Option<(NaiveDate, u32)> {
    let stem = name.strip_prefix("events-")?.strip_suffix(".jsonl")?;
    let (date_part, suffix) = match stem.split_once('-') {
        Some((date_part, suffix_part)) => {
            let suffix: u32 = suffix_part.parse().ok()?;
            if suffix < 2 {
                return None;
            }
            (date_part, suffix)
        }
        None => (stem, 1),
    };
    if date_part.len() != 8 {
        return None;
    }
    let date = NaiveDate::parse_from_str(date_part, "%Y%m%d").ok()?;
    Some((date, suffix))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn file_names_round_trip() {
        let d = date(2026, 7, 3);
        assert_eq!(file_name(d, 1), "events-20260703.jsonl");
        assert_eq!(file_name(d, 2), "events-20260703-2.jsonl");
        assert_eq!(parse_file_name("events-20260703.jsonl"), Some((d, 1)));
        assert_eq!(parse_file_name("events-20260703-2.jsonl"), Some((d, 2)));
        assert_eq!(parse_file_name("events-20260703-10.jsonl"), Some((d, 10)));
    }

    #[test]
    fn foreign_files_are_ignored() {
        assert_eq!(parse_file_name("events-20260703-1.jsonl"), None);
        assert_eq!(parse_file_name("events-20260703-0.jsonl"), None);
        assert_eq!(parse_file_name("events-2026073.jsonl"), None);
        assert_eq!(parse_file_name("cursor.json"), None);
        assert_eq!(parse_file_name("events-notadate.jsonl"), None);
        assert_eq!(parse_file_name("events-20260703.jsonl.bak"), None);
    }

    #[test]
    fn suffix_ordering_is_numeric_not_lexicographic() {
        let mut names = vec![
            parse_file_name("events-20260703-10.jsonl").unwrap(),
            parse_file_name("events-20260703-2.jsonl").unwrap(),
            parse_file_name("events-20260703.jsonl").unwrap(),
            parse_file_name("events-20260702.jsonl").unwrap(),
        ];
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                (date(2026, 7, 2), 1),
                (date(2026, 7, 3), 1),
                (date(2026, 7, 3), 2),
                (date(2026, 7, 3), 10),
            ]
        );
    }
}
