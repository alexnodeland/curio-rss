-- Read paths for the article list (#136): per-feed unread counters kept by
-- triggers, and partial indexes for the sparse membership views.
--
-- feed_unread holds, per feed, how many of its articles are unread — not
-- read and not archived; an article with no article_state row is unread,
-- exactly as before. feed_key is the feed's id, or 0 for the articles
-- that belong to no feed (manual saves, rows a feed removal orphaned):
-- feed ids start at 1, and a PRIMARY KEY cannot hold NULL as one key.
-- The triggers below keep it equal to the scan it replaces after every
-- write, whichever code path makes it; a row may sit at 0, and readers
-- skip those.
--
-- Two orderings are relied on (both are SQLite's documented behaviour
-- for foreign-key actions, and the storage tests pin them): deleting an
-- article cascades its article_state row away BEFORE the article's AFTER
-- DELETE triggers run, so the article side counts in a BEFORE DELETE
-- trigger, while the state side skips any state row whose article is
-- already gone; and a feed removal's ON DELETE SET NULL runs as an UPDATE
-- of articles.feed_id, which moves the counts to key 0.
CREATE TABLE feed_unread (
    feed_key INTEGER PRIMARY KEY,
    unread   INTEGER NOT NULL
);

INSERT INTO feed_unread (feed_key, unread)
SELECT COALESCE(a.feed_id, 0), COUNT(*)
  FROM articles a
  LEFT JOIN article_state s ON s.article_id = a.id
 WHERE COALESCE(s.is_read, 0) = 0 AND COALESCE(s.is_archived, 0) = 0
 GROUP BY COALESCE(a.feed_id, 0);

-- An article arrives unread (it has no state row yet).
CREATE TRIGGER feed_unread_article_ai AFTER INSERT ON articles
WHEN NOT EXISTS (SELECT 1 FROM article_state s WHERE s.article_id = new.id
                  AND (s.is_read <> 0 OR s.is_archived <> 0))
BEGIN
    INSERT INTO feed_unread (feed_key, unread)
    SELECT COALESCE(new.feed_id, 0), 1 WHERE 1
    ON CONFLICT (feed_key) DO UPDATE SET unread = unread + excluded.unread;
END;

-- BEFORE, so the state row the cascade is about to remove is still there.
CREATE TRIGGER feed_unread_article_bd BEFORE DELETE ON articles
WHEN NOT EXISTS (SELECT 1 FROM article_state s WHERE s.article_id = old.id
                  AND (s.is_read <> 0 OR s.is_archived <> 0))
BEGIN
    UPDATE feed_unread SET unread = unread - 1
     WHERE feed_key = COALESCE(old.feed_id, 0);
END;

-- An unread article that changes feed (a feed removal's SET NULL) moves.
CREATE TRIGGER feed_unread_article_au AFTER UPDATE OF feed_id ON articles
WHEN old.feed_id IS NOT new.feed_id
 AND NOT EXISTS (SELECT 1 FROM article_state s WHERE s.article_id = new.id
                  AND (s.is_read <> 0 OR s.is_archived <> 0))
BEGIN
    UPDATE feed_unread SET unread = unread - 1
     WHERE feed_key = COALESCE(old.feed_id, 0);
    INSERT INTO feed_unread (feed_key, unread)
    SELECT COALESCE(new.feed_id, 0), 1 WHERE 1
    ON CONFLICT (feed_key) DO UPDATE SET unread = unread + excluded.unread;
END;

-- A first state row that reads or archives the article.
CREATE TRIGGER feed_unread_state_ai AFTER INSERT ON article_state
WHEN new.is_read <> 0 OR new.is_archived <> 0
BEGIN
    UPDATE feed_unread SET unread = unread - 1
     WHERE feed_key = (SELECT COALESCE(a.feed_id, 0) FROM articles a
                        WHERE a.id = new.article_id);
END;

-- A flip that crosses the unread line, either way.
CREATE TRIGGER feed_unread_state_au AFTER UPDATE OF is_read, is_archived ON article_state
WHEN (old.is_read <> 0 OR old.is_archived <> 0)
     IS NOT (new.is_read <> 0 OR new.is_archived <> 0)
BEGIN
    UPDATE feed_unread SET unread = unread - 1
     WHERE (new.is_read <> 0 OR new.is_archived <> 0)
       AND feed_key = (SELECT COALESCE(a.feed_id, 0) FROM articles a
                        WHERE a.id = new.article_id);
    INSERT INTO feed_unread (feed_key, unread)
    SELECT COALESCE(a.feed_id, 0), 1 FROM articles a
     WHERE a.id = new.article_id AND new.is_read = 0 AND new.is_archived = 0
    ON CONFLICT (feed_key) DO UPDATE SET unread = unread + excluded.unread;
END;

-- A read/archived state row removed while its article stays (never the
-- cascade of an article delete: that article is already gone here).
CREATE TRIGGER feed_unread_state_ad AFTER DELETE ON article_state
WHEN old.is_read <> 0 OR old.is_archived <> 0
BEGIN
    INSERT INTO feed_unread (feed_key, unread)
    SELECT COALESCE(a.feed_id, 0), 1 FROM articles a WHERE a.id = old.article_id
    ON CONFLICT (feed_key) DO UPDATE SET unread = unread + excluded.unread;
END;

-- A removed feed's counter goes with it (its articles moved to key 0).
CREATE TRIGGER feed_unread_feed_ad AFTER DELETE ON feeds
BEGIN
    DELETE FROM feed_unread WHERE feed_key = old.id;
END;

-- The sparse membership views (Starred, Read later, Archived) page by
-- walking these newest-first instead of probing every article's state.
CREATE INDEX idx_article_state_starred ON article_state(article_id) WHERE is_starred = 1;
CREATE INDEX idx_article_state_read_later ON article_state(article_id) WHERE is_read_later = 1;
CREATE INDEX idx_article_state_archived ON article_state(article_id) WHERE is_archived = 1;
