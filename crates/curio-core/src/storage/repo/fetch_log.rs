//! The per-feed fetch log: recording attempts and reading recent ones.

use super::rows::parse_ts;
use crate::model::{FeedId, FetchRecord, FetchStatus};
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
                 articles_new, duration_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?
            .execute((
                record.feed_id.0,
                record.fetched_at.to_string(),
                record.status.as_str(),
                record.http_status,
                &record.error,
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
                "SELECT feed_id, fetched_at, status, http_status, error, articles_new, \
                 duration_ms FROM fetch_log WHERE feed_id = ?1 ORDER BY id DESC LIMIT ?2",
            )?;
            let raws = stmt
                .query_map((feed_id.0, limit), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<u16>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, u32>(5)?,
                        row.get::<_, Option<u32>>(6)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter()
                .map(
                    |(feed, fetched_at, status, http_status, error, articles_new, duration_ms)| {
                        Ok(FetchRecord {
                            feed_id: FeedId(feed),
                            fetched_at: parse_ts("fetch_log.fetched_at", &fetched_at)?,
                            status: FetchStatus::parse(&status).ok_or(StorageError::Corrupt {
                                column: "fetch_log.status",
                                message: format!("unknown status {status:?}"),
                            })?,
                            http_status,
                            error,
                            articles_new,
                            duration_ms,
                        })
                    },
                )
                .collect()
        })
    }
}
