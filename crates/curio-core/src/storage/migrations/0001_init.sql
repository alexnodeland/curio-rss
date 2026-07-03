-- Migration 0001 — the v1 schema.
--
-- Design invariants (docs/design/decisions.md D2/D3/D4):
--   * articles get an INTEGER PRIMARY KEY rowid alias + a UUIDv7 unique
--     column: stable FTS5 external-content mapping (rowids survive VACUUM)
--     and insert locality at 100k rows.
--   * read/star/read-later/archive state lives in article_state, a
--     current-state projection OUTSIDE articles — flag flips never touch
--     the FTS-indexed table.
--   * every timestamp column stores one format: RFC 3339 UTC with
--     millisecond precision (lexicographic order == chronological order).
--   * the FTS5 index is external-content over articles(title, content_text)
--     kept in sync by triggers — the missing-triggers rowid-corruption bug
--     class is regression-tested.

CREATE TABLE feeds (
    id                    INTEGER PRIMARY KEY,
    url                   TEXT    NOT NULL UNIQUE,
    title                 TEXT,
    site_url              TEXT,
    description           TEXT,
    etag                  TEXT,
    last_modified         TEXT,
    status                TEXT    NOT NULL DEFAULT 'active'
                          CHECK (status IN ('active', 'paused', 'dead')),
    allow_private_network INTEGER NOT NULL DEFAULT 0,
    added_at              TEXT    NOT NULL,
    last_fetched_at       TEXT,
    modified_at           TEXT    NOT NULL
);

CREATE TABLE articles (
    id                INTEGER PRIMARY KEY,
    curio_id          TEXT    NOT NULL UNIQUE,
    feed_id           INTEGER REFERENCES feeds(id) ON DELETE SET NULL,
    dedupe_key        TEXT    NOT NULL UNIQUE,
    title             TEXT    NOT NULL DEFAULT '',
    source_url        TEXT    NOT NULL,
    author            TEXT,
    published_at      TEXT,
    content_html      TEXT    NOT NULL DEFAULT '',
    content_text      TEXT    NOT NULL DEFAULT '',
    lang              TEXT,
    word_count        INTEGER,
    saved_at          TEXT    NOT NULL,
    source_updated_at TEXT,
    modified_at       TEXT    NOT NULL
);

CREATE INDEX idx_articles_feed_id ON articles(feed_id);
CREATE INDEX idx_articles_published_at ON articles(published_at);

CREATE TABLE article_state (
    article_id    INTEGER PRIMARY KEY REFERENCES articles(id) ON DELETE CASCADE,
    is_read       INTEGER NOT NULL DEFAULT 0,
    is_starred    INTEGER NOT NULL DEFAULT 0,
    is_read_later INTEGER NOT NULL DEFAULT 0,
    is_archived   INTEGER NOT NULL DEFAULT 0,
    updated_at    TEXT    NOT NULL
);

CREATE TABLE tags (
    id   INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE
);

CREATE TABLE article_tags (
    article_id INTEGER NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
    tag_id     INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (article_id, tag_id)
) WITHOUT ROWID;

-- FTS5 external-content index over title + content_text. content_rowid is
-- the explicit INTEGER PRIMARY KEY, so the mapping survives VACUUM.
CREATE VIRTUAL TABLE articles_fts USING fts5(
    title,
    content_text,
    content='articles',
    content_rowid='id'
);

-- Sync triggers: the external-content table indexes nothing by itself; a
-- missing trigger silently desynchronizes the index (the sketch's bug).
-- The update trigger fires only on content changes, never on state flips
-- (state lives in article_state anyway).
CREATE TRIGGER articles_fts_ai AFTER INSERT ON articles BEGIN
    INSERT INTO articles_fts(rowid, title, content_text)
    VALUES (new.id, new.title, new.content_text);
END;

CREATE TRIGGER articles_fts_ad AFTER DELETE ON articles BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
END;

CREATE TRIGGER articles_fts_au AFTER UPDATE OF title, content_text ON articles BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text)
    VALUES ('delete', old.id, old.title, old.content_text);
    INSERT INTO articles_fts(rowid, title, content_text)
    VALUES (new.id, new.title, new.content_text);
END;

CREATE TABLE fetch_log (
    id           INTEGER PRIMARY KEY,
    feed_id      INTEGER NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    fetched_at   TEXT    NOT NULL,
    status       TEXT    NOT NULL
                 CHECK (status IN ('ok', 'not_modified', 'error')),
    http_status  INTEGER,
    error        TEXT,
    articles_new INTEGER NOT NULL DEFAULT 0,
    duration_ms  INTEGER
);

CREATE INDEX idx_fetch_log_feed_id ON fetch_log(feed_id, id);

CREATE TABLE settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
) WITHOUT ROWID;

-- Write-ahead event-emission intents (crash recovery for curio.events.v1).
-- A state-changing transaction commits its intent row atomically with the
-- state change; the emitter appends the envelope to the JSONL log, fsyncs,
-- then deletes the row. Rows still present at startup are replayed —
-- consumers dedupe by event_id, so the append-then-crash window is safe.
CREATE TABLE event_intents (
    id            INTEGER PRIMARY KEY,
    event_id      TEXT NOT NULL UNIQUE,
    ts            TEXT NOT NULL,
    envelope_json TEXT NOT NULL
);
