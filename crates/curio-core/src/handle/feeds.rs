//! Feed subscriptions: add, remove, list, per-feed settings, ordering.

use std::sync::PoisonError;

use crate::model::{Feed, FeedId, NewFeed};

use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Subscribes to a feed; emits `feed.added` (with tags).
    ///
    /// # Errors
    ///
    /// Storage (including duplicate URL) or emission errors.
    pub fn add_feed(&self, new: NewFeed) -> Result<Feed, CoreError> {
        let (feed, _envelope) = self.storage.add_feed(new)?;
        self.emit()?;
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
        self.emit()?;
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
