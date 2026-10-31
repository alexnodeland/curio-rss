//! The storage-coupled emitter: intents → JSONL, crash-recoverably.
//!
//! Ordering contract (the "transactional-ish" pair):
//!
//! 1. the DB state change commits first, *with* its write-ahead intent row
//!    in the same transaction (the storage repo does this);
//! 2. the emitter takes the cross-process events lock (see [`EventLog`]);
//! 3. under it, the emitter claims the staged intents, appends their
//!    envelopes to the JSONL log, and fsyncs the log;
//! 4. only then — still under the lock — are the intent rows deleted.
//!
//! Crash between 1 and 4 → the intent survives and is replayed by
//! [`EventEmitter::new`] at the next startup. Crash between 3 and 4 → the
//! replay appends a duplicate line, which consumers drop by `event_id`
//! dedupe. No window loses an event.
//!
//! A failure mid-batch (a full disk on the third of five appends) is the
//! same story in-process: the log rolls the torn line back (see
//! [`EventLog::append`]), the lines already appended are fsynced and their
//! intents deleted, and the rest stay staged for the next drain — so a
//! retry neither duplicates the prefix nor extends a partial line.
//!
//! Power loss is covered by step 1 being *durable*: the writer connection
//! runs `synchronous=FULL`, so a commit that returned is on disk before
//! the emitter can fsync its event line. (Under WAL + `NORMAL` a returned
//! commit could still roll back on power loss, leaving a JSONL line —
//! say `article.starred` — whose state change the database forgot.)
//!
//! Claim + append + fsync + delete is one critical section, so the claim
//! is atomic across processes: when the desktop app and a CLI run share a
//! profile, an intent one of them staged is drained by one of them at a
//! time — whichever takes the lock first sees it, and the other sees it
//! already deleted. (Deleting at claim time instead, `DELETE … RETURNING`
//! style, would open a crash window that *loses* the event.)
//!
//! Delivery is still **at-least-once**, never exactly-once: the delete in
//! step 4 goes through this process's writer and can fail after step 3
//! made the lines durable — a crash, or [`StorageError::Busy`] when
//! another process holds a database write past the busy timeout. The
//! intents then stay staged and the next drain (in either process)
//! appends them again; consumers drop the duplicate by `event_id`, as the
//! contract requires. The lock is also held while that delete waits
//! behind this process's queued write jobs, so a long one (a vacuum, a
//! big import) delays other processes' emits.
//!
//! [`StorageError::Busy`]: crate::storage::StorageError::Busy

use std::sync::Arc;

use super::log::LockedLog;
use super::{EventLog, EventsError};
use crate::storage::{PendingIntent, Storage};

/// Drains staged event intents from [`Storage`] into an [`EventLog`].
#[derive(Debug)]
pub struct EventEmitter {
    storage: Arc<Storage>,
    log: EventLog,
}

impl EventEmitter {
    /// Couples a storage handle to a log and immediately replays any
    /// intents staged but never emitted — by a previous process that
    /// crashed, or by a live one about to drain them itself (the lock
    /// makes sure the two never drain the same intents at once).
    ///
    /// # Errors
    ///
    /// Storage or log errors during the recovery replay.
    pub fn new(storage: Arc<Storage>, log: EventLog) -> Result<Self, EventsError> {
        let mut emitter = Self::attach(storage, log);
        let replayed = emitter.emit_pending()?;
        if replayed > 0 {
            tracing::info!(replayed, "replayed unemitted event intents at startup");
        }
        Ok(emitter)
    }

    /// Couples a storage handle to a log **without** the recovery replay —
    /// for callers that drain on their own terms and treat an emission
    /// failure as non-fatal (the [`crate::CoreHandle`] facade does, so a
    /// full disk cannot keep a profile from opening).
    #[must_use]
    pub fn attach(storage: Arc<Storage>, log: EventLog) -> Self {
        Self { storage, log }
    }

