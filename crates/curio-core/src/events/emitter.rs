//! The storage-coupled emitter: intents → JSONL, crash-recoverably.
//!
//! Ordering contract (the "transactional-ish" pair):
//!
//! 1. the DB state change commits first, *with* its write-ahead intent row
//!    in the same transaction (the storage repo does this);
//! 2. the emitter appends the staged envelope(s) to the JSONL log;
//! 3. the emitter fsyncs the log ([`EventLog::flush`]);
//! 4. only then are the intent rows deleted.
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

use std::sync::Arc;

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
    /// intents a previous process staged but never emitted.
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

    /// Appends every staged intent to the log (oldest first), fsyncs,
    /// then deletes the intents. Returns how many events were emitted.
    ///
    /// # Errors
    ///
    /// Storage or log errors. On an append failure the lines already
    /// appended in this call are still fsynced and their intents deleted
    /// (best effort); the failed intent and everything after it stay
    /// staged, and the next call (or startup) retries them.
    pub fn emit_pending(&mut self) -> Result<usize, EventsError> {
        let pending = self.storage.pending_intents()?;
        if pending.is_empty() {
            return Ok(0);
        }
        for (appended, intent) in pending.iter().enumerate() {
            if let Err(err) = self.log.append(&intent.envelope) {
                self.commit_prefix(&pending[..appended]);
                return Err(err.into());
            }
        }
        // Durability point: the fsync happens BEFORE the intents are
        // deleted, so a crash can only ever duplicate, never lose.
        self.log.flush()?;
        self.storage
            .mark_intents_emitted(pending.iter().map(|p| p.intent_id).collect())?;
        Ok(pending.len())
    }

    /// After a mid-batch append failure: fsync the whole lines that did
    /// land and retire their intents, so the retry does not re-append
    /// them. Best effort — if either step fails, those intents simply stay
    /// staged and the retry duplicates them (harmless: `event_id` dedupe).
    fn commit_prefix(&mut self, landed: &[PendingIntent]) {
        if landed.is_empty() {
            return;
        }
        let retired = self.log.flush().map_err(EventsError::from).and_then(|()| {
            self.storage
                .mark_intents_emitted(landed.iter().map(|p| p.intent_id).collect())
                .map_err(EventsError::from)
        });
        if let Err(err) = retired {
            tracing::warn!(error = %err, "could not retire the intents emitted before a failed append");
        }
    }

    /// The underlying log (rotation config, retention sweeps).
    pub fn log_mut(&mut self) -> &mut EventLog {
        &mut self.log
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
        let mut emitter =
            EventEmitter::attach(Arc::clone(&storage), EventLog::open(&events_dir).unwrap());
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
        let mut emitter =
            EventEmitter::attach(Arc::clone(&storage), EventLog::open(&events_dir).unwrap());
        let star = storage.star_article(id).unwrap().unwrap();

        emitter.log_mut().inject_short_write(0, 3);
        emitter.emit_pending().unwrap_err();
        assert_eq!(storage.pending_intents().unwrap().len(), 1);
        assert_eq!(read_all(&events_dir).unwrap(), vec![]);

        assert_eq!(emitter.emit_pending().unwrap(), 1);
        assert_eq!(read_all(&events_dir).unwrap(), vec![star]);
    }
}
