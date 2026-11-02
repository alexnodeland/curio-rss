//! Routine maintenance: the article-retention reap (with its tombstones
//! and the export records that protect), the orphan purge, unused-tag
//! and fetch-log housekeeping, the search index's merge/`rebuild`, and
//! the planner/WAL upkeep.

use std::collections::BTreeSet;

use curio_types::{CurioId, Timestamp};
use rusqlite::{Connection, OptionalExtension as _};

use super::begin_write;
use super::fetch_log::FETCH_LOG_KEEP;
use crate::model::{ArticleId, RetentionPolicy};
use crate::storage::{Storage, StorageError};

/// One day in milliseconds — how often a tombstone hit re-stamps
/// `seen_at` at most, and the slack its pruning allows on top of that.
const DAY_MS: i64 = 86_400_000;

/// The most search-index pages one pass's incremental FTS5 merge writes.
/// The budget is passed negative (`'merge', -N`): every segment then
/// counts as one level, so successive passes keep folding the index
/// together — what deletions left behind included — a bounded step at a
/// time.
///
/// A full `optimize` rewrites the whole index under the write lock
/// instead: on a 100k-article profile (650 MB, 1 000 articles just
/// deleted) it took ~6 s and grew the WAL by ~240 MB, where this budget
/// took ~0.2 s and ~9 MB. A positive budget did nothing there: the
/// automerge every insert runs already keeps the levels in shape, so it
/// never reclaims what deletions leave behind.
const FTS_MERGE_PAGES: i64 = 2_000;

/// What one maintenance pass reaps, beyond the housekeeping it always does
/// (see [`Storage::run_maintenance`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenancePlan {
    /// The article-retention policy; a disabled one reaps nothing.
    pub retention: RetentionPolicy,
    /// Also delete the unprotected articles a feed removal left behind.
    pub purge_orphans: bool,
    /// Articles the destinations' manifests list as exported. The pass
    /// records them as exports first (as
    /// [`Storage::mark_article_exported`] does), so neither reap touches
    /// them — in this pass or any later one.
    pub exported: BTreeSet<CurioId>,
    /// The pass's clock: the age cutoff and tombstone stamps are relative
    /// to it.
    pub now: Timestamp,
}

/// What one maintenance pass removed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MaintenanceCounts {
    /// Feed articles the retention policy reaped.
    pub articles_reaped: u64,
    /// Orphaned articles (their feed was removed) purged on request.
    pub orphans_purged: u64,
    /// Tags no article carries any more.
    pub tags_dropped: u64,
    /// Tombstones forgotten because their feed stopped listing the item.
    pub tombstones_pruned: u64,
    /// `fetch_log` rows beyond the per-feed cap.
    pub fetch_log_trimmed: u64,
    /// Why the upkeep after the commit (the search-index merge, `PRAGMA
    /// optimize`, the WAL checkpoint) stopped short — another process
    /// held the write lock past the busy timeout, say. What the
    /// transaction deleted stays deleted and is counted above.
    pub upkeep_skipped: Option<String>,
}

impl Storage {
    /// One maintenance pass. In one transaction:
    ///
    /// - records the stored articles in `plan.exported` as exports;
    /// - trims every feed's `fetch_log` to its newest [`FETCH_LOG_KEEP`]
    ///   rows (inserts already do; this catches logs grown otherwise);
    /// - applies `plan.retention` to feed articles, never touching a
    ///   protected one (starred, read-later, tagged, ever exported — see
    ///   [`Self::mark_article_exported`] — or unread while the policy
    ///   keeps unread). A reaped article leaves a tombstone, so its feed
    ///   listing it again does not resurrect it (see
    ///   [`Self::upsert_articles`]);
    /// - with `plan.purge_orphans`, deletes the unprotected articles whose
    ///   feed was removed (feedless saves are never orphans);
    /// - forgets tombstones whose feed had a successful fetch more than a
    ///   day after the item was last listed — it has left the feed;
    /// - drops tags no article carries.
    ///
    /// Then, outside the transaction and best effort: a bounded
    /// incremental FTS5 merge when articles were deleted, `PRAGMA
    /// optimize`, and a `wal_checkpoint(TRUNCATE)` (readers in flight can
    /// leave the log in place). One of them failing — another process
    /// holding the write lock past the busy timeout — does not fail the
    /// pass: it is logged and reported in
    /// [`MaintenanceCounts::upkeep_skipped`].
    ///
    /// Deleting articles emits no event — `curio.events.v1` has no
    /// deletion event, exactly as for [`Self::delete_article`].
    ///
    /// # Errors
    ///
    /// Database errors in the transaction, which then rolls back as a
    /// whole.
    pub fn run_maintenance(
        &self,
        plan: MaintenancePlan,
    ) -> Result<MaintenanceCounts, StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            record_exports(&tx, &plan.exported, plan.now)?;
            let mut counts = MaintenanceCounts {
                fetch_log_trimmed: trim_fetch_logs(&tx)?,
                ..MaintenanceCounts::default()
            };
            if plan.retention.is_enabled() {
                counts.articles_reaped = reap_by_policy(&tx, &plan)?;
            }
            if plan.purge_orphans {
                counts.orphans_purged = purge_orphans(&tx)?;
            }
            counts.tombstones_pruned = count(tx.execute(
                "DELETE FROM article_tombstones \
                 WHERE strftime('%Y-%m-%dT%H:%M:%fZ', seen_at, '+2 days') < \
                       (SELECT MAX(f.fetched_at) FROM fetch_log f \
                         WHERE f.feed_id = article_tombstones.feed_id AND f.status = 'ok')",
                [],
            )?);
            counts.tags_dropped = drop_unused_tags(&tx)?;
            tx.commit()?;

