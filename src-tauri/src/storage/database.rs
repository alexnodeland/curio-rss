//! SQLite database operations.

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::core::models::{
    Article, ArticleFilter, ArticlePage, Feed, FeedUpdate, Folder, FolderUpdate, MediaAttachment,
    Pagination, ViewMode,
};
use crate::error::{CoreError, InfraError};
use crate::storage::migrations;

/// Database connection wrapper
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Open or create a database at the given path
    pub fn open(path: &Path) -> Result<Self, InfraError> {
        let conn = Connection::open(path)?;

        // Enable foreign keys
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        // Run migrations
        migrations::run_migrations(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Create an in-memory database (for testing)
    pub fn in_memory() -> Result<Self, InfraError> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        migrations::run_migrations(&conn)?;

        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // =========================================================================
    // Feed Operations
    // =========================================================================

    /// Insert a new feed
    pub fn insert_feed(&self, feed: &Feed) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO feeds (
                id, url, title, description, site_url, icon_url, view_mode,
                tags, folder_id, etag, last_modified, last_fetched, last_error,
                refresh_interval, notify_new, position, created_at, updated_at
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                feed.id.to_string(),
                feed.url,
                feed.title,
                feed.description,
                feed.site_url,
                feed.icon_url,
                serde_json::to_string(&feed.view_mode).unwrap_or_default(),
                serde_json::to_string(&feed.tags).unwrap_or_default(),
                feed.folder_id.map(|id| id.to_string()),
                feed.etag,
                feed.last_modified,
                feed.last_fetched.map(|dt| dt.to_rfc3339()),
                feed.last_error,
                feed.refresh_interval,
                feed.notify_new as i32,
                feed.position,
                feed.created_at.to_rfc3339(),
                feed.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Get a feed by ID
    pub fn get_feed(&self, id: Uuid) -> Result<Option<Feed>, InfraError> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT * FROM feeds WHERE id = ?",
                params![id.to_string()],
                |row| self.row_to_feed(row),
            )
            .optional()?;
        Ok(result)
    }

    /// Get a feed by URL
    pub fn get_feed_by_url(&self, url: &str) -> Result<Option<Feed>, InfraError> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row("SELECT * FROM feeds WHERE url = ?", params![url], |row| {
                self.row_to_feed(row)
            })
            .optional()?;
        Ok(result)
    }

    /// Get all feeds
    pub fn get_all_feeds(&self) -> Result<Vec<Feed>, InfraError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM feeds ORDER BY position, title")?;
        let feeds = stmt
            .query_map([], |row| self.row_to_feed(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(feeds)
    }

    /// Update a feed
    pub fn update_feed(&self, id: Uuid, update: &FeedUpdate) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        let mut updates = Vec::new();
        let mut values: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref title) = update.title {
            updates.push("title = ?");
            values.push(Box::new(title.clone()));
        }
        if let Some(ref desc) = update.description {
            updates.push("description = ?");
            values.push(Box::new(desc.clone()));
        }
        if let Some(ref mode) = update.view_mode {
            updates.push("view_mode = ?");
            values.push(Box::new(serde_json::to_string(mode).unwrap_or_default()));
        }
        if let Some(ref tags) = update.tags {
            updates.push("tags = ?");
            values.push(Box::new(serde_json::to_string(tags).unwrap_or_default()));
        }
        if let Some(ref folder) = update.folder_id {
            updates.push("folder_id = ?");
            values.push(Box::new(folder.map(|id| id.to_string())));
        }
        if let Some(notify) = update.notify_new {
            updates.push("notify_new = ?");
            values.push(Box::new(notify as i32));
        }
        if let Some(pos) = update.position {
            updates.push("position = ?");
            values.push(Box::new(pos));
        }

        if updates.is_empty() {
            return Ok(());
        }

        updates.push("updated_at = ?");
        values.push(Box::new(Utc::now().to_rfc3339()));

        let sql = format!("UPDATE feeds SET {} WHERE id = ?", updates.join(", "));
        values.push(Box::new(id.to_string()));

        let params: Vec<&dyn rusqlite::ToSql> = values.iter().map(|v| v.as_ref()).collect();
        conn.execute(&sql, params.as_slice())?;

        Ok(())
    }

    /// Delete a feed
    pub fn delete_feed(&self, id: Uuid) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM feeds WHERE id = ?", params![id.to_string()])?;
        Ok(())
    }

    /// Create a new feed (alias for insert_feed, returns the feed)
    pub fn create_feed(&self, feed: &Feed) -> Result<Feed, InfraError> {
        self.insert_feed(feed)?;
        Ok(feed.clone())
    }

    /// Get all feeds (alias)
    pub fn get_feeds(&self) -> Result<Vec<Feed>, InfraError> {
        self.get_all_feeds()
    }

    /// Update feed fetch metadata
    pub fn update_feed_fetch(
        &self,
        id: Uuid,
        etag: Option<&str>,
        last_modified: Option<&str>,
        error: Option<&str>,
    ) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            UPDATE feeds SET
                etag = ?,
                last_modified = ?,
                last_fetched = ?,
                last_error = ?,
                updated_at = ?
            WHERE id = ?
            "#,
            params![
                etag,
                last_modified,
                Utc::now().to_rfc3339(),
                error,
                Utc::now().to_rfc3339(),
                id.to_string(),
            ],
        )?;
        Ok(())
    }

    // =========================================================================
    // Article Operations
    // =========================================================================

    /// Insert or update articles (upsert)
    pub fn upsert_articles(&self, articles: &[Article]) -> Result<(i32, i32), InfraError> {
        let conn = self.conn.lock().unwrap();
        let mut new_count = 0;
        let mut updated_count = 0;

        for article in articles {
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM articles WHERE feed_id = ? AND guid = ?",
                    params![article.feed_id.to_string(), article.guid],
                    |row| row.get(0),
                )
                .optional()?;

            if existing.is_some() {
                // Update existing
                conn.execute(
                    r#"
                    UPDATE articles SET
                        title = ?, url = ?, content_html = ?, content_text = ?,
                        summary = ?, author = ?, thumbnail_url = ?,
                        published_at = ?, updated_at = ?
                    WHERE feed_id = ? AND guid = ?
                    "#,
                    params![
                        article.title,
                        article.url,
                        article.content_html,
                        article.content_text,
                        article.summary,
                        article.author,
                        article.thumbnail_url,
                        article.published_at.map(|dt| dt.to_rfc3339()),
                        article.updated_at.map(|dt| dt.to_rfc3339()),
                        article.feed_id.to_string(),
                        article.guid,
                    ],
                )?;
                updated_count += 1;
            } else {
                // Insert new
                conn.execute(
                    r#"
                    INSERT INTO articles (
                        id, feed_id, guid, title, url, content_html, content_text,
                        summary, author, thumbnail_url, published_at, updated_at, fetched_at,
                        reddit_score, reddit_num_comments, youtube_duration,
                        podcast_duration, podcast_progress
                    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                    "#,
                    params![
                        article.id.to_string(),
                        article.feed_id.to_string(),
                        article.guid,
                        article.title,
                        article.url,
                        article.content_html,
                        article.content_text,
                        article.summary,
                        article.author,
                        article.thumbnail_url,
                        article.published_at.map(|dt| dt.to_rfc3339()),
                        article.updated_at.map(|dt| dt.to_rfc3339()),
                        article.fetched_at.to_rfc3339(),
                        article.reddit_score,
                        article.reddit_num_comments,
                        article.youtube_duration,
                        article.podcast_duration,
                        article.podcast_progress,
                    ],
                )?;
                new_count += 1;

                // Insert media attachments
                for media in &article.media {
                    conn.execute(
                        r#"
                        INSERT INTO media_attachments (id, article_id, url, mime_type, size_bytes, title)
                        VALUES (?, ?, ?, ?, ?, ?)
                        "#,
                        params![
                            Uuid::new_v4().to_string(),
                            article.id.to_string(),
                            media.url,
                            media.mime_type,
                            media.size_bytes,
                            media.title,
                        ],
                    )?;
                }
            }
        }

        Ok((new_count, updated_count))
    }

    /// Get an article by ID
    pub fn get_article(&self, id: Uuid) -> Result<Option<Article>, InfraError> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT * FROM articles WHERE id = ?",
                params![id.to_string()],
                |row| self.row_to_article(row),
            )
            .optional()?;

        if let Some(mut article) = result {
            article.media = self.get_media_for_article(&conn, article.id)?;
            return Ok(Some(article));
        }

        Ok(None)
    }

    /// Get articles with filtering and pagination
    pub fn get_articles(
        &self,
        filter: &ArticleFilter,
        pagination: &Pagination,
    ) -> Result<ArticlePage, InfraError> {
        let conn = self.conn.lock().unwrap();

        let mut where_clauses = Vec::new();
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(feed_id) = filter.feed_id {
            where_clauses.push("a.feed_id = ?");
            params.push(Box::new(feed_id.to_string()));
        }

        if filter.unread_only {
            where_clauses.push("a.is_read = 0");
        }

        if filter.starred_only {
            where_clauses.push("a.is_starred = 1");
        }

        if filter.read_later_only {
            where_clauses.push("a.is_read_later = 1");
        }

        if !filter.archived {
            where_clauses.push("a.is_archived = 0");
        }

        let where_sql = if where_clauses.is_empty() {
            "1=1".to_string()
        } else {
            where_clauses.join(" AND ")
        };

        // Count total
        let count_sql = format!("SELECT COUNT(*) FROM articles a WHERE {}", where_sql);
        let count_params: Vec<&dyn rusqlite::ToSql> = params.iter().map(|v| v.as_ref()).collect();
        let total: i64 = conn.query_row(&count_sql, count_params.as_slice(), |row| row.get(0))?;

        // Get page
        let query_sql = format!(
            r#"
            SELECT a.* FROM articles a
            WHERE {}
            ORDER BY a.published_at DESC NULLS LAST, a.fetched_at DESC
            LIMIT ? OFFSET ?
            "#,
            where_sql
        );

        params.push(Box::new(pagination.limit()));
        params.push(Box::new(pagination.offset()));

        let query_params: Vec<&dyn rusqlite::ToSql> = params.iter().map(|v| v.as_ref()).collect();
        let mut stmt = conn.prepare(&query_sql)?;
        let articles: Vec<Article> = stmt
            .query_map(query_params.as_slice(), |row| self.row_to_article(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(ArticlePage::new(articles, total, pagination))
    }

    /// Mark articles as read
    pub fn mark_read(&self, ids: &[Uuid]) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        for id in ids {
            conn.execute(
                "UPDATE articles SET is_read = 1, read_at = ? WHERE id = ?",
                params![now, id.to_string()],
            )?;
        }
        Ok(())
    }

    /// Mark articles as unread
    pub fn mark_unread(&self, ids: &[Uuid]) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();

        for id in ids {
            conn.execute(
                "UPDATE articles SET is_read = 0, read_at = NULL WHERE id = ?",
                params![id.to_string()],
            )?;
        }
        Ok(())
    }

    /// Toggle starred status
    pub fn toggle_starred(&self, id: Uuid) -> Result<bool, InfraError> {
        let conn = self.conn.lock().unwrap();

        let current: i32 = conn.query_row(
            "SELECT is_starred FROM articles WHERE id = ?",
            params![id.to_string()],
            |row| row.get(0),
        )?;

        let new_value = 1 - current;
        conn.execute(
            "UPDATE articles SET is_starred = ? WHERE id = ?",
            params![new_value, id.to_string()],
        )?;

        Ok(new_value == 1)
    }

    /// Search articles using full-text search
    pub fn search_articles(&self, query: &str, limit: i32) -> Result<Vec<Article>, InfraError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            r#"
            SELECT a.* FROM articles a
            JOIN articles_fts fts ON a.rowid = fts.rowid
            WHERE articles_fts MATCH ?
            ORDER BY rank
            LIMIT ?
            "#,
        )?;

        let articles: Vec<Article> = stmt
            .query_map(params![query, limit], |row| self.row_to_article(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(articles)
    }

    /// Add article to read later queue
    pub fn add_to_read_later(&self, id: Uuid) -> Result<i32, InfraError> {
        let conn = self.conn.lock().unwrap();
        let now = Utc::now().to_rfc3339();

        // Get the next position (max + 1)
        let max_pos: Option<i32> = conn
            .query_row(
                "SELECT MAX(read_later_position) FROM articles WHERE is_read_later = 1",
                [],
                |row| row.get(0),
            )
            .optional()?
            .flatten();

        let new_position = max_pos.unwrap_or(0) + 1;

        conn.execute(
            "UPDATE articles SET is_read_later = 1, read_later_at = ?, read_later_position = ? WHERE id = ?",
            params![now, new_position, id.to_string()],
        )?;

        Ok(new_position)
    }

    /// Remove article from read later queue
    pub fn remove_from_read_later(&self, id: Uuid) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "UPDATE articles SET is_read_later = 0, read_later_at = NULL, read_later_position = NULL WHERE id = ?",
            params![id.to_string()],
        )?;

        Ok(())
    }

    /// Get all articles in read later queue ordered by position
    pub fn get_read_later(&self) -> Result<Vec<Article>, InfraError> {
        let conn = self.conn.lock().unwrap();

        let mut stmt = conn.prepare(
            "SELECT * FROM articles WHERE is_read_later = 1 ORDER BY read_later_position ASC",
        )?;

        let articles: Vec<Article> = stmt
            .query_map([], |row| self.row_to_article(row))?
            .filter_map(|r| r.ok())
            .collect();

        Ok(articles)
    }

    /// Reorder read later queue
    pub fn reorder_read_later(&self, article_ids: &[Uuid]) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();

        for (index, id) in article_ids.iter().enumerate() {
            conn.execute(
                "UPDATE articles SET read_later_position = ? WHERE id = ? AND is_read_later = 1",
                params![index as i32 + 1, id.to_string()],
            )?;
        }

        Ok(())
    }

    // =========================================================================
    // Folder Operations
    // =========================================================================

    /// Insert a new folder
    pub fn insert_folder(&self, folder: &Folder) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            r#"
            INSERT INTO folders (id, name, parent_id, icon, color, view_mode, position, created_at, updated_at)
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            "#,
            params![
                folder.id.to_string(),
                folder.name,
                folder.parent_id.map(|id| id.to_string()),
                folder.icon,
                folder.color,
                folder.view_mode.map(|m| serde_json::to_string(&m).unwrap_or_default()),
                folder.position,
                folder.created_at.to_rfc3339(),
                folder.updated_at.to_rfc3339(),
            ],
        )?;
        Ok(())
    }

    /// Get a folder by ID
    pub fn get_folder(&self, id: Uuid) -> Result<Option<Folder>, InfraError> {
        let conn = self.conn.lock().unwrap();
        let result = conn
            .query_row(
                "SELECT * FROM folders WHERE id = ?",
                params![id.to_string()],
                |row| self.row_to_folder(row),
            )
            .optional()?;
        Ok(result)
    }

    /// Get all folders
    pub fn get_all_folders(&self) -> Result<Vec<Folder>, InfraError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT * FROM folders ORDER BY position, name")?;
        let folders = stmt
            .query_map([], |row| self.row_to_folder(row))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(folders)
    }

    /// Delete a folder
    pub fn delete_folder(&self, id: Uuid) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM folders WHERE id = ?", params![id.to_string()])?;
        Ok(())
    }

    /// Create a new folder
    pub fn create_folder(&self, name: &str, parent_id: Option<Uuid>) -> Result<Folder, InfraError> {
        let folder = Folder {
            id: Uuid::new_v4(),
            name: name.to_string(),
            parent_id,
            icon: None,
            color: None,
            view_mode: None,
            position: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        self.insert_folder(&folder)?;
        Ok(folder)
    }

    /// Get all folders (alias)
    pub fn get_folders(&self) -> Result<Vec<Folder>, InfraError> {
        self.get_all_folders()
    }

    /// Update a folder
    pub fn update_folder(
        &self,
        id: Uuid,
        updates: &std::collections::HashMap<String, String>,
    ) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();

        for (key, value) in updates {
            match key.as_str() {
                "icon" => {
                    conn.execute(
                        "UPDATE folders SET icon = ?, updated_at = ? WHERE id = ?",
                        params![value, Utc::now().to_rfc3339(), id.to_string()],
                    )?;
                }
                "color" => {
                    conn.execute(
                        "UPDATE folders SET color = ?, updated_at = ? WHERE id = ?",
                        params![value, Utc::now().to_rfc3339(), id.to_string()],
                    )?;
                }
                "name" => {
                    conn.execute(
                        "UPDATE folders SET name = ?, updated_at = ? WHERE id = ?",
                        params![value, Utc::now().to_rfc3339(), id.to_string()],
                    )?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    // =========================================================================
    // Media Attachment Operations
    // =========================================================================

    /// Get media attachments for an article (public version)
    pub fn get_media_attachments(
        &self,
        article_id: Uuid,
    ) -> Result<Vec<MediaAttachment>, InfraError> {
        let conn = self.conn.lock().unwrap();
        self.get_media_for_article(&conn, article_id)
    }

    /// Mark a media attachment as downloaded
    pub fn mark_media_downloaded(&self, id: Uuid, local_path: &str) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE media_attachments SET is_downloaded = 1, local_path = ? WHERE id = ?",
            params![local_path, id.to_string()],
        )?;
        Ok(())
    }

    /// Mark a media attachment as not downloaded
    pub fn mark_media_not_downloaded(&self, id: Uuid) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE media_attachments SET is_downloaded = 0, local_path = NULL WHERE id = ?",
            params![id.to_string()],
        )?;
        Ok(())
    }

    /// Update podcast playback progress
    pub fn update_podcast_progress(
        &self,
        article_id: Uuid,
        progress: i32,
    ) -> Result<(), InfraError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE articles SET podcast_progress = ? WHERE id = ?",
            params![progress, article_id.to_string()],
        )?;
        Ok(())
    }

    // =========================================================================
    // Helper Methods
    // =========================================================================

    fn row_to_feed(&self, row: &rusqlite::Row) -> rusqlite::Result<Feed> {
        let view_mode_str: String = row.get("view_mode")?;
        let tags_str: String = row.get("tags")?;
        let folder_id_str: Option<String> = row.get("folder_id")?;
        let last_fetched_str: Option<String> = row.get("last_fetched")?;
        let created_at_str: String = row.get("created_at")?;
        let updated_at_str: String = row.get("updated_at")?;

        Ok(Feed {
            id: Uuid::parse_str(&row.get::<_, String>("id")?).unwrap_or_default(),
            url: row.get("url")?,
            title: row.get("title")?,
            description: row.get("description")?,
            site_url: row.get("site_url")?,
            icon_url: row.get("icon_url")?,
            view_mode: serde_json::from_str(&view_mode_str).unwrap_or_default(),
            tags: serde_json::from_str(&tags_str).unwrap_or_default(),
            folder_id: folder_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
            etag: row.get("etag")?,
            last_modified: row.get("last_modified")?,
            last_fetched: last_fetched_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            last_error: row.get("last_error")?,
            refresh_interval: row.get("refresh_interval")?,
            notify_new: row.get::<_, i32>("notify_new")? != 0,
            position: row.get("position")?,
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn row_to_article(&self, row: &rusqlite::Row) -> rusqlite::Result<Article> {
        let published_str: Option<String> = row.get("published_at")?;
        let updated_str: Option<String> = row.get("updated_at")?;
        let fetched_str: String = row.get("fetched_at")?;
        let read_at_str: Option<String> = row.get("read_at")?;
        let read_later_at_str: Option<String> = row.get("read_later_at")?;

        Ok(Article {
            id: Uuid::parse_str(&row.get::<_, String>("id")?).unwrap_or_default(),
            feed_id: Uuid::parse_str(&row.get::<_, String>("feed_id")?).unwrap_or_default(),
            guid: row.get("guid")?,
            title: row.get("title")?,
            url: row.get("url")?,
            content_html: row.get("content_html")?,
            content_text: row.get("content_text")?,
            summary: row.get("summary")?,
            author: row.get("author")?,
            thumbnail_url: row.get("thumbnail_url")?,
            media: Vec::new(), // Loaded separately
            published_at: published_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            updated_at: updated_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            fetched_at: DateTime::parse_from_rfc3339(&fetched_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            is_read: row.get::<_, i32>("is_read")? != 0,
            is_starred: row.get::<_, i32>("is_starred")? != 0,
            is_read_later: row.get::<_, i32>("is_read_later")? != 0,
            is_archived: row.get::<_, i32>("is_archived")? != 0,
            read_at: read_at_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            read_later_at: read_later_at_str.and_then(|s| {
                DateTime::parse_from_rfc3339(&s)
                    .ok()
                    .map(|dt| dt.with_timezone(&Utc))
            }),
            read_later_position: row.get("read_later_position")?,
            reddit_score: row.get("reddit_score")?,
            reddit_num_comments: row.get("reddit_num_comments")?,
            youtube_duration: row.get("youtube_duration")?,
            podcast_duration: row.get("podcast_duration")?,
            podcast_progress: row.get("podcast_progress")?,
        })
    }

    fn row_to_folder(&self, row: &rusqlite::Row) -> rusqlite::Result<Folder> {
        let parent_id_str: Option<String> = row.get("parent_id")?;
        let view_mode_str: Option<String> = row.get("view_mode")?;
        let created_at_str: String = row.get("created_at")?;
        let updated_at_str: String = row.get("updated_at")?;

        Ok(Folder {
            id: Uuid::parse_str(&row.get::<_, String>("id")?).unwrap_or_default(),
            name: row.get("name")?,
            parent_id: parent_id_str.and_then(|s| Uuid::parse_str(&s).ok()),
            icon: row.get("icon")?,
            color: row.get("color")?,
            view_mode: view_mode_str.and_then(|s| serde_json::from_str(&s).ok()),
            position: row.get("position")?,
            created_at: DateTime::parse_from_rfc3339(&created_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: DateTime::parse_from_rfc3339(&updated_at_str)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
        })
    }

    fn get_media_for_article(
        &self,
        conn: &Connection,
        article_id: Uuid,
    ) -> Result<Vec<MediaAttachment>, InfraError> {
        let mut stmt = conn.prepare(
            "SELECT id, url, mime_type, size_bytes, title, is_downloaded, local_path FROM media_attachments WHERE article_id = ?",
        )?;

        let media = stmt
            .query_map(params![article_id.to_string()], |row| {
                let id_str: String = row.get("id")?;
                Ok(MediaAttachment {
                    id: Uuid::parse_str(&id_str).unwrap_or_default(),
                    url: row.get("url")?,
                    mime_type: row.get("mime_type")?,
                    size_bytes: row.get("size_bytes")?,
                    title: row.get("title")?,
                    is_downloaded: row.get::<_, i32>("is_downloaded")? != 0,
                    local_path: row.get("local_path")?,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(media)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_db() -> Database {
        Database::in_memory().unwrap()
    }

    #[test]
    fn test_insert_and_get_feed() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Example Feed");
        let feed_id = feed.id;

        db.insert_feed(&feed).unwrap();

        let retrieved = db.get_feed(feed_id).unwrap().unwrap();
        assert_eq!(retrieved.url, feed.url);
        assert_eq!(retrieved.title, feed.title);
    }

    #[test]
    fn test_get_feed_by_url() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/unique.xml", "Unique Feed");

        db.insert_feed(&feed).unwrap();

        let retrieved = db
            .get_feed_by_url("https://example.com/unique.xml")
            .unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().title, "Unique Feed");
    }

    #[test]
    fn test_get_all_feeds() {
        let db = setup_db();

        db.insert_feed(&Feed::new("https://example.com/1.xml", "Feed 1"))
            .unwrap();
        db.insert_feed(&Feed::new("https://example.com/2.xml", "Feed 2"))
            .unwrap();
        db.insert_feed(&Feed::new("https://example.com/3.xml", "Feed 3"))
            .unwrap();

        let feeds = db.get_all_feeds().unwrap();
        assert_eq!(feeds.len(), 3);
    }

    #[test]
    fn test_update_feed() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Original Title");
        let feed_id = feed.id;

        db.insert_feed(&feed).unwrap();

        let update = FeedUpdate {
            title: Some("Updated Title".to_string()),
            ..Default::default()
        };
        db.update_feed(feed_id, &update).unwrap();

        let retrieved = db.get_feed(feed_id).unwrap().unwrap();
        assert_eq!(retrieved.title, "Updated Title");
    }

    #[test]
    fn test_delete_feed() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Feed");
        let feed_id = feed.id;

        db.insert_feed(&feed).unwrap();
        assert!(db.get_feed(feed_id).unwrap().is_some());

        db.delete_feed(feed_id).unwrap();
        assert!(db.get_feed(feed_id).unwrap().is_none());
    }

    #[test]
    fn test_upsert_articles() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Feed");
        let feed_id = feed.id;
        db.insert_feed(&feed).unwrap();

        let articles = vec![
            Article::new(feed_id, "guid-1", "Article 1"),
            Article::new(feed_id, "guid-2", "Article 2"),
        ];

        let (new, updated) = db.upsert_articles(&articles).unwrap();
        assert_eq!(new, 2);
        assert_eq!(updated, 0);

        // Upsert again - should update
        let (new, updated) = db.upsert_articles(&articles).unwrap();
        assert_eq!(new, 0);
        assert_eq!(updated, 2);
    }

    #[test]
    fn test_get_articles_with_filter() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Feed");
        let feed_id = feed.id;
        db.insert_feed(&feed).unwrap();

        let mut articles = vec![
            Article::new(feed_id, "guid-1", "Article 1"),
            Article::new(feed_id, "guid-2", "Article 2"),
            Article::new(feed_id, "guid-3", "Article 3"),
        ];

        // Mark one as read
        articles[0].is_read = true;

        db.upsert_articles(&articles).unwrap();

        // Get all
        let page = db
            .get_articles(&ArticleFilter::default(), &Pagination::default())
            .unwrap();
        assert_eq!(page.total, 3);

        // Get unread only
        let filter = ArticleFilter {
            unread_only: true,
            ..Default::default()
        };
        let page = db.get_articles(&filter, &Pagination::default()).unwrap();
        assert_eq!(page.total, 2);
    }

    #[test]
    fn test_mark_read_unread() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Feed");
        let feed_id = feed.id;
        db.insert_feed(&feed).unwrap();

        let article = Article::new(feed_id, "guid-1", "Article 1");
        let article_id = article.id;
        db.upsert_articles(&[article]).unwrap();

        // Initially unread
        let retrieved = db.get_article(article_id).unwrap().unwrap();
        assert!(!retrieved.is_read);

        // Mark read
        db.mark_read(&[article_id]).unwrap();
        let retrieved = db.get_article(article_id).unwrap().unwrap();
        assert!(retrieved.is_read);
        assert!(retrieved.read_at.is_some());

        // Mark unread
        db.mark_unread(&[article_id]).unwrap();
        let retrieved = db.get_article(article_id).unwrap().unwrap();
        assert!(!retrieved.is_read);
        assert!(retrieved.read_at.is_none());
    }

    #[test]
    fn test_toggle_starred() {
        let db = setup_db();
        let feed = Feed::new("https://example.com/feed.xml", "Feed");
        let feed_id = feed.id;
        db.insert_feed(&feed).unwrap();

        let article = Article::new(feed_id, "guid-1", "Article 1");
        let article_id = article.id;
        db.upsert_articles(&[article]).unwrap();

        // Initially not starred
        let is_starred = db.toggle_starred(article_id).unwrap();
        assert!(is_starred);

        // Toggle again
        let is_starred = db.toggle_starred(article_id).unwrap();
        assert!(!is_starred);
    }

    #[test]
    fn test_insert_and_get_folder() {
        let db = setup_db();
        let folder = Folder::new("Tech");
        let folder_id = folder.id;

        db.insert_folder(&folder).unwrap();

        let retrieved = db.get_folder(folder_id).unwrap().unwrap();
        assert_eq!(retrieved.name, "Tech");
    }

    #[test]
    fn test_folder_with_feeds() {
        let db = setup_db();

        // Create folder
        let folder = Folder::new("Tech");
        let folder_id = folder.id;
        db.insert_folder(&folder).unwrap();

        // Create feeds in folder
        let mut feed1 = Feed::new("https://example.com/1.xml", "Feed 1");
        feed1.folder_id = Some(folder_id);
        let mut feed2 = Feed::new("https://example.com/2.xml", "Feed 2");
        feed2.folder_id = Some(folder_id);

        db.insert_feed(&feed1).unwrap();
        db.insert_feed(&feed2).unwrap();

        // Delete folder - feeds should have folder_id set to NULL
        db.delete_folder(folder_id).unwrap();

        let feeds = db.get_all_feeds().unwrap();
        assert_eq!(feeds.len(), 2);
        for feed in feeds {
            assert!(feed.folder_id.is_none());
        }
    }
}
