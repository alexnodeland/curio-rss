//! The per-feed fetch log: recording attempts and reading recent ones.

use super::rows::{parse_fetch_error_kind, parse_ts};
use crate::model::{FeedId, FetchErrorKind, FetchRecord, FetchStatus};
use crate::storage::{Storage, StorageError};

impl Storage {
    /// Appends a fetch attempt to `fetch_log`.
    ///
    /// # Errors
    ///
    /// Database errors (including a foreign-key violation for an unknown
    /// feed).
    pub fn record_fetch(&self, record: FetchRecord) -> Result<(), StorageError> {
        self.write(move |conn| {
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
