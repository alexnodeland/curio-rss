//! The per-feed fetch log: recording attempts (bounded per feed) and
//! reading recent ones.

use rusqlite::Connection;

use super::rows::{parse_fetch_error_kind, parse_ts};
use super::{FeedSchedule, begin_write};
use crate::model::{FeedId, FeedStatus, FetchErrorKind, FetchRecord, FetchStatus};
use crate::storage::{Storage, StorageError};

/// How many `fetch_log` rows each feed keeps: every insert trims the
/// feed's older rows, so the log (and the feed list's health subqueries
/// over it) stay bounded however often a feed is refreshed.
pub const FETCH_LOG_KEEP: u32 = 50;

impl Storage {
    /// Appends a fetch attempt to `fetch_log`, trimming the feed's log to
    /// its newest [`FETCH_LOG_KEEP`] rows.
    ///
    /// # Errors
    ///
    /// Database errors (including a foreign-key violation for an unknown
    /// feed).
    pub fn record_fetch(&self, record: FetchRecord) -> Result<(), StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            insert_fetch(&tx, &record)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Settles one fetch attempt in one transaction: the `fetch_log` row
    /// (with the per-feed trim of [`Self::record_fetch`]) and the feed's
    /// scheduling state — failure count, next attempt, failing-since and,
    /// when the attempt caused one, the lifecycle change (an auto-pause
    /// stamps `auto_paused_at`).
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist; database
    /// errors.
    pub fn settle_fetch(
        &self,
        record: FetchRecord,
        schedule: FeedSchedule,
    ) -> Result<(), StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let auto_paused_at = (schedule.status == Some(FeedStatus::Paused))
                .then(|| record.fetched_at.to_string());
            let n = tx
                .prepare_cached(
                    "UPDATE feeds SET consecutive_failures = ?2, next_attempt_at = ?3, \
                     failing_since = ?4, status = COALESCE(?5, status), \
                     auto_paused_at = COALESCE(?6, auto_paused_at) WHERE id = ?1",
                )?
                .execute((
                    record.feed_id.0,
                    schedule.consecutive_failures,
                    schedule.next_attempt_at.map(|at| at.to_string()),
                    schedule.failing_since.map(|at| at.to_string()),
                    schedule.status.map(FeedStatus::as_str),
                    auto_paused_at,
                ))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            insert_fetch(&tx, &record)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// The most recent fetch attempts for a feed, newest first.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn recent_fetches(
        &self,
        feed_id: FeedId,
        limit: u32,
    ) -> Result<Vec<FetchRecord>, StorageError> {
        self.read(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT feed_id, fetched_at, status, http_status, error, error_kind, \
                 articles_new, duration_ms FROM fetch_log WHERE feed_id = ?1 \
                 ORDER BY id DESC LIMIT ?2",
            )?;
            let raws = stmt
                .query_map((feed_id.0, limit), |row| {
                    Ok(RawFetch {
                        feed: row.get(0)?,
                        fetched_at: row.get(1)?,
                        status: row.get(2)?,
                        http_status: row.get(3)?,
                        error: row.get(4)?,
                        error_kind: row.get(5)?,
                        articles_new: row.get(6)?,
                        duration_ms: row.get(7)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter().map(RawFetch::into_record).collect()
        })
    }
}

/// One `fetch_log` row as stored, before its TEXT columns are parsed.
struct RawFetch {
    feed: i64,
    fetched_at: String,
    status: String,
    http_status: Option<u16>,
    error: Option<String>,
    error_kind: Option<String>,
    articles_new: u32,
    duration_ms: Option<u32>,
}

impl RawFetch {
    fn into_record(self) -> Result<FetchRecord, StorageError> {
        Ok(FetchRecord {
            feed_id: FeedId(self.feed),
            fetched_at: parse_ts("fetch_log.fetched_at", &self.fetched_at)?,
            status: FetchStatus::parse(&self.status).ok_or_else(|| StorageError::Corrupt {
                column: "fetch_log.status",
                message: format!("unknown status {:?}", self.status),
            })?,
            http_status: self.http_status,
            error: self.error,
            error_kind: parse_fetch_error_kind(self.error_kind.as_deref()),
            articles_new: self.articles_new,
            duration_ms: self.duration_ms,
        })
    }
}

/// Inserts one `fetch_log` row and trims the feed's log to its newest
/// [`FETCH_LOG_KEEP`] rows (the trim is a no-op until the log is full).
fn insert_fetch(conn: &Connection, record: &FetchRecord) -> Result<(), StorageError> {
    conn.prepare_cached(
        "INSERT INTO fetch_log (feed_id, fetched_at, status, http_status, error, \
         error_kind, articles_new, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute((
        record.feed_id.0,
        record.fetched_at.to_string(),
        record.status.as_str(),
        record.http_status,
        &record.error,
        record.error_kind.map(FetchErrorKind::as_str),
        record.articles_new,
        record.duration_ms,
    ))?;
    conn.prepare_cached(
        "DELETE FROM fetch_log WHERE feed_id = ?1 AND id < (SELECT id FROM fetch_log \
         WHERE feed_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET ?2)",
    )?
    .execute((record.feed_id.0, FETCH_LOG_KEEP - 1))?;
    Ok(())
}
