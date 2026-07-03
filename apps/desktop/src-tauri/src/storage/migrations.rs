//! Database migrations for SQLite.

use rusqlite::Connection;

/// Current schema version
pub const SCHEMA_VERSION: i32 = 1;

/// Run all pending migrations
pub fn run_migrations(conn: &Connection) -> rusqlite::Result<()> {
    let current_version = get_schema_version(conn)?;

    if current_version < 1 {
        migrate_v1(conn)?;
    }

    Ok(())
}

/// Get current schema version
fn get_schema_version(conn: &Connection) -> rusqlite::Result<i32> {
    // Check if schema_version table exists
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='schema_version')",
        [],
        |row| row.get(0),
    )?;

    if !exists {
        return Ok(0);
    }

    conn.query_row("SELECT version FROM schema_version", [], |row| row.get(0))
}

/// Set schema version
fn set_schema_version(conn: &Connection, version: i32) -> rusqlite::Result<()> {
    conn.execute("DELETE FROM schema_version", [])?;
    conn.execute("INSERT INTO schema_version (version) VALUES (?)", [version])?;
    Ok(())
}

/// Migration to version 1: Initial schema
fn migrate_v1(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        r#"
        -- Schema version tracking
        CREATE TABLE IF NOT EXISTS schema_version (
            version INTEGER NOT NULL
        );

        -- Folders for organizing feeds
        CREATE TABLE IF NOT EXISTS folders (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            parent_id TEXT REFERENCES folders(id) ON DELETE CASCADE,
            icon TEXT,
            color TEXT,
            view_mode TEXT,
            position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- RSS/Atom feeds
        CREATE TABLE IF NOT EXISTS feeds (
            id TEXT PRIMARY KEY,
            url TEXT NOT NULL UNIQUE,
            title TEXT NOT NULL,
            description TEXT,
            site_url TEXT,
            icon_url TEXT,
            view_mode TEXT NOT NULL DEFAULT 'article',
            tags TEXT,
            folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
            etag TEXT,
            last_modified TEXT,
            last_fetched TEXT,
            last_error TEXT,
            refresh_interval INTEGER,
            notify_new INTEGER NOT NULL DEFAULT 0,
            position INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        -- Articles/items from feeds
        CREATE TABLE IF NOT EXISTS articles (
            id TEXT PRIMARY KEY,
            feed_id TEXT NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
            guid TEXT NOT NULL,
            title TEXT NOT NULL,
            url TEXT,
            content_html TEXT,
            content_text TEXT,
            summary TEXT,
            author TEXT,
            thumbnail_url TEXT,
            published_at TEXT,
            updated_at TEXT,
            fetched_at TEXT NOT NULL DEFAULT (datetime('now')),
            is_read INTEGER NOT NULL DEFAULT 0,
            is_starred INTEGER NOT NULL DEFAULT 0,
            is_read_later INTEGER NOT NULL DEFAULT 0,
            is_archived INTEGER NOT NULL DEFAULT 0,
            read_at TEXT,
            read_later_at TEXT,
            read_later_position INTEGER,
            reddit_score INTEGER,
            reddit_num_comments INTEGER,
            youtube_duration INTEGER,
            podcast_duration INTEGER,
            podcast_progress INTEGER,
            UNIQUE(feed_id, guid)
        );

        -- Media attachments
        CREATE TABLE IF NOT EXISTS media_attachments (
            id TEXT PRIMARY KEY,
            article_id TEXT NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
            url TEXT NOT NULL,
            mime_type TEXT NOT NULL,
            size_bytes INTEGER,
            title TEXT,
            is_downloaded INTEGER NOT NULL DEFAULT 0,
            local_path TEXT
        );

        -- Full-text search
        CREATE VIRTUAL TABLE IF NOT EXISTS articles_fts USING fts5(
            title,
            content_text,
            author,
            content='articles',
            content_rowid='rowid'
        );

        -- Triggers for FTS sync
        CREATE TRIGGER IF NOT EXISTS articles_ai AFTER INSERT ON articles BEGIN
            INSERT INTO articles_fts(rowid, title, content_text, author)
            VALUES (NEW.rowid, NEW.title, NEW.content_text, NEW.author);
        END;

        CREATE TRIGGER IF NOT EXISTS articles_ad AFTER DELETE ON articles BEGIN
            INSERT INTO articles_fts(articles_fts, rowid, title, content_text, author)
            VALUES ('delete', OLD.rowid, OLD.title, OLD.content_text, OLD.author);
        END;

        CREATE TRIGGER IF NOT EXISTS articles_au AFTER UPDATE ON articles BEGIN
            INSERT INTO articles_fts(articles_fts, rowid, title, content_text, author)
            VALUES ('delete', OLD.rowid, OLD.title, OLD.content_text, OLD.author);
            INSERT INTO articles_fts(rowid, title, content_text, author)
            VALUES (NEW.rowid, NEW.title, NEW.content_text, NEW.author);
        END;

        -- Indexes for common queries
        CREATE INDEX IF NOT EXISTS idx_articles_feed_id ON articles(feed_id);
        CREATE INDEX IF NOT EXISTS idx_articles_published_at ON articles(published_at DESC);
        CREATE INDEX IF NOT EXISTS idx_articles_is_read ON articles(is_read);
        CREATE INDEX IF NOT EXISTS idx_articles_is_starred ON articles(is_starred);
        CREATE INDEX IF NOT EXISTS idx_articles_is_read_later ON articles(is_read_later, read_later_position);
        CREATE INDEX IF NOT EXISTS idx_feeds_folder_id ON feeds(folder_id);
        CREATE INDEX IF NOT EXISTS idx_folders_parent_id ON folders(parent_id);
        CREATE INDEX IF NOT EXISTS idx_media_article_id ON media_attachments(article_id);
        "#,
    )?;

    set_schema_version(conn, 1)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    #[test]
    fn test_migrations_from_scratch() {
        let conn = Connection::open_in_memory().unwrap();

        run_migrations(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"folders".to_string()));
        assert!(tables.contains(&"feeds".to_string()));
        assert!(tables.contains(&"articles".to_string()));
        assert!(tables.contains(&"media_attachments".to_string()));
        assert!(tables.contains(&"schema_version".to_string()));
    }

    #[test]
    fn test_migrations_idempotent() {
        let conn = Connection::open_in_memory().unwrap();

        // Run twice - should not error
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, SCHEMA_VERSION);
    }

    #[test]
    fn test_schema_version() {
        let conn = Connection::open_in_memory().unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, 0);

        run_migrations(&conn).unwrap();

        let version = get_schema_version(&conn).unwrap();
        assert_eq!(version, 1);
    }
}
