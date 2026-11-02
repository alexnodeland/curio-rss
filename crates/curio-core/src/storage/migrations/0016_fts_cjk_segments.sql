-- CJK substring search (#138). `unicode61` indexes a run of CJK text with
-- no spaces (`東京都の天気`) as one token, so nothing inside it could be
-- found. The index now stores every CJK run as its overlapping bigrams
-- plus its last character (`東京 京都 都の の天 天気 気`), written by the
-- Rust function `curio_fts_segment` (storage::fts, registered on every
-- connection Curio opens); non-CJK text passes through it unchanged.
--
-- The external content is a view over `articles` that applies the same
-- function, so FTS5's own `rebuild` and `integrity-check` see exactly the
-- text the triggers indexed — reading `articles` raw would desynchronize
-- the index on the next rebuild.

DROP TRIGGER articles_fts_ai;
DROP TRIGGER articles_fts_ad;
DROP TRIGGER articles_fts_au;
DROP TABLE articles_fts;

CREATE VIEW articles_fts_source AS
SELECT id,
       curio_fts_segment(title)        AS title,
       curio_fts_segment(content_text) AS content_text
FROM articles;

-- `prefix='1'`: a one-character CJK query is a prefix query (`"天"*`, see
-- storage::fts) that, without a prefix index, merges the doclist of every
-- bigram starting with that character — thousands of terms for a common
-- one — on every keystroke. The one-character prefix index makes it a
-- single doclist read: on a 20 000-article CJK corpus, `"一"*` drops from
-- ~650 ms to ~50 ms, for ~20% more index. (Prefix lengths count
-- characters, not bytes.)
CREATE VIRTUAL TABLE articles_fts USING fts5(
    title,
    content_text,
    content='articles_fts_source',
    content_rowid='id',
    prefix='1'
);

CREATE TRIGGER articles_fts_ai AFTER INSERT ON articles BEGIN
    INSERT INTO articles_fts(rowid, title, content_text)
    VALUES (new.id, curio_fts_segment(new.title), curio_fts_segment(new.content_text));
END;

CREATE TRIGGER articles_fts_ad AFTER DELETE ON articles BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, curio_fts_segment(old.title), curio_fts_segment(old.content_text));
END;

-- Reindexes only on an actual title/content change (migration 0007's D4).
CREATE TRIGGER articles_fts_au AFTER UPDATE OF title, content_text ON articles
WHEN old.title IS NOT new.title OR old.content_text IS NOT new.content_text
BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, curio_fts_segment(old.title), curio_fts_segment(old.content_text));
    INSERT INTO articles_fts(rowid, title, content_text)
    VALUES (new.id, curio_fts_segment(new.title), curio_fts_segment(new.content_text));
END;

INSERT INTO articles_fts(articles_fts) VALUES ('rebuild');
