//! Typed repository functions — the only place SQL exists.
//!
//! Every function either runs on the writer thread (mutations, always
//! transactional) or on a pooled read-only connection (queries). State
//! changes that the `curio.events.v1` contract cares about mint their
//! event envelope and stage it as a write-ahead intent row **in the same
//! transaction** as the state change; the events layer appends staged
//! intents to the JSONL log and deletes them (crash-recoverable — see
//! `events::EventEmitter`).

use std::collections::BTreeMap;

use curio_types::{ArticleSnapshot, CurioId, EventEnvelope, EventPayload, Timestamp};
use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension as _, Row};

use super::{Storage, StorageError};
use crate::model::{
    Article, ArticleContent, ArticleId, ArticleState, Feed, FeedId, FeedStatus, FetchRecord,
    FetchStatus, NewArticle, NewFeed,
};

/// What a batch upsert did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpsertOutcome {
    /// Articles that did not exist and were inserted.
    pub inserted: u64,
    /// Articles that existed (by dedupe key) and were refreshed.
    pub updated: u64,
}

/// Keyset-paginated article listing parameters.
///
/// Filters are backend-owned so heads never re-filter client-side: every
/// `Some` filter is `AND`ed into the query. An article with no
/// `article_state` row counts as unread/unstarred/not-read-later/
/// unarchived. The keyset order is fixed — `id DESC` (newest row first),
/// paginated by `before` — and filters never change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListArticles {
    /// Restrict to one feed; `None` lists across feeds.
    pub feed_id: Option<FeedId>,
    /// Keyset cursor: return articles with `id` strictly below this.
    pub before: Option<ArticleId>,
    /// Page size.
    pub limit: u32,
    /// Keep only (un)read articles.
    pub read: Option<bool>,
    /// Keep only (un)starred articles.
    pub starred: Option<bool>,
    /// Keep only articles (not) in the read-later queue.
    pub read_later: Option<bool>,
    /// Keep only (un)archived articles.
    pub archived: Option<bool>,
    /// Keep only articles carrying this tag (trimmed before matching,
    /// mirroring how tags are stored).
    pub tag: Option<String>,
}

impl Default for ListArticles {
    fn default() -> Self {
        Self {
            feed_id: None,
            before: None,
            limit: 50,
            read: None,
            starred: None,
            read_later: None,
            archived: None,
            tag: None,
        }
    }
}

/// A staged, not-yet-emitted event intent (write-ahead row for the JSONL
/// append).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingIntent {
    /// Row id — pass back to [`Storage::mark_intents_emitted`].
    pub intent_id: i64,
    /// The full envelope to append, exactly as staged.
    pub envelope: EventEnvelope,
}

const FEED_COLS: &str = "id, url, title, site_url, description, etag, last_modified, status, \
                         allow_private_network, added_at, last_fetched_at, tags";

const ARTICLE_COLS: &str = "id, curio_id, feed_id, dedupe_key, title, source_url, author, \
                            published_at, content_html, content_text, lang, word_count, \
                            saved_at, source_updated_at";

impl Storage {
    // ------------------------------------------------------------ feeds

    /// Subscribes to a feed and stages the `feed.added` event. Tags are
    /// normalized first (trimmed, empties dropped, deduplicated in
    /// first-seen order) so the staged event satisfies the published
    /// schema's `minLength: 1` / `uniqueItems` constraints no matter how
    /// sloppy the input (CLI flags, OPML categories).
    ///
    /// # Errors
    ///
    /// Fails with [`StorageError::Sqlite`] if the URL is already
    /// subscribed (UNIQUE), or on any database error.
    pub fn add_feed(&self, new: NewFeed) -> Result<(Feed, EventEnvelope), StorageError> {
        self.write(move |conn| {
            let tx = conn.transaction()?;
            let now = Timestamp::now();
            let tags = normalize_tags(new.tags);
            tx.prepare_cached(
                "INSERT INTO feeds (url, title, tags, added_at, modified_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )?
            .execute((
                &new.url,
                &new.title,
                serde_json::to_string(&tags)?,
                now.to_string(),
                now.to_string(),
            ))?;
            let id = tx.last_insert_rowid();
            let envelope = EventEnvelope::new(EventPayload::FeedAdded {
                feed: new.url.clone(),
                feed_title: new.title.clone(),
                tags,
            });
            insert_intent(&tx, &envelope)?;
            let feed = feed_by_id(&tx, id)?.ok_or(StorageError::NotFound { entity: "feed" })?;
            tx.commit()?;
            Ok((feed, envelope))
        })
    }

    /// Unsubscribes a feed and stages the `feed.removed` negation event.
    /// Articles keep their rows (`feed_id` becomes NULL).
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn remove_feed(&self, id: FeedId) -> Result<EventEnvelope, StorageError> {
        self.write(move |conn| {
            let tx = conn.transaction()?;
            let url: String = tx
                .prepare_cached("SELECT url FROM feeds WHERE id = ?1")?
                .query_row([id.0], |row| row.get(0))
                .optional()?
                .ok_or(StorageError::NotFound { entity: "feed" })?;
            tx.prepare_cached("DELETE FROM feeds WHERE id = ?1")?
                .execute([id.0])?;
            let envelope = EventEnvelope::new(EventPayload::FeedRemoved { feed: url });
            insert_intent(&tx, &envelope)?;
            tx.commit()?;
            Ok(envelope)
        })
    }

    /// Fetches a feed by row id.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn get_feed(&self, id: FeedId) -> Result<Option<Feed>, StorageError> {
        self.read(|conn| feed_by_id(conn, id.0))
    }

