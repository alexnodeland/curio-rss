-- Per-feed full-text mode: when set, new articles from this feed have
-- their source pages fetched and readability-extracted at refresh time
-- (the Lire-style upgrade for content-free feeds). 0/1 boolean.
ALTER TABLE feeds ADD COLUMN fetch_full_text INTEGER NOT NULL DEFAULT 0;
