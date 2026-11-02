//! Bulk export: every article matching a filter into one destination, in
//! batches of [`BULK_SAVE_BATCH`] notes.
//!
//! Each batch keeps the contract's crash ordering — every note of the
//! batch is written, then all of their event intents are staged in one
//! transaction, then the destination's manifest is written once — and
//! drains the intents to the event log once. A per-note export would read,
//! index and rewrite the whole manifest, and fsync the event log, once per
//! note: quadratic in the size of the destination.
//!
//! A note that fails — its content does not convert, or its file cannot be
//! written — is reported and skipped. A failure of the destination itself
//! (its folder gone, its lock or manifest unusable) or of the profile's
//! storage ends the run early: every further note would fail the same way.
//! A storage failure partway through a batch drops that batch before its
//! intents and manifest write, exactly like a crash there; the next export
//! reconciles the notes it had already written.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use curio_types::{CurioId, Destination, DestinationName, EventPayload};

use crate::export::{self, ExportBatch, ExportDisposition, StageError};
use crate::model::{Article, ArticleId, Feed, FeedId};
use crate::storage::ListArticles;

use super::destinations::{load_exported, load_roots};
use super::{CoreError, CoreHandle};

/// Notes per batch of a bulk export: one page of the article walk, one
/// event transaction, one manifest write and one event-log flush. A
/// destination's export lock is held for at most one batch at a time.
pub const BULK_SAVE_BATCH: u32 = 200;

/// How many failures [`BulkSaveOutcome::failures`] lists in detail
/// ([`BulkSaveOutcome::failed`] stays exact past the cap).
pub const MAX_BULK_SAVE_FAILURES: usize = 100;

/// Outcome of a bulk note export: how many notes each disposition got,
/// and which articles could not be exported.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BulkSaveOutcome {
    /// Notes written for the first time (`article.saved` each).
    pub created: u64,
    /// Existing notes whose managed region changed (`article.updated` each).
    pub updated: u64,
    /// Idempotency hits — already exported, byte-identical, no event.
    pub unchanged: u64,
    /// Articles that could not be exported. The run carried on past each
    /// of them; re-running the export retries them.
    pub failed: u64,
    /// The first [`MAX_BULK_SAVE_FAILURES`] failures, in walk order.
    pub failures: Vec<BulkSaveFailure>,
    /// Whether a [`BulkSaveCancel`] stopped the run before it walked every
    /// matching article. Everything exported until then stays exported.
    pub cancelled: bool,
}

impl BulkSaveOutcome {
    /// Articles exported or found up to date — every article the run
    /// walked except the failed ones.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.created + self.updated + self.unchanged
    }

    fn fail(&mut self, article: &Article, error: &CoreError) {
        // Reported to the caller in the outcome (and so by every head), not
        // a warning of its own; past the detail cap this is the record.
        tracing::debug!(curio_id = %article.curio_id, %error, "bulk export skipped an article");
        self.failed += 1;
        if self.failures.len() < MAX_BULK_SAVE_FAILURES {
            self.failures.push(BulkSaveFailure {
                article_id: article.id,
                curio_id: article.curio_id,
                title: article.title.clone(),
                error: error.to_string(),
            });
        }
    }
}

/// One article a bulk export could not write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BulkSaveFailure {
    /// The article's row id.
    pub article_id: ArticleId,
    /// The article's identity.
    pub curio_id: CurioId,
    /// The article's title, to tell the user which one it was.
    pub title: String,
    /// Why it failed (the error's message).
    pub error: String,
}

/// A bulk export's progress, reported once per committed batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BulkSaveProgress {
    /// Articles walked so far, failed ones included.
    pub done: u64,
    /// Articles matching the filter when the run started. Articles added
    /// meanwhile are not walked, and removed ones are not found, so `done`
    /// can end short of it.
    pub total: u64,
    /// Of `done`, the articles that failed.
    pub failed: u64,
}

/// Stops a running bulk export from another thread: clone it, hand one to
/// [`CoreHandle::save_all_to_destination_with`], and call
/// [`Self::cancel`] on the other. The run finishes the note in hand,
/// commits its batch, and returns with [`BulkSaveOutcome::cancelled`].
#[derive(Debug, Clone, Default)]
pub struct BulkSaveCancel(Arc<AtomicBool>);