    /// Under the events lock: appends every staged intent to the log
    /// (oldest first), fsyncs, then deletes the intents. Returns how many
    /// events were emitted.
    ///
    /// # Errors
    ///
    /// [`super::EventLogError::ProfileLocked`] when another process holds
    /// the lock past its timeout; storage or log errors. On an append
    /// failure the lines already appended in this call are still fsynced
    /// and their intents deleted (best effort); the failed intent and
    /// everything after it stay staged, and the next call (or startup)
    /// retries them.
    pub fn emit_pending(&mut self) -> Result<usize, EventsError> {
        let storage = &self.storage;
        self.log.locked(|log| {
            let pending = storage.pending_intents()?;
            if pending.is_empty() {
                return Ok(0);
            }
            for (appended, intent) in pending.iter().enumerate() {
                if let Err(err) = log.append(&intent.envelope) {
                    commit_prefix(log, storage, &pending[..appended]);
                    return Err(err.into());
                }
            }
            // Durability point: the fsync happens BEFORE the intents are
            // deleted, so a crash can only ever duplicate, never lose.
            log.flush()?;
            storage.mark_intents_emitted(pending.iter().map(|p| p.intent_id).collect())?;
            Ok(pending.len())
        })
    }

    /// The underlying log (rotation config, retention sweeps).
    pub fn log_mut(&mut self) -> &mut EventLog {
        &mut self.log
    }
}

/// After a mid-batch append failure: fsync the whole lines that did
/// land and retire their intents, so the retry does not re-append
/// them. Best effort — if either step fails, those intents simply stay
/// staged and the retry duplicates them (harmless: `event_id` dedupe).
/// Runs inside the same events-lock critical section as the failed
/// drain, so no other process can append in between.
fn commit_prefix(log: &mut LockedLog<'_>, storage: &Storage, landed: &[PendingIntent]) {
    if landed.is_empty() {
        return;
    }
    let retired = log.flush().map_err(EventsError::from).and_then(|()| {
        storage
            .mark_intents_emitted(landed.iter().map(|p| p.intent_id).collect())
            .map_err(EventsError::from)
    });
    if let Err(err) = retired {
        tracing::warn!(error = %err, "could not retire the intents emitted before a failed append");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::events::read_all;
    use crate::model::{ArticleContent, NewArticle};
    use crate::storage::ListArticles;

    fn seeded() -> (tempfile::TempDir, Arc<Storage>, crate::model::ArticleId) {
        let dir = tempfile::tempdir().unwrap();
        let storage = Arc::new(Storage::open(dir.path().join("curio.db")).unwrap());
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
                lead_image: None,
            }])
            .unwrap();
        let id = storage.list_articles(ListArticles::default()).unwrap()[0].id;
        (dir, storage, id)
    }

    /// #118: a disk-full on the second of three appends retires the first
    /// (fsynced) intent and keeps the rest staged; the retry then emits
    /// each event exactly once, as whole lines.
    #[test]
    fn a_mid_batch_append_failure_keeps_the_rest_staged_without_duplicates() {
        let (dir, storage, id) = seeded();
        let events_dir = dir.path().join("events");
        let mut emitter = EventEmitter::attach(
            Arc::clone(&storage),
            EventLog::open(&events_dir, dir.path().join("events.lock")).unwrap(),
        );
        let star = storage.star_article(id).unwrap().unwrap();
        let tag = storage.tag_article(id, "rust").unwrap().unwrap();
        let archive = storage.archive_article(id).unwrap().unwrap();

        emitter.log_mut().inject_short_write(1, 9);
        emitter.emit_pending().unwrap_err();
        let staged: Vec<_> = storage
            .pending_intents()
            .unwrap()
            .into_iter()
            .map(|p| p.envelope)
            .collect();
        assert_eq!(staged, vec![tag.clone(), archive.clone()]);
        assert_eq!(read_all(&events_dir).unwrap(), vec![star.clone()]);

        assert_eq!(emitter.emit_pending().unwrap(), 2);
        assert_eq!(storage.pending_intents().unwrap(), []);
        assert_eq!(read_all(&events_dir).unwrap(), vec![star, tag, archive]);
    }

    /// A failure on the first append retires nothing and loses nothing.
    #[test]
    fn a_failure_on_the_first_append_keeps_every_intent_staged() {
        let (dir, storage, id) = seeded();
        let events_dir = dir.path().join("events");
        let mut emitter = EventEmitter::attach(
            Arc::clone(&storage),
            EventLog::open(&events_dir, dir.path().join("events.lock")).unwrap(),
        );
        let star = storage.star_article(id).unwrap().unwrap();

        emitter.log_mut().inject_short_write(0, 3);
        emitter.emit_pending().unwrap_err();
        assert_eq!(storage.pending_intents().unwrap().len(), 1);
        assert_eq!(read_all(&events_dir).unwrap(), vec![]);

        assert_eq!(emitter.emit_pending().unwrap(), 1);
        assert_eq!(read_all(&events_dir).unwrap(), vec![star]);
    }
}
