-- Migration 0013 — retention and routine maintenance (#137).
--
-- article_tombstones remembers the dedupe keys the article-retention
-- policy reaped from a feed that is still subscribed. Without it, a feed
-- that keeps listing a reaped item would bring it back on the next
-- refresh as a brand-new unread article. A refresh that meets a
-- tombstoned key skips it and stamps seen_at (at most once a day); the
-- maintenance pass forgets a tombstone once a later successful fetch of
-- its feed no longer listed the item, and a feed removal drops that
-- feed's tombstones with it.
--
-- dedupe_key is the stored, feed-scoped key ('f<feed_id>:…'); source_url
-- is the reaped row's, so a refresh settling a reused guid's key (which
-- compares a stored `guid:` row's source_url with the entry's link) finds
-- the tombstone exactly as it would have found the row; seen_at is
-- RFC 3339 UTC like every other timestamp column.
CREATE TABLE article_tombstones (
    dedupe_key TEXT    PRIMARY KEY,
    feed_id    INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    source_url TEXT    NOT NULL,
    seen_at    TEXT    NOT NULL
) WITHOUT ROWID;

CREATE INDEX idx_article_tombstones_feed_id ON article_tombstones(feed_id);

-- article_tags is keyed (article_id, tag_id), so "is this tag still
-- used?" — asked on every untag now that unused tags are dropped — would
-- otherwise scan the whole table.
CREATE INDEX idx_article_tags_tag_id ON article_tags(tag_id);

-- article_exports remembers every article Curio exported to a
-- destination, so retention never reaps it — whatever happens to the
-- destination afterwards (removed, re-rooted, unmounted). An export
-- records its article here before it writes the note, so a maintenance
-- pass running at the same moment either sees the record or finds the
-- article already gone (and the export then fails instead of writing a
-- note for it). The maintenance pass also records the exports the
-- registered destinations' manifests list, which picks up the exports
-- made before this table existed. Deleting the article drops its row.
CREATE TABLE article_exports (
    article_id  INTEGER PRIMARY KEY REFERENCES articles(id) ON DELETE CASCADE,
    recorded_at TEXT    NOT NULL
);
