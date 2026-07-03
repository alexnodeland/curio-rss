//! Integration tests for the `curio.events.v1` log: JSONL shape, rotation,
//! retention, fold semantics, and crash-recoverable emission.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::Path;

use curio_core::events::{EventEmitter, EventLog, FoldedState, read_all};
use curio_core::model::{ArticleContent, NewArticle};
use curio_core::storage::{ListArticles, Storage};
use curio_types::{CurioId, EventEnvelope, EventId, EventPayload, EventsSchemaV1Marker};

fn envelope_at(ts: &str, event: EventPayload) -> EventEnvelope {
    EventEnvelope {
        schema: EventsSchemaV1Marker,
        event_id: EventId::new(),
        ts: ts.parse().unwrap(),
        event,
    }
}

fn opened(ts: &str, id: CurioId) -> EventEnvelope {
    envelope_at(
        ts,
        EventPayload::ArticleOpened {
            curio_id: id,
            dwell_ms: None,
        },
    )
}

fn list_files(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

// ---------------------------------------------------------------- envelope

#[test]
fn lines_are_contract_shaped_json() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = EventLog::open(dir.path()).unwrap();
    let id = CurioId::new();
    log.append(&envelope_at(
        "2026-07-03T09:15:00.123Z",
        EventPayload::ArticleStarred {
            curio_id: id,
            tags: vec!["rust".to_owned()],
        },
    ))
    .unwrap();
    log.flush().unwrap();

    let raw = std::fs::read_to_string(dir.path().join("events-20260703.jsonl")).unwrap();
    let line = raw.lines().next().unwrap();
    let value: serde_json::Value = serde_json::from_str(line).unwrap();
    assert_eq!(value["schema"], "curio.events.v1");
    assert_eq!(value["type"], "article.starred");
    assert_eq!(value["ts"], "2026-07-03T09:15:00.123Z");
    assert_eq!(value["payload"]["curio_id"], id.to_string());
    assert_eq!(value["payload"]["tags"][0], "rust");
    assert_eq!(value["event_id"].as_str().unwrap().len(), 26);
}

// ---------------------------------------------------------------- rotation

#[test]
fn rotates_at_utc_midnight() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = EventLog::open(dir.path()).unwrap();
    let id = CurioId::new();
    log.append(&opened("2026-07-01T23:59:59.999Z", id)).unwrap();
    log.append(&opened("2026-07-02T00:00:00.000Z", id)).unwrap();
    log.append(&opened("2026-07-02T08:00:00.000Z", id)).unwrap();
    log.flush().unwrap();

    assert_eq!(
        list_files(dir.path()),
        vec!["events-20260701.jsonl", "events-20260702.jsonl"]
    );
    let day2 = std::fs::read_to_string(dir.path().join("events-20260702.jsonl")).unwrap();
    assert_eq!(day2.lines().count(), 2);
}

#[test]
fn rotates_with_numeric_suffixes_at_the_size_cap() {
    let dir = tempfile::tempdir().unwrap();
    // Cap of 1 byte: every line overflows → one file per event.
    let mut log = EventLog::open(dir.path()).unwrap().with_max_file_bytes(1);
    let id = CurioId::new();
    let events: Vec<EventEnvelope> = (0..12)
        .map(|_| opened("2026-07-03T10:00:00.000Z", id))
        .collect();
    for event in &events {
        log.append(event).unwrap();
    }
    log.flush().unwrap();

    let files = list_files(dir.path());
    assert_eq!(files.len(), 12);
    assert!(files.contains(&"events-20260703.jsonl".to_owned()));
    assert!(files.contains(&"events-20260703-2.jsonl".to_owned()));
    assert!(files.contains(&"events-20260703-12.jsonl".to_owned()));

    // read_all orders suffixes numerically (…-2 before …-10), so the
    // stream comes back in exact append order.
    let read = read_all(dir.path()).unwrap();
    assert_eq!(read, events);
}

