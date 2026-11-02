//! Maintenance: the database health probes and housekeeping the doctor
//! panels run (schema version, integrity checks, `VACUUM`, staged
//! intents, the search-index rebuild), the event-log retention sweep the
//! facade owns, and the routine [`CoreHandle::maintenance`] pass with its
//! article-retention policy.

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::PoisonError;
use std::sync::atomic::Ordering;

use curio_types::Timestamp;
use serde::{Deserialize, Serialize};

use crate::events::EventsError;
use crate::model::RetentionPolicy;
use crate::storage::MaintenancePlan;

use super::{CoreError, CoreHandle};

/// Settings key holding the article-retention policy (JSON; facade-owned —
/// see [`CoreHandle::retention_policy`]).
pub(super) const RETENTION_POLICY_KEY: &str = "retention.policy";

/// What a [`CoreHandle::maintenance`] pass does beyond its routine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MaintenanceOptions {
    /// Also delete the articles a feed removal left behind (unprotected
    /// ones only — starred, read-later, tagged and exported stay). Off by
    /// default: orphans are purged on request.
    pub purge_orphans: bool,
}

/// What a [`CoreHandle::maintenance`] pass did.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MaintenanceReport {
    /// Feed articles the retention policy reaped.
    pub articles_reaped: u64,
    /// Orphaned articles purged (with [`MaintenanceOptions::purge_orphans`]).
    pub orphans_purged: u64,
    /// Tags no article carried any more.
    pub tags_dropped: u64,
    /// Retention tombstones forgotten (their item has left its feed).
    pub tombstones_pruned: u64,
    /// `fetch_log` rows beyond the per-feed cap.
    pub fetch_log_trimmed: u64,
    /// Event-log files past the retention window that were deleted.
    pub event_files_removed: Vec<PathBuf>,
    /// Why no article was reaped this pass although a reap was asked for:
    /// the exported set could not be read (a vault that is unmounted or
    /// whose manifest does not load), and nothing is deleted on a guess.
    pub reap_skipped: Option<String>,
    /// Why the event-log sweep did not run (another process holds the
    /// events lock, a filesystem error); the rest of the pass still ran.
    pub event_sweep_skipped: Option<String>,
    /// Why the database upkeep after the reap (the search-index merge,
    /// `PRAGMA optimize`, the WAL checkpoint) stopped short — another
    /// process held the write lock too long, say. What was deleted stays
    /// deleted and is counted; the next pass catches up.
    pub upkeep_skipped: Option<String>,
}

/// The persisted form of a [`RetentionPolicy`].
#[derive(Debug, Serialize, Deserialize)]
struct StoredPolicy {
    max_age_days: Option<NonZeroU32>,
    max_per_feed: Option<NonZeroU32>,
    keep_unread: bool,
}

impl CoreHandle {
    /// The article-retention policy as persisted (the default — off,
    /// keep unread — until one is set).
    ///
    /// # Errors
    ///
    /// Storage errors; [`CoreError::Settings`] for a stored value that
    /// does not parse.
    pub fn retention_policy(&self) -> Result<RetentionPolicy, CoreError> {
        let Some(raw) = self.storage.get_setting(RETENTION_POLICY_KEY)? else {
            return Ok(RetentionPolicy::default());
        };
        let stored: StoredPolicy = serde_json::from_str(&raw)?;
        Ok(RetentionPolicy {
            max_age_days: stored.max_age_days,
            max_per_feed: stored.max_per_feed,
            keep_unread: stored.keep_unread,
        })
    }

    /// Persists the article-retention policy. Nothing is reaped until the
    /// next [`Self::maintenance`] pass.
    ///
    /// # Errors
    ///
    /// Storage or serialization errors.
    pub fn set_retention_policy(&self, policy: RetentionPolicy) -> Result<(), CoreError> {
        let stored = StoredPolicy {
            max_age_days: policy.max_age_days,
            max_per_feed: policy.max_per_feed,
            keep_unread: policy.keep_unread,
        };
        let raw = serde_json::to_string(&stored)?;
        Ok(self.storage.set_setting(RETENTION_POLICY_KEY, &raw)?)
    }

