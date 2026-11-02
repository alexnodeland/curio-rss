//! Per-article state: star, read-later, archive, read, reading position,
//! opened, tags.

use crate::model::{ArticleFlag, ArticleId, FeedId, ReadProgress};

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
    /// library (`None`). Returns the ids that changed (ascending), so a head
    /// can offer an exact undo via [`Self::mark_read_many`]. DB-local, no
    /// event.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn mark_all_read(&self, feed_id: Option<FeedId>) -> Result<Vec<ArticleId>, CoreError> {
        Ok(self.storage.mark_all_read(feed_id)?)
    }

    /// Read/unread on many articles at once (one transaction). Unknown ids
    /// are skipped; returns how many changed. DB-local, no event.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn mark_read_many(&self, ids: &[ArticleId], read: bool) -> Result<u64, CoreError> {
        Ok(self.storage.mark_read_many(ids, read)?)
    }

    /// Sets one flag on many articles at once — one transaction, staging
    /// each changed article's event (none for read), then one emit. Unknown
    /// ids are skipped; returns the ids that changed, which an undo hands
    /// back with the opposite `value`.
    ///
    /// # Errors
    ///
    /// Storage errors (nothing is written on error).
    pub fn set_flag_many(
        &self,
        ids: &[ArticleId],
        flag: ArticleFlag,
        value: bool,
    ) -> Result<Vec<ArticleId>, CoreError> {
        let changed = self.storage.set_flag_many(ids, flag, value)?;
        self.emit_if(flag != ArticleFlag::Read && !changed.is_empty());
        Ok(changed)
    }

    /// Records how far through the body the reader is — what a reopen
    /// resumes at. DB-local, no event. Idempotent: `false` when the stored
    /// position is already `progress`.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_read_progress(
        &self,
        id: ArticleId,
        progress: ReadProgress,
    ) -> Result<bool, CoreError> {
        Ok(self.storage.set_read_progress(id, progress)?)
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
