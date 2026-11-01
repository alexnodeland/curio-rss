//! Feed subscriptions: add/remove, lookup, listing, fetch state and
//! per-feed settings (tags, title, full-text, private-network opt-in, order).

use super::rows::{RawFeed, feed_by_id, insert_intent, normalize_tags, raw_feed};
use super::{FEED_COLS, begin_write};
use crate::model::{Feed, FeedId, FeedStatus, NewFeed};
use crate::storage::{Storage, StorageError};
use curio_types::{EventEnvelope, EventPayload, Timestamp};
use rusqlite::OptionalExtension as _;

impl Storage {
    /// Subscribes to a feed and stages the `feed.added` event. Tags are
    /// normalized first (trimmed, empties dropped, deduplicated in
    /// first-seen order) so the staged event satisfies the published
    /// schema's `minLength: 1` / `uniqueItems` constraints no matter how
    /// sloppy the input (CLI flags, OPML categories).
    ///
    /// # Errors
    ///
    /// [`StorageError::Duplicate`] (`entity: "feed"`) if the URL is
    /// already subscribed; any other database error.
    pub fn add_feed(&self, new: NewFeed) -> Result<(Feed, EventEnvelope), StorageError> {
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            let now = Timestamp::now();
            let tags = normalize_tags(new.tags);
            tx.prepare_cached(
                "INSERT INTO feeds (url, title, tags, added_at, modified_at, sort_order) \
                 VALUES (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(sort_order), 0) + 1 FROM feeds))",
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
            let tx = begin_write(conn)?;
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
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {FEED_COLS} FROM feeds ORDER BY sort_order, id"
            ))?;
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

    /// Flips a feed's full-text mode: fetch + readability-extract the
    /// source page of every new article at refresh time.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn set_feed_full_text(&self, id: FeedId, enabled: bool) -> Result<(), StorageError> {
        self.write(move |conn| {
            let n = conn
                .prepare_cached(
                    "UPDATE feeds SET fetch_full_text = ?2, modified_at = ?3 WHERE id = ?1",
                )?
                .execute((id.0, enabled, Timestamp::now().to_string()))?;
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

    /// Replaces a feed's tags wholesale — the move-to-folder / re-tag path
    /// (folders are a `/`-path-tag rendering). Unlike [`Self::update_feed_metadata`]
    /// this is an unconditional overwrite: a user reorganizing folders means
    /// *exactly* these tags. Tags are normalized (trimmed, empties dropped,
    /// deduped first-seen) so the row keeps the shape `add_feed` writes.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist; JSON or
    /// database errors.
    pub fn set_feed_tags(&self, id: FeedId, tags: Vec<String>) -> Result<(), StorageError> {
        let tags = normalize_tags(tags);
        self.write(move |conn| {
            let n = conn
                .prepare_cached("UPDATE feeds SET tags = ?2, modified_at = ?3 WHERE id = ?1")?
                .execute((
                    id.0,
                    serde_json::to_string(&tags)?,
                    Timestamp::now().to_string(),
                ))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    /// Renames a feed — an unconditional title overwrite (unlike
    /// [`Self::update_feed_metadata`], which only *fills* a NULL title so a
    /// fetch never clobbers a human edit). An empty/whitespace title clears it
    /// to NULL, so the feed falls back to showing its URL; a later fetch may
    /// then re-fill the feed-provided title.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn set_feed_title(&self, id: FeedId, title: Option<String>) -> Result<(), StorageError> {
        let title = title.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
        self.write(move |conn| {
            let n = conn
                .prepare_cached("UPDATE feeds SET title = ?2, modified_at = ?3 WHERE id = ?1")?
                .execute((id.0, title, Timestamp::now().to_string()))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    /// Overwrites a feed's site URL and description with the user's edits —
    /// the edit-feed counterpart to the fetch-fill
    /// [`Self::update_feed_metadata`], which only COALESCE-*fills* a NULL so a
    /// refresh never clobbers a human edit. This is an unconditional overwrite:
    /// editing the field in the modal means *exactly* this value. An
    /// empty/whitespace value clears the field to NULL (a later fetch may then
    /// re-fill it via `update_feed_metadata`). The display title stays on
    /// [`Self::set_feed_title`].
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the feed does not exist.
    pub fn set_feed_metadata(
        &self,
        id: FeedId,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), StorageError> {
        let clean =
            |value: Option<String>| value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
        let site_url = clean(site_url);
        let description = clean(description);
        self.write(move |conn| {
            let n = conn
                .prepare_cached(
                    "UPDATE feeds SET site_url = ?2, description = ?3, \
                     modified_at = ?4 WHERE id = ?1",
                )?
                .execute((id.0, site_url, description, Timestamp::now().to_string()))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "feed" });
            }
            Ok(())
        })
    }

    /// Rewrites feed ordering: assigns `sort_order = position` for each id in
    /// `ordered`, in one transaction. Callers pass the complete new sequence
    /// (the sidebar's flattened order); unknown ids are simply skipped.
    /// DB-local — feed order is a local preference the events contract does
    /// not model, so no envelope is staged.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn reorder_feeds(&self, ordered: &[FeedId]) -> Result<(), StorageError> {
        let ids: Vec<i64> = ordered.iter().map(|feed| feed.0).collect();
        self.write(move |conn| {
            let tx = begin_write(conn)?;
            {
                let mut stmt =
                    tx.prepare_cached("UPDATE feeds SET sort_order = ?2 WHERE id = ?1")?;
                for (position, id) in ids.iter().enumerate() {
                    stmt.execute((id, i64::try_from(position).unwrap_or(i64::MAX)))?;
                }
            }
            tx.commit()?;
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
            let tx = begin_write(conn)?;
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
}