impl BulkSaveCancel {
    /// A token that has not been cancelled.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Asks the run holding a clone of this token to stop.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    /// Whether [`Self::cancel`] was called on any clone.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Relaxed)
    }
}

/// What a batch did, counted once its manifest write is committed.
#[derive(Default)]
struct Tally {
    created: u64,
    updated: u64,
    unchanged: u64,
}

impl CoreHandle {
    /// Exports every article matching `filter` to a named destination —
    /// the export-everything path ("my library as markdown"). See
    /// [`Self::save_all_to_destination_with`]; this one cannot be
    /// cancelled and reports no progress.
    ///
    /// # Errors
    ///
    /// See [`Self::save_all_to_destination_with`].
    pub fn save_all_to_destination(
        &self,
        filter: &ListArticles,
        destination: &DestinationName,
    ) -> Result<BulkSaveOutcome, CoreError> {
        self.save_all_to_destination_with(filter, destination, &BulkSaveCancel::new(), |_| {})
    }

    /// Exports every article matching `filter` to a named destination.
    /// Each note goes through the same steps as
    /// [`Self::save_to_destination`] (frontmatter, managed region,
    /// manifest idempotency, `article.saved`/`article.updated` events), so
    /// re-running a bulk export only rewrites what actually changed.
    /// `filter.before`/`limit` are pagination internals and are ignored.
    ///
    /// The notes go in batches of [`BULK_SAVE_BATCH`] (see the module
    /// docs); `on_progress` is called after each batch commits. An article
    /// that fails — including a note the user edited into something that
    /// no longer parses, even when the article itself is unchanged — is
    /// skipped and listed in [`BulkSaveOutcome::failures`]; the run
    /// carries on. `cancel` stops the run after the note in hand.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`], and a destination that is
    /// missing ([`crate::export::ExportError::DestinationMissing`]) or
    /// whose lock or manifest is unusable — checked before anything is
    /// walked, and again by every batch. [`CoreError::Storage`] from any
    /// read or write of the profile (walking the articles, an article's
    /// feed or tags, staging a batch's events) ends the run too. Batches
    /// committed before the error stay exported.
    pub fn save_all_to_destination_with(
        &self,
        filter: &ListArticles,
        destination: &DestinationName,
        cancel: &BulkSaveCancel,
        mut on_progress: impl FnMut(BulkSaveProgress),
    ) -> Result<BulkSaveOutcome, CoreError> {
        let root = load_roots(&self.storage)?
            .remove(destination)
            .ok_or_else(|| CoreError::UnknownDestination {
                name: destination.to_string(),
            })?;
        let mut marked = load_exported(&self.storage)?.contains(destination);
        let dest = Destination {
            name: destination.clone(),
            root,
        };
        let mut page_filter = filter.clone();
        page_filter.before = None;
        page_filter.limit = BULK_SAVE_BATCH;
        let total = self.storage.count_listed_articles(page_filter.clone())?;
        let mut feeds = HashMap::new();
        let mut outcome = BulkSaveOutcome::default();
        loop {
            // A destination that has exported must still be there — before
            // the first batch, and before every later one: a volume
            // unmounted or a vault removed mid-run leaves an empty folder
            // at the old path, which must be refused, not filled. (Neither
            // the batch nor the export lock checks for the marker; the
            // lock would create `.curio/` again.)
            if marked {
                export::require_marker(&dest.root)?;
            }
            let page = self.storage.list_articles(page_filter.clone())?;
            let Some(last) = page.last() else {
                break;
            };
            // Checked here too, not only per note: a cancel that landed
            // after the last note of a full page stops the run without
            // starting another batch (or reporting the same progress twice).
            if cancel.is_cancelled() {
                outcome.cancelled = true;
                break;
            }
            page_filter.before = Some(last.id);
            let full = page.len() == BULK_SAVE_BATCH as usize;
            self.export_batch(&dest, page, &mut feeds, cancel, &mut outcome)?;
            if !marked && outcome.total() > 0 {
                self.mark_exported(destination, dest.root.clone());
                marked = true;
            }
            on_progress(BulkSaveProgress {
                done: outcome.total() + outcome.failed,
                total,
                failed: outcome.failed,
            });
            if outcome.cancelled || !full {
                break;
            }
        }
        Ok(outcome)
    }