    /// Fetches a feed by its URL (the subscription key).
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn get_feed_by_url(&self, url: &str) -> Result<Option<Feed>, StorageError> {
        self.read(|conn| {
            conn.prepare_cached(&format!("SELECT {FEED_COLS} FROM feeds WHERE url = ?1"))?
                .query_row([url], raw_feed)
                .optional()?
                .map(RawFeed::into_feed)
                .transpose()
        })
    }

    /// Lists every feed, ordered by row id.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn list_feeds(&self) -> Result<Vec<Feed>, StorageError> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare_cached(&format!("SELECT {FEED_COLS} FROM feeds ORDER BY id"))?;
            let raws = stmt
                .query_map([], raw_feed)?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter().map(RawFeed::into_feed).collect()
        })
    }

    /// Updates a feed's conditional-GET state after a fetch. Preserved on
    /// error paths by the caller passing the previous values back.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn update_feed_fetch_state(
        &self,
        id: FeedId,
        etag: Option<String>,
        last_modified: Option<String>,
        fetched_at: Timestamp,
    ) -> Result<(), StorageError> {
        self.write(move |conn| {
            let n = conn
                .prepare_cached(
                    "UPDATE feeds SET etag = ?2, last_modified = ?3, last_fetched_at = ?4, \
                     modified_at = ?5 WHERE id = ?1",
                )?
                .execute((
                    id.0,
                    etag,
                    last_modified,
                    fetched_at.to_string(),
                    Timestamp::now().to_string(),
                ))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    /// Flips a feed's contract-W1 private-network exemption. Reachable
    /// only from explicit configuration surfaces — never feed content.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn set_feed_allow_private_network(
        &self,
        id: FeedId,
        allow: bool,
    ) -> Result<(), StorageError> {
        self.write(move |conn| {
            let n = conn
                .prepare_cached(
                    "UPDATE feeds SET allow_private_network = ?2, modified_at = ?3 WHERE id = ?1",
                )?
                .execute((id.0, allow, Timestamp::now().to_string()))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    /// Fills in feed metadata learned from a fetch, without clobbering
    /// anything already set (human edits win over parsed values).
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn update_feed_metadata(
        &self,
        id: FeedId,
        title: Option<String>,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), StorageError> {
        self.write(move |conn| {
            let n = conn
                .prepare_cached(
                    "UPDATE feeds SET title = COALESCE(title, ?2), \
                     site_url = COALESCE(site_url, ?3), \
                     description = COALESCE(description, ?4), \
                     modified_at = ?5 WHERE id = ?1",
                )?
                .execute((
                    id.0,
                    title,
                    site_url,
                    description,
                    Timestamp::now().to_string(),
                ))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    /// Adopts a permanent-redirect target as the feed's stored URL.
    /// A conflict with another subscription's URL leaves the row
    /// unchanged (`UPDATE OR IGNORE`) — better a stale URL than a
    /// UNIQUE explosion mid-refresh.
    ///
    /// The feed URL is the identity key of `feed.added`/`feed.removed`,
    /// so an actual rewrite stages the negation pair in the same
    /// transaction — `feed.removed{old}` then `feed.added{new}` (with
    /// the stored title/tags). Without it, a later removal would carry
    /// the new URL and never negate the `feed.added` a consumer folded
    /// under the original URL, leaving a phantom live subscription.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn update_feed_url(&self, id: FeedId, url: String) -> Result<(), StorageError> {
        self.write(move |conn| {
            let tx = conn.transaction()?;
            let row: Option<(String, Option<String>, String)> = tx
                .prepare_cached("SELECT url, title, tags FROM feeds WHERE id = ?1")?
                .query_row([id.0], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .optional()?;
            let Some((old_url, title, tags_json)) = row else {
                return Ok(()); // vanished mid-refresh: nothing to adopt
            };
            if old_url != url {
                let n = tx
                    .prepare_cached(
                        "UPDATE OR IGNORE feeds SET url = ?2, modified_at = ?3 WHERE id = ?1",
                    )?
                    .execute((id.0, &url, Timestamp::now().to_string()))?;
                // Events only when the rewrite actually happened (an
                // OR-IGNORE conflict keeps the old URL — and the old
                // membership stays correct).
                if n > 0 {
                    let tags: Vec<String> =
                        serde_json::from_str(&tags_json).map_err(|err| StorageError::Corrupt {
                            column: "feeds.tags",
                            message: err.to_string(),
                        })?;
                    insert_intent(
                        &tx,
                        &EventEnvelope::new(EventPayload::FeedRemoved { feed: old_url }),
                    )?;
                    insert_intent(
                        &tx,
                        &EventEnvelope::new(EventPayload::FeedAdded {
                            feed: url,
                            feed_title: title,
                            tags,
                        }),
                    )?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Sets a feed's lifecycle status (pause, dead-on-410, reactivate).
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn set_feed_status(&self, id: FeedId, status: FeedStatus) -> Result<(), StorageError> {
        self.write(move |conn| {
            let n = conn
                .prepare_cached("UPDATE feeds SET status = ?2, modified_at = ?3 WHERE id = ?1")?
                .execute((id.0, status.as_str(), Timestamp::now().to_string()))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    // --------------------------------------------------------- articles

    /// Batch-upserts articles in a single transaction (one fsync per
    /// batch, not per row). Identity: existing rows are matched by the
    /// **feed-scoped** dedupe key (`f<feed_id>:`/`m:` prefix) — colliding
    /// guids from different feeds are different articles, never
    /// overwrites. An update refreshes content columns but never touches
    /// `curio_id` or `saved_at`.
    ///
    /// # Errors
    ///
    /// Database errors; the whole batch rolls back on failure.
    pub fn upsert_articles(
        &self,
        articles: Vec<NewArticle>,
    ) -> Result<UpsertOutcome, StorageError> {
        self.write(move |conn| {
            let tx = conn.transaction()?;
            let mut outcome = UpsertOutcome::default();
            let now = Timestamp::now().to_string();
            for article in articles {
                let dedupe_key = scoped_dedupe_key(article.feed_id, &article.dedupe_key);
                let existing: Option<i64> = tx
                    .prepare_cached("SELECT id FROM articles WHERE dedupe_key = ?1")?
                    .query_row([&dedupe_key], |row| row.get(0))
                    .optional()?;
                if let Some(id) = existing {
                    tx.prepare_cached(
                        "UPDATE articles SET title = ?2, source_url = ?3, author = ?4, \
                         published_at = ?5, content_html = ?6, content_text = ?7, lang = ?8, \
                         word_count = ?9, source_updated_at = ?10, modified_at = ?11 \
                         WHERE id = ?1",
                    )?
                    .execute((
                        id,
                        &article.title,
                        &article.source_url,
                        &article.author,
                        article.published_at.map(|t| t.to_string()),
                        &article.content.html,
                        &article.content.text,
                        &article.lang,
                        article.word_count,
                        article.source_updated_at.map(|t| t.to_string()),
                        &now,
                    ))?;
                    outcome.updated += 1;
                } else {
                    tx.prepare_cached(
                        "INSERT INTO articles (curio_id, feed_id, dedupe_key, title, \
                         source_url, author, published_at, content_html, content_text, lang, \
                         word_count, saved_at, source_updated_at, modified_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
                    )?
                    .execute((
                        CurioId::new().to_string(),
                        article.feed_id.map(|f| f.0),
                        &dedupe_key,
                        &article.title,
                        &article.source_url,
                        &article.author,
                        article.published_at.map(|t| t.to_string()),
                        &article.content.html,
                        &article.content.text,
                        &article.lang,
                        article.word_count,
                        &now,
                        article.source_updated_at.map(|t| t.to_string()),
                        &now,
                    ))?;
                    outcome.inserted += 1;
                }
            }
            tx.commit()?;
            Ok(outcome)
        })
    }

    /// Fetches an article by row id.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn get_article(&self, id: ArticleId) -> Result<Option<Article>, StorageError> {
        self.read(|conn| article_by_id(conn, id.0))
    }

    /// Fetches an article by its `curio_id` (contract identity).
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn get_article_by_curio_id(&self, id: CurioId) -> Result<Option<Article>, StorageError> {
        self.read(move |conn| {
            conn.prepare_cached(&format!(
                "SELECT {ARTICLE_COLS} FROM articles WHERE curio_id = ?1"
            ))?
            .query_row([id.to_string()], raw_article)
            .optional()?
            .map(RawArticle::into_article)
            .transpose()
        })
    }

    /// Keyset-paginated listing, newest row id first. See [`ListArticles`]
    /// for the filter semantics; filters never change the order.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn list_articles(&self, params: ListArticles) -> Result<Vec<Article>, StorageError> {
        self.read(move |conn| {
            let (sql, binds) = list_articles_query(&params);
            let mut stmt = conn.prepare_cached(&sql)?;
            let raws = stmt
                .query_map(
                    rusqlite::params_from_iter(binds.iter().map(Box::as_ref)),
                    raw_article,
                )?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter().map(RawArticle::into_article).collect()
        })
    }

    /// Unread-article counts, grouped by feed (`GROUP BY feed_id` in
    /// SQL — counts are backend-owned, never client badge math). The
    /// `None` key counts articles that belong to no feed (manual saves,
    /// rows orphaned by a feed removal); the total unread count is the
    /// sum of the map's values. An article with no `article_state` row
    /// has never been touched and counts as unread; the other flags
    /// (starred, read-later, archived) do not affect unreadness.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn unread_counts(&self) -> Result<BTreeMap<Option<FeedId>, u64>, StorageError> {
        self.read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT a.feed_id, COUNT(*) FROM articles a \
                 LEFT JOIN article_state s ON s.article_id = a.id \
                 WHERE COALESCE(s.is_read, 0) = 0 GROUP BY a.feed_id",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, i64>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            Ok(rows
                .into_iter()
                .map(|(feed_id, n)| (feed_id.map(FeedId), u64::try_from(n).unwrap_or_default()))
                .collect())
        })
    }

    /// Total number of stored articles (computed in SQL, never in a head).
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn count_articles(&self) -> Result<u64, StorageError> {
        self.read(|conn| {
            let n: i64 = conn
                .prepare_cached("SELECT COUNT(*) FROM articles")?
                .query_row([], |row| row.get(0))?;
            Ok(u64::try_from(n).unwrap_or_default())
        })
    }

    /// Full-text search over title + extracted content. User input is
    /// compiled into an escaped FTS5 phrase query — never raw `MATCH`
    /// (unbalanced quotes must not surface as database errors).
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn search_articles(&self, query: &str, limit: u32) -> Result<Vec<Article>, StorageError> {
        let Some(expr) = fts_match_expr(query) else {
            return Ok(Vec::new());
        };
        self.read(move |conn| {
            let cols = article_cols_prefixed("a");
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {cols} FROM articles_fts \
                 JOIN articles a ON a.id = articles_fts.rowid \
                 WHERE articles_fts MATCH ?1 ORDER BY rank LIMIT ?2"
            ))?;
            let raws = stmt
                .query_map((expr, limit), raw_article)?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter().map(RawArticle::into_article).collect()
        })
    }

    /// Deletes an article (state, tags, and the FTS index entry go with
    /// it via cascade + trigger).
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn delete_article(&self, id: ArticleId) -> Result<(), StorageError> {
        self.write(move |conn| {
            let n = conn
                .prepare_cached("DELETE FROM articles WHERE id = ?1")?
                .execute([id.0])?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "article" });
            }
            Ok(())
        })
    }

    // ------------------------------------------------------------ state

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
            let tx = conn.transaction()?;
            require_article(&tx, id.0)?;
            let changed = set_flag(&tx, id.0, "is_read", read)?;
            tx.commit()?;
            Ok(changed)
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
            let tx = conn.transaction()?;
            let curio_id = require_article(&tx, id.0)?;
            let envelope = EventEnvelope::new(EventPayload::ArticleOpened { curio_id, dwell_ms });
            insert_intent(&tx, &envelope)?;
            tx.commit()?;
            Ok(envelope)
        })
    }

    /// Stages `article.saved` for a completed first export. The snapshot
    /// (destination, path, checksum, tags) is the export layer's to build
    /// — see [`Article::snapshot`].
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
            let tx = conn.transaction()?;
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
            let tx = conn.transaction()?;
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

    // ------------------------------------------------------------- tags

    /// Adds a tag to an article; stages `article.tagged`. Idempotent:
    /// `None` when the article already carries the tag. The tag is
    /// trimmed before storage.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist;
    /// [`StorageError::InvalidTag`] for an empty/whitespace-only tag
    /// (the published schemas require `minLength: 1`).
    pub fn tag_article(
        &self,
        id: ArticleId,
        tag: &str,
    ) -> Result<Option<EventEnvelope>, StorageError> {
        let tag = validated_tag(tag)?;
        self.write(move |conn| {
            let tx = conn.transaction()?;
            let curio_id = require_article(&tx, id.0)?;
            tx.prepare_cached("INSERT OR IGNORE INTO tags (name) VALUES (?1)")?
                .execute([&tag])?;
            let tag_id: i64 = tx
                .prepare_cached("SELECT id FROM tags WHERE name = ?1")?
                .query_row([&tag], |row| row.get(0))?;
            let n = tx
                .prepare_cached(
                    "INSERT OR IGNORE INTO article_tags (article_id, tag_id) VALUES (?1, ?2)",
                )?
                .execute((id.0, tag_id))?;
            let envelope = if n > 0 {
                let envelope = EventEnvelope::new(EventPayload::ArticleTagged { curio_id, tag });
                insert_intent(&tx, &envelope)?;
                Some(envelope)
            } else {
                None
            };
            tx.commit()?;
            Ok(envelope)
        })
    }

    /// Removes a tag from an article; stages the `article.untagged`
    /// negation. `None` when the article did not carry the tag. The tag
    /// is trimmed before matching, mirroring [`Storage::tag_article`].
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist;
    /// [`StorageError::InvalidTag`] for an empty/whitespace-only tag.
    pub fn untag_article(
        &self,
        id: ArticleId,
        tag: &str,
    ) -> Result<Option<EventEnvelope>, StorageError> {
        let tag = validated_tag(tag)?;
        self.write(move |conn| {
            let tx = conn.transaction()?;
            let curio_id = require_article(&tx, id.0)?;
            let n = tx
                .prepare_cached(
                    "DELETE FROM article_tags WHERE article_id = ?1 \
                     AND tag_id = (SELECT id FROM tags WHERE name = ?2)",
                )?
                .execute((id.0, &tag))?;
            let envelope = if n > 0 {
                let envelope = EventEnvelope::new(EventPayload::ArticleUntagged { curio_id, tag });
                insert_intent(&tx, &envelope)?;
                Some(envelope)
            } else {
                None
            };
            tx.commit()?;
            Ok(envelope)
        })
    }

    /// The article's tags, sorted.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn article_tags(&self, id: ArticleId) -> Result<Vec<String>, StorageError> {
        self.read(move |conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT t.name FROM tags t \
                 JOIN article_tags at ON at.tag_id = t.id \
                 WHERE at.article_id = ?1 ORDER BY t.name",
            )?;
            let tags = stmt
                .query_map([id.0], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(tags)
        })
    }

    /// Every known tag name, sorted.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn list_tags(&self) -> Result<Vec<String>, StorageError> {
        self.read(|conn| {
            let mut stmt = conn.prepare_cached("SELECT name FROM tags ORDER BY name")?;
            let tags = stmt
                .query_map([], |row| row.get(0))?
                .collect::<Result<Vec<String>, _>>()?;
            Ok(tags)
        })
    }

    // -------------------------------------------------------- fetch log

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

    // --------------------------------------------------------- settings

    /// Reads a settings value.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>, StorageError> {
        self.read(move |conn| {
            let value = conn
                .prepare_cached("SELECT value FROM settings WHERE key = ?1")?
                .query_row([key], |row| row.get(0))
                .optional()?;
            Ok(value)
        })
    }

    /// Writes (upserts) a settings value.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), StorageError> {
        let (key, value) = (key.to_owned(), value.to_owned());
        self.write(move |conn| {
            conn.prepare_cached(
                "INSERT INTO settings (key, value) VALUES (?1, ?2) \
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            )?
            .execute((&key, &value))?;
            Ok(())
        })
    }

    // ----------------------------------------------------- event intents

    /// Staged-but-unemitted event intents, oldest first (emission order).
    ///
    /// # Errors
    ///
    /// Database or envelope-deserialization errors.
    pub fn pending_intents(&self) -> Result<Vec<PendingIntent>, StorageError> {
        self.read(|conn| {
            let mut stmt =
                conn.prepare_cached("SELECT id, envelope_json FROM event_intents ORDER BY id ASC")?;
            let raws = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?;
            raws.into_iter()
                .map(|(intent_id, json)| {
                    Ok(PendingIntent {
                        intent_id,
                        envelope: serde_json::from_str(&json)?,
                    })
                })
                .collect()
        })
    }

    /// Deletes intents whose envelopes have been durably appended to the
    /// JSONL log.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn mark_intents_emitted(&self, intent_ids: Vec<i64>) -> Result<(), StorageError> {
        self.write(move |conn| {
            let tx = conn.transaction()?;
            for id in intent_ids {
                tx.prepare_cached("DELETE FROM event_intents WHERE id = ?1")?
                    .execute([id])?;
            }
            tx.commit()?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------- helpers

/// Namespaces an ingest-computed dedupe key by its provenance:
/// `f<feed_id>:` for feed articles, `m:` for manual saves — so two feeds
/// carrying the same guid (or the same hash-tier key) can never match
/// each other's rows, while the global UNIQUE constraint stays as
/// defense in depth. Migration 0002 applied the same transform to
/// pre-existing rows.
///
/// Provenance is the feed's row id, not its URL, so permanent-redirect
/// URL adoption never churns keys. Rows orphaned by a feed removal keep
/// their historical prefix (feed row ids are effectively never reused at
/// personal scale, and a re-subscription is a new subscription).
fn scoped_dedupe_key(feed_id: Option<FeedId>, key: &str) -> String {
    match feed_id {
        Some(feed) => format!("f{}:{key}", feed.0),
        None => format!("m:{key}"),
    }
}

/// Trims a tag and refuses the empty result — the producer-side
/// enforcement of the published schemas' `minLength: 1` on tags.
fn validated_tag(tag: &str) -> Result<String, StorageError> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(StorageError::InvalidTag);
    }
    Ok(tag.to_owned())
}