    /// The routine maintenance pass — for heads to run at startup and
    /// about daily, and on demand (`curio maintenance`).
    ///
    /// - applies the [`Self::retention_policy`] (and, with
    ///   [`MaintenanceOptions::purge_orphans`], the orphan purge), never
    ///   reaping a starred, read-later, tagged or exported article — nor
    ///   an unread one while the policy keeps unread. A reaped article
    ///   does not come back while its feed keeps listing it;
    /// - drops unused tags and trims the per-feed fetch log;
    /// - runs a bounded incremental search-index merge (after deletes),
    ///   `PRAGMA optimize` and a WAL checkpoint;
    /// - sweeps event-log files past the retention window, as
    ///   [`Self::sweep_event_retention`].
    ///
    /// "Exported" is every article [`Self::save_to_destination`] ever
    /// exported — recorded in the database before the note is written, so
    /// an export running alongside the pass is never reaped, and the
    /// record outlives the destination being removed or moved — plus,
    /// for exports made before Curio recorded them, what each registered
    /// destination's manifest lists (recorded by the pass in turn). If
    /// any of those manifests cannot be read, the pass reaps nothing (see
    /// [`MaintenanceReport::reap_skipped`]) and does the rest. An event
    /// sweep or upkeep step that cannot run is reported the same way
    /// ([`MaintenanceReport::event_sweep_skipped`],
    /// [`MaintenanceReport::upkeep_skipped`]).
    ///
    /// # Errors
    ///
    /// Storage or settings errors.
    pub fn maintenance(&self, options: MaintenanceOptions) -> Result<MaintenanceReport, CoreError> {
        let mut plan = MaintenancePlan {
            retention: self.retention_policy()?,
            purge_orphans: options.purge_orphans,
            exported: std::collections::BTreeSet::new(),
            now: Timestamp::now(),
        };
        let mut reap_skipped = None;
        if plan.retention.is_enabled() || plan.purge_orphans {
            match self.exported_curio_ids() {
                Ok(exported) => plan.exported = exported,
                Err(err) => {
                    tracing::warn!(%err, "maintenance reaps nothing: the exported set is unreadable");
                    reap_skipped = Some(err.to_string());
                    plan.retention = RetentionPolicy::default();
                    plan.purge_orphans = false;
                }
            }
        }
        let counts = self.storage.run_maintenance(plan)?;
        let (event_files_removed, event_sweep_skipped) = match self.sweep_event_retention() {
            Ok(removed) => (removed, None),
            Err(err) => {
                tracing::warn!(%err, "maintenance skipped the event-log sweep");
                (Vec::new(), Some(err.to_string()))
            }
        };
        Ok(MaintenanceReport {
            articles_reaped: counts.articles_reaped,
            orphans_purged: counts.orphans_purged,
            tags_dropped: counts.tags_dropped,
            tombstones_pruned: counts.tombstones_pruned,
            fetch_log_trimmed: counts.fetch_log_trimmed,
            event_files_removed,
            reap_skipped,
            event_sweep_skipped,
            upkeep_skipped: counts.upkeep_skipped,
        })
    }