#[test]
fn append_resumes_the_newest_file_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let id = CurioId::new();
    let first = opened("2026-07-03T10:00:00.000Z", id);
    {
        let mut log = EventLog::open(dir.path()).unwrap();
        log.append(&first).unwrap();
        log.flush().unwrap();
    }
    let second = opened("2026-07-03T11:00:00.000Z", id);
    {
        let mut log = EventLog::open(dir.path()).unwrap();
        log.append(&second).unwrap();
        log.flush().unwrap();
    }
    assert_eq!(list_files(dir.path()), vec!["events-20260703.jsonl"]);
    assert_eq!(read_all(dir.path()).unwrap(), vec![first, second]);
}

/// Crash window C: the process died (or the disk filled) mid-append,
/// leaving a torn partial line at EOF. The line's fsync never ran, so its
/// intent survives and is replayed — but without healing, the replay
/// would be glued onto the torn bytes and the file would be unreadable
/// forever. Opening the log truncates the torn tail back to the last
/// complete line.
#[test]
fn a_torn_final_line_is_healed_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let id = CurioId::new();
    let whole = opened("2026-07-03T10:00:00.000Z", id);
    {
        let mut log = EventLog::open(dir.path()).unwrap();
        log.append(&whole).unwrap();
        log.flush().unwrap();
    }
    // Simulate the crash: a partial line, no trailing newline.
    let path = dir.path().join("events-20260703.jsonl");
    {
        use std::io::Write as _;
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"schema\":\"curio.events.v1\",\"event_id\":\"tor")
            .unwrap();
    }

    // Reopen and append (the replayed envelope in real life).
    let replayed = opened("2026-07-03T10:00:01.000Z", id);
    let mut log = EventLog::open(dir.path()).unwrap();
    log.append(&replayed).unwrap();
    log.flush().unwrap();

    let raw = std::fs::read_to_string(&path).unwrap();
    assert_eq!(raw.lines().count(), 2, "torn bytes must be gone:\n{raw}");
    assert_eq!(read_all(dir.path()).unwrap(), vec![whole, replayed]);
}

/// A file that is nothing but one torn line heals to empty — and stays
/// readable without any append happening first (`curio events` right
/// after a crash).
#[test]
fn a_wholly_torn_file_heals_to_empty_at_open() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events-20260703.jsonl");
    std::fs::write(&path, b"{\"schema\":\"curio.eve").unwrap();

    let _log = EventLog::open(dir.path()).unwrap();
    assert_eq!(read_all(dir.path()).unwrap(), vec![]);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
}

// --------------------------------------------------------------- retention

#[test]
fn retention_sweep_removes_only_files_past_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = EventLog::open(dir.path()).unwrap();
    let id = CurioId::new();
    log.append(&opened("2026-01-01T12:00:00.000Z", id)).unwrap();
    log.append(&opened("2026-04-01T12:00:00.000Z", id)).unwrap();
    log.append(&opened("2026-07-01T12:00:00.000Z", id)).unwrap();
    log.flush().unwrap();
    // A foreign file must never be touched.
    std::fs::write(dir.path().join("cursor.json"), "{}").unwrap();

    let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 3).unwrap();
    let removed = log.sweep_retention(today).unwrap();
    // 2026-01-01 is 183 days old (> 90); 2026-04-01 is 93 days old (> 90);
    // 2026-07-01 is 2 days old (kept).
    assert_eq!(removed.len(), 2);
    assert_eq!(
        list_files(dir.path()),
        vec!["cursor.json", "events-20260701.jsonl"]
    );
}

#[test]
fn retention_cannot_be_configured_below_the_contract_floor() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = EventLog::open(dir.path()).unwrap().with_retention_days(7); // clamped to 90
    let id = CurioId::new();
    log.append(&opened("2026-06-01T12:00:00.000Z", id)).unwrap();
    log.flush().unwrap();
    let today = chrono::NaiveDate::from_ymd_opt(2026, 7, 3).unwrap();
    // 32 days old — must survive a "7 day" (clamped to 90) sweep. It is
    // also the currently open file, so re-check with a fresh handle.
    assert!(log.sweep_retention(today).unwrap().is_empty());
    let mut fresh = EventLog::open(dir.path()).unwrap().with_retention_days(7);
    assert!(fresh.sweep_retention(today).unwrap().is_empty());
    assert_eq!(list_files(dir.path()), vec!["events-20260601.jsonl"]);
}

