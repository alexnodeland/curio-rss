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
-- Existing rows default to 'feed' (manual-scope rows to 'manual'): nothing
-- recorded provenance before, and a body already reverted to the excerpt
-- is indistinguishable from one never hydrated.
ALTER TABLE articles ADD COLUMN content_origin TEXT NOT NULL DEFAULT 'feed'
    CHECK (content_origin IN ('feed', 'manual', 'full_text', 'enriched'));

UPDATE articles SET content_origin = 'manual' WHERE substr(dedupe_key, 1, 2) = 'm:';

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