            finish_upkeep(conn, &mut counts);
            Ok(counts)
        })
    }

    /// Records that the article was exported, so retention never reaps
    /// it (see [`Self::run_maintenance`]) — whatever later happens to the
    /// destination. An export calls this before it writes its note: a
    /// maintenance pass running at the same moment, in this process or
    /// another, then either sees the record or has already deleted the
    /// article, and the export fails instead of writing a note for a
    /// deleted article. Only an unrecorded article takes the write lock;
    /// a re-export is one read.
    ///
    /// Returns `false` when the article does not exist (any more).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn mark_article_exported(
        &self,
        id: ArticleId,
        now: Timestamp,
    ) -> Result<bool, StorageError> {
        Ok(self.mark_articles_exported(&[id], now)?.is_empty())
    }

    /// [`Self::mark_article_exported`] for a bulk export's batch: records
    /// every article of `ids` in one transaction (none at all when each is
    /// recorded already — then it is one read), and returns the ones that
    /// do not exist (any more), for which the export must write no note.
    ///
    /// # Errors
    ///
    /// Database errors; nothing is recorded on error.
    pub fn mark_articles_exported(
        &self,
        ids: &[ArticleId],
        now: Timestamp,
    ) -> Result<BTreeSet<ArticleId>, StorageError> {
        let ids = ids.to_vec();
        let unrecorded = self.read(move |conn| {
            let mut recorded =
                conn.prepare_cached("SELECT 1 FROM article_exports WHERE article_id = ?1")?;
            let mut unrecorded = Vec::new();
            for id in ids {
                if recorded.query_row([id.0], |_| Ok(())).optional()?.is_none() {
                    unrecorded.push(id);
                }
            }
            Ok(unrecorded)
        })?;
        if unrecorded.is_empty() {
            return Ok(BTreeSet::new());
        }
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let mut gone = BTreeSet::new();
            {
                let mut insert = tx.prepare_cached(
                    "INSERT INTO article_exports (article_id, recorded_at) \
                     SELECT id, ?2 FROM articles WHERE id = ?1 \
                     ON CONFLICT (article_id) DO NOTHING",
                )?;
                let mut exists = tx.prepare_cached("SELECT 1 FROM articles WHERE id = ?1")?;
                let now = now.to_string();
                for id in unrecorded {
                    // No row inserted: the article is gone, or another
                    // export recorded it a moment ago.
                    let inserted = insert.execute((id.0, &now))? > 0;
                    if !inserted && exists.query_row([id.0], |_| Ok(())).optional()?.is_none() {
                        gone.insert(id);
                    }
                }
            }
            tx.commit()?;
            Ok(gone)
        })
    }

    /// Rebuilds the FTS5 search index from the `articles` table — the
    /// repair for an index [`Self::fts_integrity_check`] found out of step.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn rebuild_fts(&self) -> Result<(), StorageError> {
        self.write(|conn| {
            let tx = begin_write(conn)?;
            tx.execute(
                "INSERT INTO articles_fts(articles_fts) VALUES ('rebuild')",
                [],
            )?;
            tx.commit()?;
            Ok(())
        })
    }
}