// -------------------------------------------------------------------- fold

#[test]
fn fold_honors_negation_events() {
    let id_a = CurioId::new();
    let id_b = CurioId::new();
    let ts = "2026-07-03T10:00:00.000Z";
    let events = vec![
        envelope_at(
            ts,
            EventPayload::ArticleStarred {
                curio_id: id_a,
                tags: vec![],
            },
        ),
        envelope_at(
            ts,
            EventPayload::ArticleStarred {
                curio_id: id_b,
                tags: vec![],
            },
        ),
        envelope_at(ts, EventPayload::ArticleUnstarred { curio_id: id_a }),
        envelope_at(
            ts,
            EventPayload::ReadLaterAdded {
                curio_id: id_a,
                tags: vec![],
            },
        ),
        envelope_at(ts, EventPayload::ReadLaterRemoved { curio_id: id_a }),
        envelope_at(ts, EventPayload::ArticleArchived { curio_id: id_b }),
        envelope_at(ts, EventPayload::ArticleUnarchived { curio_id: id_b }),
        envelope_at(
            ts,
            EventPayload::ArticleTagged {
                curio_id: id_a,
                tag: "x".to_owned(),
            },
        ),
        envelope_at(
            ts,
            EventPayload::ArticleTagged {
                curio_id: id_a,
                tag: "y".to_owned(),
            },
        ),
        envelope_at(
            ts,
            EventPayload::ArticleUntagged {
                curio_id: id_a,
                tag: "x".to_owned(),
            },
        ),
        envelope_at(
            ts,
            EventPayload::FeedAdded {
                feed: "https://example.com/f.xml".to_owned(),
                feed_title: None,
                tags: vec![],
            },
        ),
        envelope_at(
            ts,
            EventPayload::FeedRemoved {
                feed: "https://example.com/f.xml".to_owned(),
            },
        ),
    ];
    let state = FoldedState::fold(events);
    assert!(
        !state.starred.contains(&id_a),
        "negation must remove membership"
    );
    assert!(state.starred.contains(&id_b));
    assert!(!state.read_later.contains(&id_a));
    assert!(!state.archived.contains(&id_b));
    assert_eq!(
        state
            .tags
            .get(&id_a)
            .map(|t| t.iter().cloned().collect::<Vec<_>>()),
        Some(vec!["y".to_owned()])
    );
    assert!(state.feeds.is_empty());
}

#[test]
fn fold_dedupes_by_event_id() {
    let id = CurioId::new();
    let star = envelope_at(
        "2026-07-03T10:00:00.000Z",
        EventPayload::ArticleStarred {
            curio_id: id,
            tags: vec![],
        },
    );
    let unstar = envelope_at(
        "2026-07-03T10:00:01.000Z",
        EventPayload::ArticleUnstarred { curio_id: id },
    );
    // The same starred event replayed AFTER its negation (a consumer
    // re-reading from an older cursor) must not resurrect membership.
    let state = FoldedState::fold(vec![star.clone(), unstar, star.clone()]);
    assert!(!state.starred.contains(&id));
    assert_eq!(state.events_seen(), 2);

    let mut counting = FoldedState::new();
    assert!(counting.apply(&star));
    assert!(!counting.apply(&star), "duplicate must report unapplied");
    assert_eq!(counting.opened.len(), 0);
}

// -------------------------------------------------- transactional emission