    /// Rebuilds the full-text search index from the stored articles — the
    /// repair when [`Self::fts_integrity_check`] reports it out of step.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn rebuild_search_index(&self) -> Result<(), CoreError> {
        Ok(self.storage.rebuild_fts()?)
    }

    /// Path of the profile's `curio.db`.
    #[must_use]
    pub fn db_path(&self) -> &Path {
        self.storage.path()
    }

    /// The schema version recorded in the open database (compare with
    /// [`crate::storage::Storage::schema_version`], the version this build
    /// writes).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn db_schema_version(&self) -> Result<i64, CoreError> {
        Ok(self.storage.db_schema_version()?)
    }

    /// Runs `PRAGMA integrity_check` over the database.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `IntegrityFailed` with the verdict
    /// when the check does not pass; storage errors.
    pub fn integrity_check(&self) -> Result<(), CoreError> {
        Ok(self.storage.integrity_check()?)
    }

    /// Verifies the FTS5 index matches the `articles` table.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `StorageError::Sqlite` with
    /// `SQLITE_CORRUPT_VTAB` when the index has drifted (rebuildable — never
    /// `DatabaseCorrupt`); other storage errors.
    pub fn fts_integrity_check(&self) -> Result<(), CoreError> {
        Ok(self.storage.fts_integrity_check()?)
    }

    /// Compacts the database file (`VACUUM`).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn vacuum(&self) -> Result<(), CoreError> {
        Ok(self.storage.vacuum()?)
    }

    /// How many event intents are staged in the database but not yet
    /// flushed to the JSONL log. Zero in a healthy profile: every
    /// operation flushes before it returns, and `open` replays leftovers;
    /// non-zero after a crash or a failed flush (full disk, unwritable
    /// events dir — see [`CoreHandle::has_pending_events`]).
    ///
    /// A pure read (the doctor probe): it counts every staged row, an
    /// undecodable one included, and never quarantines — that only
    /// happens when intents are drained.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn pending_intent_count(&self) -> Result<usize, CoreError> {
        let count = self.storage.pending_intent_count()?;
        Ok(usize::try_from(count).unwrap_or(usize::MAX))
    }

    /// How many staged event intents were quarantined because their
    /// envelope no longer decoded. Zero in a healthy profile; anything
    /// else means those events never reached the JSONL log (the raw rows
    /// are kept in the database's `event_intents_dead` table).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn quarantined_intent_count(&self) -> Result<u64, CoreError> {
        Ok(self.storage.quarantined_intent_count()?)
    }

    /// The "pending events" health flag: `true` once a flush to the JSONL
    /// log failed and its intents are still staged — or the log could not
    /// be opened at all (an unwritable `.curio/events`), so nothing can be
    /// flushed yet; cleared by the next successful drain. A cheap in-memory check — heads poll it to decide
    /// whether to call [`CoreHandle::flush_pending_events`].
    #[must_use]
    pub fn has_pending_events(&self) -> bool {
        self.events_backlog.load(Ordering::Acquire)
    }

    /// Drains every staged event intent to the JSONL log now — the retry
    /// for a flush that failed earlier (mutations treat their own flush
    /// as non-fatal). Returns how many events were emitted, and updates
    /// the [`CoreHandle::has_pending_events`] flag either way.
    ///
    /// # Errors
    ///
    /// Storage or event-log errors; the intents stay staged and the flag
    /// stays raised.
    pub fn flush_pending_events(&self) -> Result<usize, CoreError> {
        let mut events = self.events.lock().unwrap_or_else(PoisonError::into_inner);
        // A log that could not be opened yet is retried here, so a
        // repaired events directory drains on the next call.
        let drained = events
            .emitter()
            .and_then(|emitter| Ok(emitter.emit_pending()?));
        // Under the sink lock, so concurrent drains set the flag in the
        // order they ran.
        self.events_backlog
            .store(drained.is_err(), Ordering::Release);
        drop(events);
        drained
    }

    /// Deletes event-log files older than the retention window (≥ 90
    /// days — the `curio.events.v1` floor), measured from today (UTC).
    /// Returns the removed paths.
    ///
    /// Facade-owned on purpose: the handle owns this process's log
    /// writer (other processes on the profile are fenced off by the
    /// events lock), so heads must never open their own
    /// [`crate::events::EventLog`] to sweep.
    ///
    /// # Errors
    ///
    /// [`crate::events::EventLogError::ProfileLocked`] while another
    /// process holds the events lock; filesystem errors scanning or
    /// deleting log files.
    pub fn sweep_event_retention(&self) -> Result<Vec<PathBuf>, CoreError> {
        let today = Timestamp::now().as_datetime().date_naive();
        let removed = self
            .events
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .emitter()?
            .log_mut()
            .sweep_retention(today)
            .map_err(EventsError::Log)?;
        Ok(removed)
    }
}