    /// Exports one page of articles as one batch: notes, then their event
    /// intents in one transaction, then the manifest, then one flush.
    fn export_batch(
        &self,
        dest: &Destination,
        page: Vec<Article>,
        feeds: &mut HashMap<FeedId, Option<Feed>>,
        cancel: &BulkSaveCancel,
        outcome: &mut BulkSaveOutcome,
    ) -> Result<(), CoreError> {
        let mut batch = ExportBatch::begin(dest)?;
        let mut events = Vec::new();
        let mut tally = Tally::default();
        for article in page {
            if cancel.is_cancelled() {
                outcome.cancelled = true;
                break;
            }
            let feed = match article.feed_id {
                Some(feed_id) => match feeds.get(&feed_id) {
                    Some(feed) => feed.clone(),
                    // A storage error is the profile's, not this article's:
                    // it ends the run (see the module docs).
                    None => feeds
                        .entry(feed_id)
                        .or_insert(self.storage.get_feed(feed_id)?)
                        .clone(),
                },
                None => None,
            };
            let input = match self.export_input(&article, feed.as_ref()) {
                Ok(input) => input,
                Err(err @ CoreError::Storage(_)) => return Err(err),
                Err(err) => {
                    outcome.fail(&article, &err);
                    continue;
                }
            };
            let staged = match batch.stage(&input) {
                Ok(staged) => staged,
                // Every further note would fail the same way; nothing of
                // this batch was written yet (only the first write takes
                // the lock), so there is nothing to commit.
                Err(StageError::Destination(err)) => return Err(err.into()),
                Err(StageError::Note(err)) => {
                    outcome.fail(&article, &err.into());
                    continue;
                }
            };
            let snapshot = || {
                article.snapshot(
                    input.feed.clone(),
                    input.feed_title.clone(),
                    input.tags.clone(),
                    dest.name.clone(),
                    staged.path.clone(),
                    staged.checksum,
                )
            };
            match staged.disposition {
                ExportDisposition::Created => {
                    events.push(EventPayload::ArticleSaved(snapshot()));
                    tally.created += 1;
                }
                ExportDisposition::Updated => {
                    events.push(EventPayload::ArticleUpdated(snapshot()));
                    tally.updated += 1;
                }
                ExportDisposition::Unchanged => tally.unchanged += 1,
            }
        }
        // Ordering, per batch: notes (written above) → event intents →
        // manifest. A failure staging the intents drops the batch with its
        // manifest write, so no manifest entry ever outruns its event.
        let emitted = !events.is_empty();
        self.storage.record_article_exports(events)?;
        batch.commit()?;
        outcome.created += tally.created;
        outcome.updated += tally.updated;
        outcome.unchanged += tally.unchanged;
        if emitted {
            self.emit();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::cell::Cell;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::events::read_all;
    use crate::export::{ExportError, MANIFEST_WRITES, load_manifest};
    use crate::model::{ArticleContent, NewArticle};

    /// What exporting 5,000 notes may take, end to end, in this unoptimized
    /// test build on the slowest CI runner (Windows, whose file creation is
    /// slow). It runs in about 25 s on a Linux runner — almost all of it
    /// per-note CPU work (HTML → markdown, YAML), since the batched path
    /// does linear I/O. The per-note path it replaced rewrote 5,000
    /// manifests averaging 2,500 entries — gigabytes of writes, each
    /// preceded by a linear collision scan of the manifest — and fsynced
    /// the event log 5,000 times.
    const BUDGET_5K: Duration = Duration::from_secs(120);

    fn manual(i: usize) -> NewArticle {
        NewArticle {
            feed_id: None,
            dedupe_key: format!("k{i}"),
            title: format!("Note {i}"),
            source_url: format!("https://example.com/{i}"),
            author: None,
            published_at: None,
            content: ArticleContent {
                html: format!("<p>body {i}</p>"),
                text: format!("body {i}"),
            },
            lang: None,
            word_count: None,
            source_updated_at: None,
            lead_image: None,
        }
    }

    fn manifest_writes() -> usize {
        MANIFEST_WRITES.with(Cell::get)
    }

    fn vault_core(articles: usize) -> (tempfile::TempDir, tempfile::TempDir, CoreHandle) {
        let profile = tempfile::tempdir().unwrap();
        let vault = tempfile::tempdir().unwrap();
        let core = CoreHandle::open(profile.path()).unwrap();
        core.add_destination(vault_name(), vault.path().to_path_buf())
            .unwrap();
        core.storage
            .upsert_articles((0..articles).map(manual).collect())
            .unwrap();
        (profile, vault, core)
    }

    fn vault_name() -> DestinationName {
        "vault".parse().unwrap()
    }

    /// A second connection to the profile database, to break it under a
    /// running export.
    fn raw_db(profile: &tempfile::TempDir) -> rusqlite::Connection {
        let db = rusqlite::Connection::open(profile.path().join(crate::paths::DB_FILE)).unwrap();
        db.busy_timeout(Duration::from_secs(10)).unwrap();
        db
    }

    /// Markdown notes under `root`, outside `.curio/`.
    fn notes_on_disk(root: &std::path::Path) -> usize {
        std::fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| !path.ends_with(".curio"))
            .map(|path| {
                if path.is_dir() {
                    notes_on_disk(&path)
                } else {
                    usize::from(path.extension().is_some_and(|ext| ext == "md"))
                }
            })
            .sum()
    }

    #[test]
    fn five_thousand_notes_write_the_manifest_once_per_batch() {
        const N: usize = 5_000;
        let (profile, vault, core) = vault_core(N);

        let writes_before = manifest_writes();
        let started = Instant::now();
        let mut progress = Vec::new();
        let outcome = core
            .save_all_to_destination_with(
                &ListArticles::default(),
                &vault_name(),
                &BulkSaveCancel::new(),
                |step| progress.push(step),
            )
            .unwrap();
        let elapsed = started.elapsed();

        assert_eq!(outcome.created, N as u64);
        assert_eq!((outcome.failed, outcome.cancelled), (0, false));
        let batches = N.div_ceil(BULK_SAVE_BATCH as usize);
        let writes = manifest_writes() - writes_before;
        assert!(
            writes <= batches + 1,
            "{writes} manifest writes for {N} notes (at most {} allowed)",
            batches + 1
        );
        assert!(
            elapsed < BUDGET_5K,
            "exporting {N} notes took {elapsed:?} (budget {BUDGET_5K:?})"
        );
        // One progress step per committed batch, ending on the total.
        assert_eq!(progress.len(), batches);
        assert!(progress.windows(2).all(|w| w[0].done < w[1].done));
        assert_eq!(
            progress.last(),
            Some(&BulkSaveProgress {
                done: N as u64,
                total: N as u64,
                failed: 0
            })
        );
        // Nothing was lost to the batching: every note is recorded, and
        // the event log carries one article.saved per note.
        assert_eq!(load_manifest(vault.path()).unwrap().notes.len(), N);
        let saved = read_all(&profile.path().join(".curio/events"))
            .unwrap()
            .iter()
            .filter(|e| matches!(e.event, EventPayload::ArticleSaved(_)))
            .count();
        assert_eq!(saved, N);

        // A re-run is all idempotency hits: no manifest write at all.
        let writes_before = manifest_writes();
        let again = core
            .save_all_to_destination(&ListArticles::default(), &vault_name())
            .unwrap();
        assert_eq!(again.unchanged, N as u64);
        assert_eq!(manifest_writes(), writes_before);
    }

    #[test]
    fn a_cancel_stops_the_run_and_keeps_what_was_committed() {
        let batch = BULK_SAVE_BATCH as usize;
        let (_profile, vault, core) = vault_core(batch + 10);

        // Cancelled before the run: nothing is walked.
        let cancel = BulkSaveCancel::new();
        cancel.cancel();
        let none = core
            .save_all_to_destination_with(&ListArticles::default(), &vault_name(), &cancel, |_| {})
            .unwrap();
        assert!(none.cancelled);
        assert_eq!(none.total(), 0);

        // Cancelled from the first progress step: the first batch stays
        // exported and recorded; the run stops before the second batch
        // begins, so the first step is the only one reported.
        let cancel = BulkSaveCancel::new();
        let canceller = cancel.clone();
        let mut steps = 0;
        let outcome = core
            .save_all_to_destination_with(&ListArticles::default(), &vault_name(), &cancel, |_| {
                steps += 1;
                canceller.cancel();
            })
            .unwrap();
        assert!(outcome.cancelled);
        assert_eq!(steps, 1);
        assert_eq!(outcome.created, u64::from(BULK_SAVE_BATCH));
        assert_eq!(load_manifest(vault.path()).unwrap().notes.len(), batch);

        // Running it again finishes the rest.
        let rest = core
            .save_all_to_destination(&ListArticles::default(), &vault_name())
            .unwrap();
        assert_eq!(
            (rest.created, rest.unchanged, rest.cancelled),
            (10, u64::from(BULK_SAVE_BATCH), false)
        );
    }

    #[test]
    fn a_destination_that_vanishes_between_batches_ends_the_run() {
        let batch = BULK_SAVE_BATCH as usize;
        let (_profile, vault, core) = vault_core(batch + 10);

        // After the first batch the vault is unmounted, leaving an empty
        // folder at its path — once, so a run that carried on would leave
        // its second batch behind for the asserts below.
        let mut unmounted = false;
        let err = core
            .save_all_to_destination_with(
                &ListArticles::default(),
                &vault_name(),
                &BulkSaveCancel::new(),
                |_| {
                    if !unmounted {
                        std::fs::remove_dir_all(vault.path()).unwrap();
                        std::fs::create_dir(vault.path()).unwrap();
                        unmounted = true;
                    }
                },
            )
            .unwrap_err();

        assert!(
            matches!(
                err,
                CoreError::Export(ExportError::DestinationMissing { .. })
            ),
            "{err:?}"
        );
        assert!(unmounted);
        assert!(
            !vault.path().join(".curio").exists(),
            "the marker is never recreated"
        );
        assert_eq!(
            notes_on_disk(vault.path()),
            0,
            "nothing is written into the empty folder"
        );
    }

    #[test]
    fn a_storage_failure_ends_the_run_instead_of_failing_each_article() {
        let batch = BULK_SAVE_BATCH as usize;
        let (profile, vault, core) = vault_core(batch + 10);
        let db = raw_db(&profile);

        // After the first batch, reading an article's tags starts failing:
        // the profile's failure, not the second batch's articles'.
        let mut broken = false;
        let err = core
            .save_all_to_destination_with(
                &ListArticles::default(),
                &vault_name(),
                &BulkSaveCancel::new(),
                |_| {
                    if !broken {
                        db.execute_batch("DROP TABLE article_tags").unwrap();
                        broken = true;
                    }
                },
            )
            .unwrap_err();

        assert!(matches!(err, CoreError::Storage(_)), "{err:?}");
        // The batch committed before the failure stays exported.
        assert_eq!(load_manifest(vault.path()).unwrap().notes.len(), batch);
    }

    /// The per-batch crash ordering, on the real bulk path: when staging a
    /// batch's event intents fails, its manifest write never happens — the
    /// notes are on disk with no entry, and the next run announces each.
    #[test]
    fn a_batch_whose_intents_fail_to_stage_never_writes_its_manifest() {
        let (profile, vault, core) = vault_core(3);
        let db = raw_db(&profile);
        db.execute_batch(
            "CREATE TRIGGER refuse_intents BEFORE INSERT ON event_intents \
             BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();

        let err = core
            .save_all_to_destination(&ListArticles::default(), &vault_name())
            .unwrap_err();

        assert!(matches!(err, CoreError::Storage(_)), "{err:?}");
        assert_eq!(notes_on_disk(vault.path()), 3, "the notes were written");
        assert!(
            load_manifest(vault.path()).unwrap().notes.is_empty(),
            "no manifest entry outruns its event"
        );

        // With the intents back, the next run finds each note on disk and
        // announces it — no event is lost to the failed batch.
        db.execute_batch("DROP TRIGGER refuse_intents").unwrap();
        let outcome = core
            .save_all_to_destination(&ListArticles::default(), &vault_name())
            .unwrap();
        assert_eq!((outcome.created, outcome.updated), (0, 3));
        assert_eq!(load_manifest(vault.path()).unwrap().notes.len(), 3);
        let updated = read_all(&profile.path().join(".curio/events"))
            .unwrap()
            .iter()
            .filter(|e| matches!(e.event, EventPayload::ArticleUpdated(_)))
            .count();
        assert_eq!(updated, 3);
    }
}