/// Normalizes a tag list: trim, drop empties, dedupe in first-seen order
/// (`uniqueItems` in the published schemas).
fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !out.iter().any(|t| t == tag) {
            out.push(tag.to_owned());
        }
    }
    out
}

/// Compiles user input into a safe FTS5 query: every whitespace-separated
/// token becomes a quoted phrase (embedded quotes doubled), joined by the
/// implicit AND. `None` for effectively-empty input.
fn fts_match_expr(input: &str) -> Option<String> {
    let tokens: Vec<String> = input
        .split_whitespace()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

/// Compiles [`ListArticles`] into SQL + positional binds. Static clause
/// fragments only — no user input ever lands in the SQL string (the tag
/// value is a bind). The `article_state` join appears only when a state
/// filter asks for it, so the unfiltered hot path stays join-free; a
/// missing state row defaults every flag to 0 via `COALESCE`.
fn list_articles_query(params: &ListArticles) -> (String, Vec<Box<dyn ToSql>>) {
    let cols = article_cols_prefixed("a");
    let mut sql = format!("SELECT {cols} FROM articles a");
    let state_filters = [
        ("is_read", params.read),
        ("is_starred", params.starred),
        ("is_read_later", params.read_later),
        ("is_archived", params.archived),
    ];
    if state_filters.iter().any(|(_, want)| want.is_some()) {
        sql.push_str(" LEFT JOIN article_state s ON s.article_id = a.id");
    }
    let mut clauses = vec!["a.id < ?".to_owned()];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(params.before.map_or(i64::MAX, |id| id.0))];
    if let Some(feed) = params.feed_id {
        clauses.push("a.feed_id = ?".to_owned());
        binds.push(Box::new(feed.0));
    }
    for (column, want) in state_filters {
        if let Some(want) = want {
            clauses.push(format!("COALESCE(s.{column}, 0) = ?"));
            binds.push(Box::new(want));
        }
    }
    if let Some(tag) = &params.tag {
        clauses.push(
            "EXISTS (SELECT 1 FROM article_tags at JOIN tags t ON t.id = at.tag_id \
             WHERE at.article_id = a.id AND t.name = ?)"
                .to_owned(),
        );
        binds.push(Box::new(tag.trim().to_owned()));
    }
    sql.push_str(" WHERE ");
    sql.push_str(&clauses.join(" AND "));
    sql.push_str(" ORDER BY a.id DESC LIMIT ?");
    binds.push(Box::new(params.limit));
    (sql, binds)
}

