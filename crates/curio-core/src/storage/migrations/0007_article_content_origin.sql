-- Migration 0007 — record where an article's stored content came from,
-- and stop the FTS update trigger from reindexing unchanged rows.
--
-- content_origin is the provenance of content_html/content_text/word_count:
--   * 'feed'      — the feed entry's own body (refresh ingest)
--   * 'manual'    — a feedless save (save-URL clip, import)
--   * 'full_text' — the readability extract of the source page (load-full,
--                   per-feed full-text mode)
--   * 'enriched'  — a source-specific provider's body (e.g. reddit JSON)
-- A refresh upsert only ever replaces 'feed'/'manual' content: hydrated
-- ('full_text'/'enriched') bodies survive every later refresh of a feed
-- still listing the item, which may update title and metadata only.
--
-- Backfill: nothing recorded provenance before, so it is inferred.
--   * manual-scope ('m:') rows → 'manual';
--   * rows of a feed in full-text mode → 'full_text': the newest batch may
--     still hold its readability extract (full-text mode only hydrates
--     fresh keys, so it would never re-hydrate them). A row already
--     reverted holds the excerpt either way; the only cost is that the
--     feed's own excerpt edits no longer reach it (load-full still does);
--   * every other row → 'feed'. A body loaded in full (load-full) on a
--     feed NOT in full-text mode is indistinguishable from the excerpt,
--     so the first refresh after the upgrade that still lists it reverts
--     it one last time; from then on provenance is recorded.
ALTER TABLE articles ADD COLUMN content_origin TEXT NOT NULL DEFAULT 'feed'
    CHECK (content_origin IN ('feed', 'manual', 'full_text', 'enriched'));

UPDATE articles SET content_origin = 'manual' WHERE substr(dedupe_key, 1, 2) = 'm:';

UPDATE articles SET content_origin = 'full_text'
WHERE substr(dedupe_key, 1, 2) <> 'm:'
  AND feed_id IN (SELECT id FROM feeds WHERE fetch_full_text = 1);

-- D4: FTS reindexes only on an actual title/content change. `UPDATE OF`
-- alone fires whenever a listed column is in the SET clause, even with an
-- identical value — a metadata-only update must not delete+reinsert the
-- row's FTS entry.
DROP TRIGGER articles_fts_au;

CREATE TRIGGER articles_fts_au AFTER UPDATE OF title, content_text ON articles
WHEN old.title IS NOT new.title OR old.content_text IS NOT new.content_text
BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
    INSERT INTO articles_fts(rowid, title, content_text)
    VALUES (new.id, new.title, new.content_text);
END;
