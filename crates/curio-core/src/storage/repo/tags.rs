//! Article tags: tag/untag (with their events), per-article and global listing.

use super::begin_write;
use super::rows::{insert_intent, require_article, validated_tag};
use crate::model::ArticleId;
use crate::storage::{Storage, StorageError};
use curio_types::{EventEnvelope, EventPayload};

impl Storage {
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
            let tx = begin_write(conn)?;
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
            let tx = begin_write(conn)?;
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
}
