//! `EventLog` tests: the cross-process lock, torn-tail healing,
//! rotation hand-off between handles, and append rollback.

#![allow(clippy::unwrap_used, clippy::panic)]

use std::io::Write as _;
use std::sync::mpsc;

use curio_types::{CurioId, EventId, EventPayload, EventsSchemaV1Marker};

use super::*;
use crate::events::read_all;

const TORN: &[u8] = b"{\"schema\":\"curio.events.v1\",\"event_id\":\"in-fli";

fn opened(ts: &str) -> EventEnvelope {
    EventEnvelope {
        schema: EventsSchemaV1Marker,
        event_id: EventId::new(),
        ts: ts.parse().unwrap(),
        event: EventPayload::ArticleOpened {
            curio_id: CurioId::new(),
            dwell_ms: None,
        },
    }
}

/// The lock file the tests pair with an events dir: its sibling, as
/// in the profile layout.
fn lock_of(events: &Path) -> PathBuf {
    events.with_file_name("events.lock")
}

/// One whole line on disk in the day's file; returns its path.
fn one_whole_line(dir: &Path) -> PathBuf {
    let mut log = EventLog::open(dir, lock_of(dir)).unwrap();
    log.append(&opened("2026-07-03T10:00:00.000Z")).unwrap();
    log.flush().unwrap();
    dir.join("events-20260703.jsonl")
}

/// Appends `TORN` raw — what a live process looks like halfway
/// through an append.
fn start_an_append(path: &Path) {
    OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(TORN)
        .unwrap();
}

fn ends_torn(path: &Path) -> bool {
    std::fs::read(path).unwrap().ends_with(TORN)
}

#[test]
fn open_creates_exactly_the_events_dir_and_the_named_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let events = dir.path().join("events");
    let lock = dir.path().join("events.lock");
    let log = EventLog::open(&events, &lock).unwrap();
    assert_eq!(log.lock_path(), lock);
    assert!(lock.exists());
    assert_eq!(
        std::fs::read_dir(&events).unwrap().count(),
        0,
        "nothing but event files ever lands in the contract dir"
    );
    let mut created: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    created.sort();
    assert_eq!(created, ["events", "events.lock"]);
}

#[test]
fn a_lock_file_in_a_missing_directory_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = EventLog::open(
        dir.path().join("events"),
        dir.path().join("nope").join("x.lock"),
    )
    .unwrap_err();
    assert!(matches!(err, EventLogError::Io(_)), "{err:?}");
}

/// The events lock file deleted while a long-lived handle (the
/// desktop app) keeps it open, then a new handle (a CLI run) opened on
/// the re-created path: the two must still exclude each other, so the
/// old handle's heal never truncates the newcomer's append in flight.
#[cfg(unix)]
#[test]
fn a_deleted_events_lock_still_serializes_old_and_new_handles() {
    let dir = tempfile::tempdir().unwrap();
    let events = dir.path().join("events");
    let path = one_whole_line(&events);
    let mut long_lived =
        EventLog::open_with_lock_timeout(&events, lock_of(&events), Duration::from_millis(50))
            .unwrap();
    std::fs::remove_file(lock_of(&events)).unwrap();
    let mut newcomer = EventLog::open(&events, lock_of(&events)).unwrap();
    newcomer
        .locked(|_| {
            start_an_append(&path);
            let err = long_lived
                .locked(|_| Ok::<_, EventLogError>(()))
                .unwrap_err();
            assert!(
                matches!(err, EventLogError::ProfileLocked { .. }),
                "{err:?}"
            );
            assert!(ends_torn(&path), "no heal under the other holder's lock");
            Ok::<_, EventLogError>(())
        })
        .unwrap();
}

/// Acceptance (#114): `heal_torn_tail` never truncates while another
/// handle holds the lock — the "torn" bytes may be that handle's
/// append in flight. The opener waits; once the holder lets go (here
/// without finishing, as if it crashed) the heal runs.
#[test]
fn heal_waits_while_another_handle_holds_the_lock() {
    let dir = tempfile::tempdir().unwrap();
    let events = dir.path().join("events");
    let path = one_whole_line(&events);
    let mut holder = EventLog::open(&events, lock_of(&events)).unwrap();
    let (opened_tx, opened_rx) = mpsc::channel();
    let opener = holder
        .locked(|_| {
            start_an_append(&path);
            let events = events.clone();
            let opener = std::thread::spawn(move || {
                let log = EventLog::open_with_lock_timeout(
                    &events,
                    lock_of(&events),
                    Duration::from_secs(30),
                );
                opened_tx.send(()).unwrap();
                log.map(|_| ())
            });
            std::thread::sleep(Duration::from_millis(200));
            assert!(
                opened_rx.try_recv().is_err(),
                "the second open must wait for the lock"
            );
            assert!(ends_torn(&path), "no truncation while the lock is held");
            Ok::<_, EventLogError>(opener)
        })
        .unwrap();
    opener.join().unwrap().unwrap();
    assert!(!ends_torn(&path), "healed once the holder let go");
    assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
}