fn article_cols_prefixed(alias: &str) -> String {
    ARTICLE_COLS
        .split(',')
        .map(|col| format!("{alias}.{}", col.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Asserts the article exists and returns its `curio_id`.
fn require_article(conn: &Connection, article_id: i64) -> Result<CurioId, StorageError> {
    let raw: String = conn
        .prepare_cached("SELECT curio_id FROM articles WHERE id = ?1")?
        .query_row([article_id], |row| row.get(0))
        .optional()?
        .ok_or(StorageError::NotFound { entity: "article" })?;
    parse_curio_id(&raw)
}

/// Upserts one boolean flag in `article_state`; `Ok(true)` iff the stored
/// value actually changed. `column` is a compile-time constant — never
/// user input.
fn set_flag(
    conn: &Connection,
    article_id: i64,
    column: &'static str,
    value: bool,
) -> Result<bool, StorageError> {
    let current: Option<bool> = conn
        .prepare_cached(&format!(
            "SELECT {column} FROM article_state WHERE article_id = ?1"
        ))?
        .query_row([article_id], |row| row.get(0))
        .optional()?;
    if current.unwrap_or(false) == value {
        return Ok(false);
    }
    conn.prepare_cached(&format!(
        "INSERT INTO article_state (article_id, {column}, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(article_id) DO UPDATE SET {column} = excluded.{column}, \
         updated_at = excluded.updated_at"
    ))?
    .execute((article_id, value, Timestamp::now().to_string()))?;
    Ok(true)
}

/// The article's tags inside an open transaction (tags-in-payload rule).
fn article_tags_tx(conn: &Connection, article_id: i64) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.name FROM tags t JOIN article_tags at ON at.tag_id = t.id \
         WHERE at.article_id = ?1 ORDER BY t.name",
    )?;
    let tags = stmt
        .query_map([article_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(tags)
}

/// Stages an envelope as a write-ahead intent row (same transaction as
/// the state change it narrates).
fn insert_intent(conn: &Connection, envelope: &EventEnvelope) -> Result<(), StorageError> {
    let json = serde_json::to_string(envelope)?;
    conn.prepare_cached(
        "INSERT INTO event_intents (event_id, ts, envelope_json) VALUES (?1, ?2, ?3)",
    )?
    .execute((envelope.event_id.to_string(), envelope.ts.to_string(), json))?;
    Ok(())
}

fn feed_by_id(conn: &Connection, id: i64) -> Result<Option<Feed>, StorageError> {
    conn.prepare_cached(&format!("SELECT {FEED_COLS} FROM feeds WHERE id = ?1"))?
        .query_row([id], raw_feed)
        .optional()?
        .map(RawFeed::into_feed)
        .transpose()
}

fn article_by_id(conn: &Connection, id: i64) -> Result<Option<Article>, StorageError> {
    conn.prepare_cached(&format!(
        "SELECT {ARTICLE_COLS} FROM articles WHERE id = ?1"
    ))?
    .query_row([id], raw_article)
    .optional()?
    .map(RawArticle::into_article)
    .transpose()
}

fn parse_ts(column: &'static str, s: &str) -> Result<Timestamp, StorageError> {
    s.parse().map_err(|err| StorageError::Corrupt {
        column,
        message: format!("{err}"),
    })
}

fn parse_opt_ts(
    column: &'static str,
    s: Option<String>,
) -> Result<Option<Timestamp>, StorageError> {
    s.map(|s| parse_ts(column, &s)).transpose()
}

fn parse_curio_id(s: &str) -> Result<CurioId, StorageError> {
    s.parse().map_err(|err| StorageError::Corrupt {
        column: "articles.curio_id",
        message: format!("{err}"),
    })
}

/// Raw row image of `feeds` — primitives out of rusqlite, parsed into the
/// domain type in a second step so parse failures surface as
/// [`StorageError::Corrupt`], not panics.
struct RawFeed {
    id: i64,
    url: String,
    title: Option<String>,
    site_url: Option<String>,
    description: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    status: String,
    allow_private_network: bool,
    added_at: String,
    last_fetched_at: Option<String>,
    tags: String,
}

fn raw_feed(row: &Row<'_>) -> rusqlite::Result<RawFeed> {
    Ok(RawFeed {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        site_url: row.get(3)?,
        description: row.get(4)?,
        etag: row.get(5)?,
        last_modified: row.get(6)?,
        status: row.get(7)?,
        allow_private_network: row.get(8)?,
        added_at: row.get(9)?,
        last_fetched_at: row.get(10)?,
        tags: row.get(11)?,
    })
}

impl RawFeed {
    fn into_feed(self) -> Result<Feed, StorageError> {
        Ok(Feed {
            id: FeedId(self.id),
            url: self.url,
            title: self.title,
            site_url: self.site_url,
            description: self.description,
            etag: self.etag,
            last_modified: self.last_modified,
            status: FeedStatus::parse(&self.status).ok_or_else(|| StorageError::Corrupt {
                column: "feeds.status",
                message: format!("unknown status {:?}", self.status),
            })?,
            allow_private_network: self.allow_private_network,
            added_at: parse_ts("feeds.added_at", &self.added_at)?,
            last_fetched_at: parse_opt_ts("feeds.last_fetched_at", self.last_fetched_at)?,
            tags: serde_json::from_str(&self.tags).map_err(|err| StorageError::Corrupt {
                column: "feeds.tags",
                message: err.to_string(),
            })?,
        })
    }
}

/// Raw row image of `articles` (see [`RawFeed`]).
struct RawArticle {
    id: i64,
    curio_id: String,
    feed_id: Option<i64>,
    dedupe_key: String,
    title: String,
    source_url: String,
    author: Option<String>,
    published_at: Option<String>,
    content_html: String,
    content_text: String,
    lang: Option<String>,
    word_count: Option<u32>,
    saved_at: String,
    source_updated_at: Option<String>,
}

fn raw_article(row: &Row<'_>) -> rusqlite::Result<RawArticle> {
    Ok(RawArticle {
        id: row.get(0)?,
        curio_id: row.get(1)?,
        feed_id: row.get(2)?,
        dedupe_key: row.get(3)?,
        title: row.get(4)?,
        source_url: row.get(5)?,
        author: row.get(6)?,
        published_at: row.get(7)?,
        content_html: row.get(8)?,
        content_text: row.get(9)?,
        lang: row.get(10)?,
        word_count: row.get(11)?,
        saved_at: row.get(12)?,
        source_updated_at: row.get(13)?,
    })
}

impl RawArticle {
    fn into_article(self) -> Result<Article, StorageError> {
        Ok(Article {
            id: ArticleId(self.id),
            curio_id: parse_curio_id(&self.curio_id)?,
            feed_id: self.feed_id.map(FeedId),
            dedupe_key: self.dedupe_key,
            title: self.title,
            source_url: self.source_url,
            author: self.author,
            published_at: parse_opt_ts("articles.published_at", self.published_at)?,
            content: ArticleContent {
                html: self.content_html,
                text: self.content_text,
            },
            lang: self.lang,
            word_count: self.word_count,
            saved_at: parse_ts("articles.saved_at", &self.saved_at)?,
            source_updated_at: parse_opt_ts("articles.source_updated_at", self.source_updated_at)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fts_match_expr_quotes_every_token() {
        assert_eq!(
            fts_match_expr("rust sqlite"),
            Some("\"rust\" \"sqlite\"".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_escapes_embedded_quotes() {
        assert_eq!(
            fts_match_expr("say \"hi\""),
            Some("\"say\" \"\"\"hi\"\"\"".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_rejects_empty_input() {
        assert_eq!(fts_match_expr("   "), None);
        assert_eq!(fts_match_expr(""), None);
    }

    #[test]
    fn article_cols_prefixed_prefixes_every_column() {
        let cols = article_cols_prefixed("a");
        assert!(cols.starts_with("a.id, a.curio_id"));
        assert!(cols.ends_with("a.source_updated_at"));
        assert!(!cols.contains(" ,"));
    }

    #[test]
    fn unfiltered_list_query_stays_join_free_with_keyset_order() {
        let (sql, binds) = list_articles_query(&ListArticles::default());
        assert!(!sql.contains("JOIN"), "no filters → no joins: {sql}");
        assert!(sql.ends_with("ORDER BY a.id DESC LIMIT ?"));
        assert_eq!(binds.len(), 2, "before + limit");
    }

    #[test]
    fn state_filters_join_and_coalesce_missing_rows() {
        let (sql, binds) = list_articles_query(&ListArticles {
            feed_id: Some(FeedId(7)),
            read: Some(false),
            archived: Some(true),
            ..ListArticles::default()
        });
        assert!(sql.contains("LEFT JOIN article_state s ON s.article_id = a.id"));
        assert!(sql.contains("COALESCE(s.is_read, 0) = ?"));
        assert!(sql.contains("COALESCE(s.is_archived, 0) = ?"));
        assert!(sql.contains("a.feed_id = ?"));
        assert!(!sql.contains("is_starred"), "unset filters stay out");
        assert!(sql.ends_with("ORDER BY a.id DESC LIMIT ?"), "order fixed");
        assert_eq!(binds.len(), 5, "before + feed + 2 flags + limit");
    }

    #[test]
    fn tag_filter_binds_the_tag_instead_of_splicing_it() {
        let (sql, binds) = list_articles_query(&ListArticles {
            tag: Some("rust'; DROP TABLE articles; --".to_owned()),
            ..ListArticles::default()
        });
        assert!(sql.contains("t.name = ?"));
        assert!(!sql.contains("DROP TABLE"), "tag value never enters SQL");
        assert!(
            !sql.contains("article_state"),
            "tag alone needs no state join"
        );
        assert_eq!(binds.len(), 3, "before + tag + limit");
    }
}