/// Whether a feed article's stored (scoped) key was reaped by retention.
/// A hit re-stamps the tombstone's `seen_at` — at most once a day, so a
/// feed listing reaped items does not rewrite them on every refresh.
pub(super) fn is_tombstoned(
    conn: &Connection,
    dedupe_key: &str,
    now: Timestamp,
) -> Result<bool, StorageError> {
    let seen_at: Option<String> = conn
        .prepare_cached("SELECT seen_at FROM article_tombstones WHERE dedupe_key = ?1")?
        .query_row([dedupe_key], |row| row.get(0))
        .optional()?;
    let Some(seen_at) = seen_at else {
        return Ok(false);
    };
    let restamp_before = Timestamp::from_unix_millis(now.unix_millis() - DAY_MS).unwrap_or(now);
    // One canonical format, so text order is time order.
    if seen_at < restamp_before.to_string() {
        conn.prepare_cached("UPDATE article_tombstones SET seen_at = ?2 WHERE dedupe_key = ?1")?
            .execute((dedupe_key, now.to_string()))?;
    }
    Ok(true)
}

/// Runs [`upkeep`] after a pass's committed transaction; a failure is
/// logged and reported in `counts`, never returned — the reap stands.
fn finish_upkeep(conn: &Connection, counts: &mut MaintenanceCounts) {
    let deleted = counts.articles_reaped + counts.orphans_purged > 0;
    if let Err(err) = upkeep(conn, deleted) {
        tracing::warn!(%err, "maintenance upkeep stopped short; the reap stands");
        counts.upkeep_skipped = Some(err.to_string());
    }
}

/// The best-effort upkeep after a pass's transaction: a bounded
/// incremental search-index merge when articles were deleted (see
/// [`FTS_MERGE_PAGES`]), `PRAGMA optimize`, and a WAL checkpoint.
fn upkeep(conn: &Connection, deleted: bool) -> Result<(), StorageError> {
    if deleted {
        conn.execute(
            "INSERT INTO articles_fts(articles_fts, rank) VALUES ('merge', ?1)",
            [-FTS_MERGE_PAGES],
        )?;
    }
    conn.execute_batch("PRAGMA optimize")?;
    // Returns (busy, log pages, checkpointed pages); a busy result only
    // means a reader kept the log — not a failure.
    conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))?;
    Ok(())
}

/// Records every stored article among `exported` as an export: the
/// manifests' view, which also covers the exports made before Curio
/// recorded them itself.
fn record_exports(
    conn: &Connection,
    exported: &BTreeSet<CurioId>,
    now: Timestamp,
) -> Result<(), StorageError> {
    let now = now.to_string();
    let mut insert = conn.prepare_cached(
        "INSERT INTO article_exports (article_id, recorded_at) \
         SELECT id, ?2 FROM articles WHERE curio_id = ?1 \
         ON CONFLICT (article_id) DO NOTHING",
    )?;
    for curio_id in exported {
        insert.execute((curio_id.to_string(), &now))?;
    }
    Ok(())
}

/// Removes tags no article carries any more; returns how many.
pub(super) fn drop_unused_tags(conn: &Connection) -> Result<u64, StorageError> {
    Ok(count(conn.execute(
        "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM article_tags)",
        [],
    )?))
}

/// Trims every feed's log to its newest [`FETCH_LOG_KEEP`] rows.
fn trim_fetch_logs(conn: &Connection) -> Result<u64, StorageError> {
    Ok(count(conn.execute(
        "DELETE FROM fetch_log WHERE id < (\
             SELECT newer.id FROM fetch_log AS newer \
              WHERE newer.feed_id = fetch_log.feed_id \
              ORDER BY newer.id DESC LIMIT 1 OFFSET ?1)",
        [FETCH_LOG_KEEP - 1],
    )?))
}

/// The protection every reap honours, over `a` (articles) and `s`
/// (its `article_state`, LEFT JOINed): not starred, not in read-later,
/// carrying no tag, never exported.
const UNPROTECTED: &str = "COALESCE(s.is_starred, 0) = 0 AND COALESCE(s.is_read_later, 0) = 0 \
     AND NOT EXISTS (SELECT 1 FROM article_tags t WHERE t.article_id = a.id) \
     AND NOT EXISTS (SELECT 1 FROM article_exports e WHERE e.article_id = a.id)";

