-- Published-date list order (#136): the article list can page by
-- publication date, latest first, an undated article standing at the
-- time Curio first saved it.
--
-- The index key is that date expression, spelled exactly as the list
-- query spells it (storage/repo/listing/mod.rs, DATED) — SQLite serves an
-- ORDER BY from an expression index only when the expressions match.
-- Every index entry ends in the rowid, so the key is (date, id): the
-- list's tie-break comes free, and a page is one walk down the index,
-- seeked to the keyset cursor, with no sort.
CREATE INDEX idx_articles_dated ON articles(COALESCE(published_at, saved_at));

-- The membership views (Starred, Read later, Archived) page through
-- article_state, not articles, and an index cannot reach across tables.
-- So each state row carries a copy of its article's list date, `dated`,
-- and each membership gets a second partial index keyed on it: a
-- published-order page of a view walks that index — (dated, article_id),
-- the rowid again the tie-break — and reads only its own members, however
-- dense the view is, with no sort.
--
-- The triggers below keep the copy equal to COALESCE(published_at,
-- saved_at) of the row's article after every write, whichever code path
-- makes it: a new state row picks the date up from its article (the
-- writers never name the column), and an article whose date changes
-- (a feed re-dating a post) carries the change to its state row.
ALTER TABLE article_state ADD COLUMN dated TEXT;

UPDATE article_state
   SET dated = (SELECT COALESCE(a.published_at, a.saved_at) FROM articles a
                 WHERE a.id = article_state.article_id);

CREATE TRIGGER article_state_dated_ai AFTER INSERT ON article_state
WHEN new.dated IS NULL
BEGIN
    UPDATE article_state
       SET dated = (SELECT COALESCE(a.published_at, a.saved_at) FROM articles a
                     WHERE a.id = new.article_id)
     WHERE article_id = new.article_id;
END;

CREATE TRIGGER article_state_dated_au AFTER UPDATE OF published_at, saved_at ON articles
WHEN COALESCE(old.published_at, old.saved_at) IS NOT COALESCE(new.published_at, new.saved_at)
BEGIN
    UPDATE article_state
       SET dated = COALESCE(new.published_at, new.saved_at)
     WHERE article_id = new.id;
END;

CREATE INDEX idx_article_state_starred_dated ON article_state(dated) WHERE is_starred = 1;
CREATE INDEX idx_article_state_read_later_dated ON article_state(dated) WHERE is_read_later = 1;
CREATE INDEX idx_article_state_archived_dated ON article_state(dated) WHERE is_archived = 1;