#[test]
fn a_lock_held_past_the_timeout_is_a_typed_error_and_touches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let events = dir.path().join("events");
    let path = one_whole_line(&events);
    let mut holder = EventLog::open(&events, lock_of(&events)).unwrap();
    holder
        .locked(|_| {
            start_an_append(&path);
            let err = EventLog::open_with_lock_timeout(
                &events,
                lock_of(&events),
                Duration::from_millis(50),
            )
            .unwrap_err();
            match &err {
                EventLogError::ProfileLocked {
                    path: lock,
                    waited_ms,
                } => {
                    assert_eq!(lock, &dir.path().join("events.lock"));
                    assert!(*waited_ms >= 50);
                }
                other => panic!("expected ProfileLocked, got {other:?}"),
            }
            assert!(err.to_string().contains("another Curio process"), "{err}");
            assert!(ends_torn(&path));
            Ok::<_, EventLogError>(())
        })
        .unwrap();
}

/// A handle that held a file open while another process rotated past
/// it adopts the newer file instead of growing the stale one.
#[test]
fn a_handle_adopts_the_file_another_handle_rotated_to() {
    let root = tempfile::tempdir().unwrap();
    let events = root.path().join("events");
    let dir = events.as_path();
    let first = opened("2026-07-03T10:00:00.000Z");
    let line = u64::try_from(serde_json::to_string(&first).unwrap().len() + 1).unwrap();
    // Two lines per file.
    let mut a = EventLog::open(dir, lock_of(dir))
        .unwrap()
        .with_max_file_bytes(line * 2);
    let mut b = EventLog::open(dir, lock_of(dir))
        .unwrap()
        .with_max_file_bytes(line * 2);
    let events: Vec<_> = (1..=4)
        .map(|s| opened(&format!("2026-07-03T10:00:0{s}.000Z")))
        .collect();
    a.append(&events[0]).unwrap();
    b.append(&events[1]).unwrap();
    b.append(&events[2]).unwrap(); // rotates to -2
    a.append(&events[3]).unwrap(); // must follow into -2, not overfill the base
    a.flush().unwrap();
    b.flush().unwrap();
    let base = std::fs::read_to_string(dir.join("events-20260703.jsonl")).unwrap();
    assert_eq!(base.lines().count(), 2, "the base file is never overfilled");
    assert_eq!(
        crate::events::read_all(dir).unwrap(),
        events,
        "file order is append order"
    );
}

/// Asserts the file is a sequence of whole lines — newline-terminated,
/// every line a parseable envelope — and returns them.
fn whole_lines(path: &Path) -> Vec<String> {
    let raw = std::fs::read_to_string(path).unwrap();
    assert!(raw.is_empty() || raw.ends_with('\n'), "torn tail:\n{raw}");
    raw.lines()
        .map(|line| {
            serde_json::from_str::<EventEnvelope>(line).unwrap();
            line.to_owned()
        })
        .collect()
}

/// #118: an append that dies mid-line (ENOSPC) must not leave torn
/// bytes behind for the in-process retry to extend.
#[test]
fn a_failed_append_is_rolled_back_and_the_retry_writes_one_whole_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events-20260703.jsonl");
    let mut log = EventLog::open(dir.path(), dir.path().join("events.lock")).unwrap();
    let first = opened("2026-07-03T10:00:00.000Z");
    log.append(&first).unwrap();
    log.flush().unwrap();

    let second = opened("2026-07-03T10:00:01.000Z");
    log.inject_short_write(0, 17);
    let err = log.append(&second).unwrap_err();
    assert!(
        matches!(&err, EventLogError::Io(io) if io.kind() == std::io::ErrorKind::StorageFull),
        "{err}"
    );
    // Nothing torn survives the failure itself...
    assert_eq!(whole_lines(&path).len(), 1);

    // ...and the retry appends exactly one whole copy.
    log.append(&second).unwrap();
    log.flush().unwrap();
    let lines = whole_lines(&path);
    assert_eq!(lines.len(), 2, "no duplicate, no partial: {lines:?}");
    assert_eq!(read_all(dir.path()).unwrap(), vec![first, second]);
}

/// If the rollback itself could not truncate + fsync, the lines that
/// landed before the failure are not known durable: `flush` must not
/// report success (the emitter would retire their intents) until the
/// file is reopened and fsynced again.
#[test]
fn flush_refuses_after_an_unsynced_rollback_until_the_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = EventLog::open(dir.path(), dir.path().join("events.lock")).unwrap();
    let first = opened("2026-07-03T10:00:00.000Z");
    log.append(&first).unwrap();
    // The state roll_back_failed_append leaves when set_len/sync_all
    // fail: file closed, durability unknown.
    log.current = None;
    log.unsynced_close = true;
    log.flush().unwrap_err();

    let second = opened("2026-07-03T10:00:01.000Z");
    log.append(&second).unwrap();
    log.flush().unwrap();
    assert_eq!(read_all(dir.path()).unwrap(), vec![first, second]);
}

/// A failure on the very first line of a fresh file rolls back to empty.
#[test]
fn a_failed_first_append_leaves_an_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("events-20260703.jsonl");
    let mut log = EventLog::open(dir.path(), dir.path().join("events.lock")).unwrap();
    let only = opened("2026-07-03T10:00:00.000Z");
    log.inject_short_write(0, 5);
    log.append(&only).unwrap_err();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);

    log.append(&only).unwrap();
    log.flush().unwrap();
    assert_eq!(whole_lines(&path).len(), 1);
    assert_eq!(read_all(dir.path()).unwrap(), vec![only]);
}
