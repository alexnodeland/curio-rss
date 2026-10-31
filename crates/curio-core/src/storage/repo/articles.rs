//! Articles: batch upsert, lookup, keyset listing, unread counts, search,
//! deletion and content replacement.

use super::rows::{
    RawArticle, article_by_id, article_cols_prefixed, fts_match_expr, list_articles_query,
    raw_article, scoped_dedupe_key,
};
use super::{ARTICLE_COLS, ListArticles, UpsertOutcome, begin_write};
use crate::model::{Article, ArticleContent, ArticleId, FeedId, NewArticle};
use crate::storage::{Storage, StorageError};
use curio_types::{CurioId, Timestamp};
use rusqlite::OptionalExtension as _;
use std::collections::BTreeMap;

impl Storage {
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
            let tx = begin_write(conn)?;
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
                         word_count = ?9, lead_image = ?10, source_updated_at = ?11, \
                         modified_at = ?12 WHERE id = ?1",
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
                        &article.lead_image,
                        article.source_updated_at.map(|t| t.to_string()),
                        &now,
                    ))?;
                    outcome.updated += 1;
                } else {
                    tx.prepare_cached(
                        "INSERT INTO articles (curio_id, feed_id, dedupe_key, title, \
                         source_url, author, published_at, content_html, content_text, lang, \
                         word_count, lead_image, saved_at, source_updated_at, modified_at) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, \
                         ?15)",
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
                        &article.lead_image,
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

    /// The row id of the article stored under `dedupe_key` within
    /// `feed_id`'s provenance scope (`f<id>:` for a feed, `m:` for manual /
    /// imported saves), if one exists. The importer upserts a feedless
    /// article and then uses this to look its id back up so it can apply
    /// read-later and tags.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn article_id_by_dedupe_key(
        &self,
        feed_id: Option<FeedId>,
        dedupe_key: &str,
    ) -> Result<Option<ArticleId>, StorageError> {
        let scoped = scoped_dedupe_key(feed_id, dedupe_key);
        self.read(move |conn| {
            Ok(conn
                .prepare_cached("SELECT id FROM articles WHERE dedupe_key = ?1")?
                .query_row([&scoped], |row| row.get::<_, i64>(0))
                .optional()?
                .map(ArticleId))
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
    /// has never been touched and counts as unread; starred and read-later
    /// do not affect unreadness, but **archived articles are excluded** —
    /// archiving takes an item out of the reading flow, badge included.
    ///
    /// # Errors
    ///
    /// Database errors.
    pub fn unread_counts(&self) -> Result<BTreeMap<Option<FeedId>, u64>, StorageError> {
        self.read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT a.feed_id, COUNT(*) FROM articles a \
                 LEFT JOIN article_state s ON s.article_id = a.id \
                 WHERE COALESCE(s.is_read, 0) = 0 AND COALESCE(s.is_archived, 0) = 0 \
                 GROUP BY a.feed_id",
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

    /// Replaces an article's stored content in place — the readability
    /// "hydrate full article" write. Unlike [`Self::upsert_articles`]
    /// (dedupe-key ingest of new rows), this is a by-id overwrite of the
    /// content columns of an existing row; the `articles_fts_au` trigger
    /// reindexes on the `content_text` change automatically.
    ///
    /// # Errors
    ///
    /// [`StorageError::NotFound`] if the article does not exist.
    pub fn update_article_content(
        &self,
        id: ArticleId,
        content: &ArticleContent,
        word_count: Option<u32>,
    ) -> Result<(), StorageError> {
        let html = content.html.clone();
        let text = content.text.clone();
        self.write(move |conn| {
            let n = conn
                .prepare_cached(
                    "UPDATE articles SET content_html = ?2, content_text = ?3, \
                     word_count = ?4, modified_at = ?5 WHERE id = ?1",
                )?
                .execute((id.0, html, text, word_count, Timestamp::now().to_string()))?;
            if n == 0 {
                return Err(StorageError::NotFound { entity: "article" });
            }
            Ok(())
        })
    }
}
