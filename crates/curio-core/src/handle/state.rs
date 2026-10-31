//! Per-article state: star, read-later, archive, read, opened, tags.

use crate::model::{ArticleId, FeedId};

use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Star / unstar; emits `article.starred` (with tags) or the
    /// `article.unstarred` negation. Idempotent — `false` means no
    /// change and no event.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_starred(&self, id: ArticleId, starred: bool) -> Result<bool, CoreError> {
        let staged = if starred {
            self.storage.star_article(id)?
        } else {
            self.storage.unstar_article(id)?
        };
        Ok(self.emit_if(staged.is_some()))
    }

    /// Add to / remove from read-later; emits `article.read_later.added`
    /// or the `.removed` negation. Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_read_later(&self, id: ArticleId, read_later: bool) -> Result<bool, CoreError> {
        let staged = if read_later {
            self.storage.add_read_later(id)?
        } else {
            self.storage.remove_read_later(id)?
        };
        Ok(self.emit_if(staged.is_some()))
    }

    /// Archive / unarchive; emits `article.archived` or the
    /// `article.unarchived` negation. Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_archived(&self, id: ArticleId, archived: bool) -> Result<bool, CoreError> {
        let staged = if archived {
            self.storage.archive_article(id)?
        } else {
            self.storage.unarchive_article(id)?
        };
        Ok(self.emit_if(staged.is_some()))
    }

    /// Read/unread. DB-local: `curio.events.v1` defines no read event.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn mark_read(&self, id: ArticleId, read: bool) -> Result<bool, CoreError> {
        Ok(self.storage.mark_read(id, read)?)
    }

    /// Marks every unread article read — one feed (`Some`) or the whole
    /// library (`None`). Returns how many were changed. DB-local, no event.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn mark_all_read(&self, feed_id: Option<FeedId>) -> Result<u64, CoreError> {
        Ok(self.storage.mark_all_read(feed_id)?)
    }

    /// Records an open-for-reading; emits `article.opened`.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn record_opened(&self, id: ArticleId, dwell_ms: Option<u64>) -> Result<(), CoreError> {
        self.storage.record_opened(id, dwell_ms)?;
        self.emit();
        Ok(())
    }

    /// Tags an article; emits `article.tagged`. Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn tag_article(&self, id: ArticleId, tag: &str) -> Result<bool, CoreError> {
        let staged = self.storage.tag_article(id, tag)?;
        Ok(self.emit_if(staged.is_some()))
    }

    /// Untags an article; emits the `article.untagged` negation.
    /// Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn untag_article(&self, id: ArticleId, tag: &str) -> Result<bool, CoreError> {
        let staged = self.storage.untag_article(id, tag)?;
        Ok(self.emit_if(staged.is_some()))
    }
}
