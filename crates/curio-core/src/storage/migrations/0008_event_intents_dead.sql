-- Dead-letter table for staged event intents whose envelope no longer
-- decodes (a hand-edited row, a corrupt page, an envelope from an
-- incompatible build). Draining moves such a row here — kept verbatim for
-- diagnosis, with the decode error — instead of failing every open and
-- every emit on it. The id is the original event_intents.id.
CREATE TABLE event_intents_dead (
    id             INTEGER PRIMARY KEY,
    event_id       TEXT NOT NULL,
    ts             TEXT NOT NULL,
    envelope_json  TEXT NOT NULL,
    error          TEXT NOT NULL,
    quarantined_at TEXT NOT NULL
);
