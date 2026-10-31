-- Dead-letter table for staged event intents whose envelope no longer
-- decodes (a hand-edited row, a corrupt page, an envelope from an
-- incompatible build). Draining moves such a row here — kept verbatim for
-- diagnosis, with the decode error — instead of failing every open and
-- every emit on it.
--
-- The table has its own AUTOINCREMENT key: event_intents.id is a plain
-- rowid that SQLite reuses once the table empties (it does after every
-- drain), so keying on it would let a later quarantine overwrite an
-- earlier dead row. The original id is kept as an ordinary column.
CREATE TABLE event_intents_dead (
    dead_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    intent_id      INTEGER NOT NULL,
    event_id       TEXT NOT NULL,
    ts             TEXT NOT NULL,
    envelope_json  TEXT NOT NULL,
    error          TEXT NOT NULL,
    quarantined_at TEXT NOT NULL
);