/// Applies the retention policy; returns how many articles it reaped.
fn reap_by_policy(conn: &Connection, plan: &MaintenancePlan) -> Result<u64, StorageError> {
    let policy = plan.retention;
    let age_cutoff = policy.max_age_days.and_then(|days| {
        let millis = i64::from(days.get()).saturating_mul(DAY_MS);
        Timestamp::from_unix_millis(plan.now.unix_millis().saturating_sub(millis))
            .map(|cutoff| cutoff.to_string())
    });
    let max_per_feed = policy.max_per_feed.map(std::num::NonZeroU32::get);
    let sql = format!(
        "SELECT a.id, a.dedupe_key, a.feed_id, a.source_url FROM (\
             SELECT id, dedupe_key, feed_id, source_url, saved_at, \
                    ROW_NUMBER() OVER (PARTITION BY feed_id ORDER BY id DESC) AS newest \
               FROM articles WHERE feed_id IS NOT NULL) AS a \
         LEFT JOIN article_state s ON s.article_id = a.id \
         WHERE ((?1 IS NOT NULL AND a.saved_at < ?1) OR (?2 IS NOT NULL AND a.newest > ?2)) \
           AND {UNPROTECTED} \
           AND (?3 = 0 OR COALESCE(s.is_read, 0) <> 0 OR COALESCE(s.is_archived, 0) <> 0)"
    );
    let candidates = conn
        .prepare(&sql)?
        .query_map((age_cutoff, max_per_feed, policy.keep_unread), |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let now = plan.now.to_string();
    let mut reaped = 0;
    for (id, dedupe_key, feed_id, source_url) in candidates {
        conn.prepare_cached("DELETE FROM articles WHERE id = ?1")?
            .execute([id])?;
        conn.prepare_cached(
            "INSERT INTO article_tombstones (dedupe_key, feed_id, source_url, seen_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT (dedupe_key) DO UPDATE \
             SET source_url = excluded.source_url, seen_at = excluded.seen_at",
        )?
        .execute((dedupe_key, feed_id, source_url, &now))?;
        reaped += 1;
    }
    Ok(reaped)
}

/// Deletes the unprotected articles a feed removal orphaned (feed-scoped
/// key, no feed); returns how many.
fn purge_orphans(conn: &Connection) -> Result<u64, StorageError> {
    let sql = format!(
        "SELECT a.id FROM articles a \
         LEFT JOIN article_state s ON s.article_id = a.id \
         WHERE a.feed_id IS NULL AND substr(a.dedupe_key, 1, 2) <> 'm:' AND {UNPROTECTED}"
    );
    let candidates = conn
        .prepare(&sql)?
        .query_map([], |row| row.get::<_, i64>(0))?
        .collect::<Result<Vec<_>, _>>()?;
    let mut purged = 0;
    for id in candidates {
        conn.prepare_cached("DELETE FROM articles WHERE id = ?1")?
            .execute([id])?;
        purged += 1;
    }
    Ok(purged)
}

fn count(rows: usize) -> u64 {
    u64::try_from(rows).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::Duration;

    use super::*;
    use crate::storage::BUSY_TIMEOUT;

    /// Another process holding the write lock past the busy timeout makes
    /// the post-commit upkeep stop short: reported, not an error, and
    /// the counts of what was deleted survive.
    #[test]
    fn a_busy_upkeep_is_reported_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        let storage = Storage::open(dir.path().join("curio.db")).unwrap();
        let other = Connection::open(storage.path()).unwrap();
        other.execute_batch("BEGIN IMMEDIATE").unwrap();

        let counts = storage
            .write(|conn| {
                conn.busy_timeout(Duration::from_millis(50))?;
                let mut counts = MaintenanceCounts {
                    articles_reaped: 3,
                    ..MaintenanceCounts::default()
                };
                finish_upkeep(conn, &mut counts);
                conn.busy_timeout(BUSY_TIMEOUT)?;
                Ok(counts)
            })
            .unwrap();
        assert_eq!(counts.articles_reaped, 3);
        let why = counts.upkeep_skipped.unwrap();
        assert!(why.contains("locked") || why.contains("busy"), "{why}");

        other.execute_batch("COMMIT").unwrap();
        let counts = storage
            .write(|conn| {
                let mut counts = MaintenanceCounts {
                    articles_reaped: 1,
                    ..MaintenanceCounts::default()
                };
                finish_upkeep(conn, &mut counts);
                Ok(counts)
            })
            .unwrap();
        assert_eq!(counts.upkeep_skipped, None);
    }
}