fn seed_article(storage: &Storage) -> curio_core::model::ArticleId {
    storage
        .upsert_articles(vec![NewArticle {
            feed_id: None,
            dedupe_key: "k1".to_owned(),
            title: "Title".to_owned(),
            source_url: "https://example.com/a".to_owned(),
            author: None,
            published_at: None,
            content: ArticleContent {
                html: "<p>body</p>".to_owned(),
                text: "body".to_owned(),
            },
            lang: None,
            word_count: None,
            source_updated_at: None,
        }])
        .unwrap();
    storage.list_articles(ListArticles::default()).unwrap()[0].id
}

#[test]
fn emitter_drains_intents_into_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let storage = std::sync::Arc::new(Storage::open(dir.path().join("curio.db")).unwrap());
    let events_dir = dir.path().join("events");
    let id = seed_article(&storage);

    let mut emitter = EventEmitter::new(
        std::sync::Arc::clone(&storage),
        EventLog::open(&events_dir).unwrap(),
    )
    .unwrap();

    let star = storage.star_article(id).unwrap().unwrap();
    let tag = storage.tag_article(id, "rust").unwrap().unwrap();
    assert_eq!(emitter.emit_pending().unwrap(), 2);
    assert_eq!(emitter.emit_pending().unwrap(), 0, "drain is idempotent");
    assert!(storage.pending_intents().unwrap().is_empty());

    let read = read_all(&events_dir).unwrap();
    assert_eq!(read, vec![star, tag]);
}

/// Crash window A: the state change committed (intent staged) but the
/// process died before the JSONL append. Startup replay emits it.
#[test]
fn startup_replays_intents_that_never_reached_the_log() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("curio.db");
    let events_dir = dir.path().join("events");

    let (article_curio_id, starred_event_id) = {
        let storage = Storage::open(&db).unwrap();
        let id = seed_article(&storage);
        let starred = storage.star_article(id).unwrap().unwrap();
        // "Crash": no emitter ever runs; the intent stays staged.
        assert_eq!(storage.pending_intents().unwrap().len(), 1);
        (
            storage.get_article(id).unwrap().unwrap().curio_id,
            starred.event_id,
        )
    };

    // Next process start: constructing the emitter replays.
    let storage = std::sync::Arc::new(Storage::open(&db).unwrap());
    let _emitter = EventEmitter::new(
        std::sync::Arc::clone(&storage),
        EventLog::open(&events_dir).unwrap(),
    )
    .unwrap();
    assert!(storage.pending_intents().unwrap().is_empty());

    let events = read_all(&events_dir).unwrap();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_id, starred_event_id);
    let state = FoldedState::fold(events);
    assert!(state.starred.contains(&article_curio_id));
}

/// Crash window B: the append + fsync happened but the process died before
/// the intent was deleted. Replay appends a duplicate line — and the
/// consumer-side `event_id` dedupe makes that harmless.
#[test]
fn replay_after_partial_emission_duplicates_harmlessly() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("curio.db");
    let events_dir = dir.path().join("events");

    let storage = std::sync::Arc::new(Storage::open(&db).unwrap());
    let id = seed_article(&storage);
    storage.star_article(id).unwrap().unwrap();
    let curio_id = storage.get_article(id).unwrap().unwrap().curio_id;

    // Simulate the partial emission: append + fsync, but never mark.
    {
        let pending = storage.pending_intents().unwrap();
        let mut log = EventLog::open(&events_dir).unwrap();
        for intent in &pending {
            log.append(&intent.envelope).unwrap();
        }
        log.flush().unwrap();
        // "Crash" before mark_intents_emitted.
    }
    assert_eq!(storage.pending_intents().unwrap().len(), 1);

    // Restart: the replay appends the same envelope again.
    let _emitter = EventEmitter::new(
        std::sync::Arc::clone(&storage),
        EventLog::open(&events_dir).unwrap(),
    )
    .unwrap();
    let events = read_all(&events_dir).unwrap();
    assert_eq!(events.len(), 2, "the duplicate line is expected");
    assert_eq!(events[0].event_id, events[1].event_id);

    let state = FoldedState::fold(events);
    assert_eq!(state.events_seen(), 1, "dedupe collapses the duplicate");
    assert!(state.starred.contains(&curio_id));
}
