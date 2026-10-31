//! Feed subscriptions: add, remove, list, lookup, per-feed settings,
//! lifecycle status, fetch-filled metadata, ordering, and the fetch log.

use std::sync::PoisonError;

use crate::model::{Feed, FeedId, FeedStatus, FetchRecord, NewFeed};

use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Subscribes to a feed; emits `feed.added` (with tags).
    ///
    /// # Errors
    ///
    /// Storage (including duplicate URL) or emission errors.
    pub fn add_feed(&self, new: NewFeed) -> Result<Feed, CoreError> {
        let (feed, _envelope) = self.storage.add_feed(new)?;
        self.emit();
        Ok(feed)
    }

    /// Unsubscribes a feed; emits the `feed.removed` negation. Articles
    /// keep their rows.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn remove_feed(&self, id: FeedId) -> Result<(), CoreError> {
        self.storage.remove_feed(id)?;
        self.refresh_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        // Its refresh lock file goes too, best effort. Safe even while
        // another process holds it: a lock file deleted under a holder is
        // re-created, not trusted (see `crate::lock`). A refresh of this
        // feed already under way in another process can re-create it
        // after this delete (it only finds the feed gone once it holds
        // the lock), leaving one empty, gitignored lock file behind.
        let lock_file = super::refresh::feed_lock_path(&self.profile_dir, id);
        if let Err(err) = std::fs::remove_file(&lock_file)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %lock_file.display(), %err, "could not delete a feed's refresh lock file");
        }
        self.emit();
        Ok(())
    }

    /// Every subscription.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_feeds(&self) -> Result<Vec<Feed>, CoreError> {
        Ok(self.storage.list_feeds()?)
    }

    /// One feed by id.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_feed(&self, id: FeedId) -> Result<Option<Feed>, CoreError> {
        Ok(self.storage.get_feed(id)?)
    }

    /// Contract W1: flips the per-feed private-network exemption. This
    /// is the *only* path to the flag — explicit configuration, never
    /// feed content.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn set_feed_allow_private_network(&self, id: FeedId, allow: bool) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_allow_private_network(id, allow)?)
    }

    /// Flips a feed's full-text mode: when on, every refresh hydrates the
    /// feed's *new* articles from their source pages (readability-extract
    /// through the policed client) — the upgrade for feeds that ship only
    /// excerpts. DB-local, no event: how a feed's content is fetched is a
    /// local reading preference, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn set_feed_full_text(&self, id: FeedId, enabled: bool) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_full_text(id, enabled)?)
    }

    /// Replaces a feed's tags — the move-to-folder / re-tag path (folders
    /// are a `/`-path-tag rendering in the head). A wholesale overwrite that
    /// is DB-local, like the feed-metadata edit: it stages no event, since
    /// consumers key feeds by URL and folder membership is a local
    /// reading-organization concern, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_tags(&self, id: FeedId, tags: Vec<String>) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_tags(id, tags)?)
    }

    /// Renames a feed — an unconditional title overwrite (an empty title
    /// clears it, falling back to the URL). DB-local, no event: the display
    /// title is not part of the published contract, and consumers key feeds
    /// by URL. This is the user-rename counterpart to the fetch-fill
    /// `update_feed_metadata`, which can only fill a NULL title.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_title(&self, id: FeedId, title: Option<String>) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_title(id, title)?)
    }

    /// Overwrites a feed's site URL and description — the edit-feed modal's
    /// user-edit path. DB-local, no contract event (the head's command layer
    /// emits the UI `FeedsChanged`): site URL / description are not part of
    /// the published contract, and consumers key feeds by URL. This is the
    /// user-overwrite counterpart to the fetch-fill `update_feed_metadata`,
    /// which only fills a NULL field so a refresh never clobbers a human edit.
    /// An empty value clears the field. The display title stays on
    /// [`Self::set_feed_title`].
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_metadata(
        &self,
        id: FeedId,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_metadata(id, site_url, description)?)
    }

    /// Looks a feed up by its subscription URL (exact match).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_feed_by_url(&self, url: &str) -> Result<Option<Feed>, CoreError> {
        Ok(self.storage.get_feed_by_url(url)?)
    }

    /// Sets a feed's lifecycle status (pause, dead-on-410, reactivate).
    /// DB-local, no contract event: feed status is a local scheduling
    /// concern, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `NotFound` for an unknown feed;
    /// storage errors.
    pub fn set_feed_status(&self, id: FeedId, status: FeedStatus) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_status(id, status)?)
    }

    /// Fills in feed metadata learned from a fetch, without clobbering
    /// anything already set (human edits win over parsed values) — the
    /// fetch-fill counterpart to [`Self::set_feed_metadata`]. DB-local, no
    /// contract event.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `NotFound` for an unknown feed;
    /// storage errors.
    pub fn update_feed_metadata(
        &self,
        id: FeedId,
        title: Option<String>,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), CoreError> {
        Ok(self
            .storage
            .update_feed_metadata(id, title, site_url, description)?)
    }

    /// The most recent fetch attempts for a feed (the `fetch_log`),
    /// newest first.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn recent_fetches(&self, id: FeedId, limit: u32) -> Result<Vec<FetchRecord>, CoreError> {
        Ok(self.storage.recent_fetches(id, limit)?)
    }

    /// Rewrites the sidebar feed order to `ordered` (drag-to-reorder). The
    /// caller passes the complete new sequence. DB-local, no event: feed
    /// ordering is a local reading-organization preference, not part of the
    /// published contract.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn reorder_feeds(&self, ordered: &[FeedId]) -> Result<(), CoreError> {
        Ok(self.storage.reorder_feeds(ordered)?)
    }
}
