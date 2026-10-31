//! Per-article state flags (read, starred, read-later, archived) and the
//! snapshot events (`opened`, `saved`, `updated`) they narrate.

use super::begin_write;
use super::rows::{article_tags_tx, insert_intent, require_article, set_flag};
use crate::model::{ArticleId, ArticleState, FeedId};
use crate::storage::{Storage, StorageError};
use curio_types::{ArticleSnapshot, CurioId, EventEnvelope, EventPayload, Timestamp};
use rusqlite::OptionalExtension as _;

impl Storage {
    /// The article's current flag projection (all-false if never touched).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn article_state(&self, id: ArticleId) -> Result<ArticleState, StorageError> {
        self.read(move |conn| {
            let state = conn
                .prepare_cached(
                    "SELECT is_read, is_starred, is_read_later, is_archived \
                     FROM article_state WHERE article_id = ?1",
                )?
                .query_row([id.0], |row| {
                    Ok(ArticleState {
                        read: row.get(0)?,
                        starred: row.get(1)?,
                        read_later: row.get(2)?,
                        archived: row.get(3)?,
                    })
                })
                .optional()?;
            Ok(state.unwrap_or_default())
        })
    }

    /// Flips read/unread. DB-local: `curio.events.v1` defines no read
    /// event, so nothing is staged.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn mark_read(&self, id: ArticleId, read: bool) -> Result<bool, StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            require_article(&tx, id.0)?;
            let changed = set_flag(&tx, id.0, "is_read", read)?;
            tx.commit()?;
            Ok(changed)
        })
    }

    /// Marks every currently-unread article read in one statement — a whole
    /// feed (`Some`) or the entire library (`None`, feedless rows included).
    /// A set-based upsert: articles with no `article_state` row get one,
    /// articles with `is_read = 0` are flipped, already-read ones are skipped
    /// (so the returned count is the number actually changed). DB-local, like
    /// [`Self::mark_read`] — no event.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn mark_all_read(&self, feed_id: Option<FeedId>) -> Result<u64, StorageError> {
        const ON_CONFLICT: &str = "ON CONFLICT(article_id) DO UPDATE SET \
             is_read = 1, updated_at = excluded.updated_at";
        const UNREAD: &str = "NOT EXISTS (SELECT 1 FROM article_state s \
             WHERE s.article_id = a.id AND s.is_read = 1)";
        self.write(move |conn| {
            let now = Timestamp::now().to_string();
            let n = match feed_id {
                Some(fid) => conn
                    .prepare_cached(&format!(
                        "INSERT INTO article_state (article_id, is_read, updated_at) \
                         SELECT a.id, 1, ?2 FROM articles a \
                         WHERE a.feed_id = ?1 AND {UNREAD} {ON_CONFLICT}"
                    ))?
                    .execute((fid.0, &now))?,
                None => conn
                    .prepare_cached(&format!(
                        "INSERT INTO article_state (article_id, is_read, updated_at) \
                         SELECT a.id, 1, ?1 FROM articles a WHERE {UNREAD} {ON_CONFLICT}"
                    ))?
                    .execute([&now])?,
            };
            Ok(u64::try_from(n).unwrap_or_default())
        })
    }

    /// Adds the article to the starred set; stages `article.starred`
    /// (with tags — the tags-in-payload rule). Idempotent: `None` when
    /// already starred, and no event is staged.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn star_article(&self, id: ArticleId) -> Result<Option<EventEnvelope>, StorageError> {
        self.flip_state(id, "is_starred", true, |curio_id, tags| {
            EventPayload::ArticleStarred { curio_id, tags }
        })
    }

    /// Removes the article from the starred set; stages the
    /// `article.unstarred` negation. `None` when not starred.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn unstar_article(&self, id: ArticleId) -> Result<Option<EventEnvelope>, StorageError> {
        self.flip_state(id, "is_starred", false, |curio_id, _| {
            EventPayload::ArticleUnstarred { curio_id }
        })
    }

    /// Adds the article to the read-later set; stages
    /// `article.read_later.added` (with tags). `None` when already queued.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn add_read_later(&self, id: ArticleId) -> Result<Option<EventEnvelope>, StorageError> {
        self.flip_state(id, "is_read_later", true, |curio_id, tags| {
            EventPayload::ReadLaterAdded { curio_id, tags }
        })
    }

    /// Removes the article from the read-later set; stages the
    /// `article.read_later.removed` negation. `None` when not queued.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn remove_read_later(&self, id: ArticleId) -> Result<Option<EventEnvelope>, StorageError> {
        self.flip_state(id, "is_read_later", false, |curio_id, _| {
            EventPayload::ReadLaterRemoved { curio_id }
        })
    }

    /// Archives the article; stages `article.archived`. `None` when
    /// already archived.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn archive_article(&self, id: ArticleId) -> Result<Option<EventEnvelope>, StorageError> {
        self.flip_state(id, "is_archived", true, |curio_id, _| {
            EventPayload::ArticleArchived { curio_id }
        })
    }

    /// Unarchives the article; stages the `article.unarchived` negation.
    /// `None` when not archived.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn unarchive_article(&self, id: ArticleId) -> Result<Option<EventEnvelope>, StorageError> {
        self.flip_state(id, "is_archived", false, |curio_id, _| {
            EventPayload::ArticleUnarchived { curio_id }
        })
    }

    /// Records an open-for-reading; always stages `article.opened`.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn record_opened(
        &self,
        id: ArticleId,
        dwell_ms: Option<u64>,
    ) -> Result<EventEnvelope, StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let curio_id = require_article(&tx, id.0)?;
            let envelope = EventEnvelope::new(EventPayload::ArticleOpened { curio_id, dwell_ms });
            insert_intent(&tx, &envelope)?;
            tx.commit()?;
            Ok(envelope)
        })
    }

    /// Stages `article.saved` for a completed first export. The snapshot
    /// (destination, path, checksum, tags) is the export layer's to build
    /// — see [`Article::snapshot`](crate::model::Article::snapshot).
    ///
    /// # Errors
    ///
    /// Database or serialization errors.
    pub fn record_article_saved(
        &self,
        snapshot: ArticleSnapshot,
    ) -> Result<EventEnvelope, StorageError> {
        self.stage_snapshot_event(EventPayload::ArticleSaved(snapshot))
    }

    /// Stages `article.updated` for a re-export with a changed checksum.
    ///
    /// # Errors
    ///
    /// Database or serialization errors.
    pub fn record_article_updated(
        &self,
        snapshot: ArticleSnapshot,
    ) -> Result<EventEnvelope, StorageError> {
        self.stage_snapshot_event(EventPayload::ArticleUpdated(snapshot))
    }

    fn stage_snapshot_event(&self, payload: EventPayload) -> Result<EventEnvelope, StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let envelope = EventEnvelope::new(payload);
            insert_intent(&tx, &envelope)?;
            tx.commit()?;
            Ok(envelope)
        })
    }

    /// Shared flag-flip: state change + staged event in one transaction,
    /// idempotent (no change → no event → `None`).
    fn flip_state(
        &self,
        id: ArticleId,
        column: &'static str,
        value: bool,
        payload: impl FnOnce(CurioId, Vec<String>) -> EventPayload + Send + 'static,
    ) -> Result<Option<EventEnvelope>, StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let curio_id = require_article(&tx, id.0)?;
            let changed = set_flag(&tx, id.0, column, value)?;
            let envelope = if changed {
                let tags = article_tags_tx(&tx, id.0)?;
                let envelope = EventEnvelope::new(payload(curio_id, tags));
                insert_intent(&tx, &envelope)?;
                Some(envelope)
            } else {
                None
            };
            tx.commit()?;
            Ok(envelope)
        })
    }
}
