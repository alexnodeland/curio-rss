//! Background refresh scheduler — an on-launch pass plus an interval loop, so
//! the reader stays current without a click. It reuses the head's manual
//! refresh path (`refresh_all_and_emit`), so the same
//! `RefreshProgress`/`RefreshFinished`/`FeedsChanged`/`ArticlesChanged` events
//! fire and counts + status update for free.
//!
//! Config lives in `SQLite` settings and is read fresh each tick, so a change in
//! Settings → General takes effect within a minute:
//! - `ui.refresh.interval-minutes` — how often to refresh (`0` disables).
//! - `ui.refresh.on-launch` — whether to refresh once shortly after start.
//!
//! Coordination with a manual refresh is the [`RefreshScheduler`]: a manual
//! refresh calls [`RefreshScheduler::mark_refreshed`], resetting the interval
//! clock so a scheduled sweep never piles on right after one; an in-flight flag
//! keeps two scheduled ticks from overlapping (per-feed locks make overlap
//! safe but pointless).
//!
//! The same tick is also the event-emission retry timer (#118): a failed
//! JSONL flush (full disk, unwritable events dir) never fails the mutation
//! that staged it — core raises its "pending events" flag instead — so each
//! tick re-drains the staged intents while that flag is up.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use curio_core::CoreHandle;
use tauri::AppHandle;

use crate::commands::feeds::refresh_all_and_emit;
use crate::commands::{SharedCore, run_blocking};

const INTERVAL_KEY: &str = "ui.refresh.interval-minutes";
const ON_LAUNCH_KEY: &str = "ui.refresh.on-launch";
const DEFAULT_INTERVAL_MINUTES: u64 = 30;
/// Let the window settle before the on-launch sweep, so first paint is snappy.
const LAUNCH_SETTLE: Duration = Duration::from_secs(5);
/// How often the loop wakes to check whether a refresh is due. Also the upper
/// bound on how long a settings change (interval / disable) takes to apply.
const TICK: Duration = Duration::from_secs(60);

/// Shared refresh coordination between the manual `refresh_all` command and the
/// background scheduler.
#[derive(Debug)]
pub struct RefreshScheduler {
    last_refresh: Mutex<Instant>,
    in_flight: AtomicBool,
}

impl Default for RefreshScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshScheduler {
    /// A scheduler whose interval clock starts now (so the first scheduled
    /// sweep is a full interval after launch, not immediately).
    #[must_use]
    pub fn new() -> Self {
        Self {
            last_refresh: Mutex::new(Instant::now()),
            in_flight: AtomicBool::new(false),
        }
    }

    /// Resets the interval clock — called after any refresh (manual or
    /// scheduled) so the next scheduled sweep is a full interval later.
    pub fn mark_refreshed(&self) {
        *self
            .last_refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Instant::now();
    }

    fn since_last(&self) -> Duration {
        self.last_refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .elapsed()
    }

    /// Claims the refresh slot; `false` if a scheduled sweep is already running.
    fn try_acquire(&self) -> bool {
        !self.in_flight.swap(true, Ordering::SeqCst)
    }

    fn release(&self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }
}

/// Whether a refresh is due: enabled (interval > 0) and at least that long
/// since the last one. Pure — the loop's decision, unit-tested without a clock.
#[must_use]
pub fn refresh_due(interval_minutes: u64, since_last: Duration) -> bool {
    interval_minutes > 0 && since_last >= Duration::from_secs(interval_minutes * 60)
}

/// The configured interval in minutes (`0` = disabled), defaulting to
/// `DEFAULT_INTERVAL_MINUTES` when unset or unparseable.
fn interval_minutes(core: &CoreHandle) -> u64 {
    core.get_setting(INTERVAL_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
}

/// Whether to refresh once shortly after launch (defaults to on).
fn refresh_on_launch(core: &CoreHandle) -> bool {
    core.get_setting(ON_LAUNCH_KEY)
        .ok()
        .flatten()
        .is_none_or(|value| value != "false")
}

/// Spawns the background refresh loop onto the Tauri async runtime: an
/// on-launch sweep (if enabled) after a short settle, then — every `TICK` —
/// the pending-events retry and a due-check against the configured interval.
/// The task lives for the process.
pub fn spawn(handle: AppHandle, core: SharedCore, scheduler: std::sync::Arc<RefreshScheduler>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LAUNCH_SETTLE).await;
        if refresh_on_launch(&core) {
            run_once(&handle, &core, &scheduler).await;
        }
        loop {
            tokio::time::sleep(TICK).await;
            retry_pending_events(&core).await;
            if refresh_due(interval_minutes(&core), scheduler.since_last()) {
                run_once(&handle, &core, &scheduler).await;
            }
        }
    });
}

/// Re-drains event intents a failed flush left staged. A no-op (one atomic
/// load) while core's pending-events flag is down.
async fn retry_pending_events(core: &SharedCore) {
    if !core.has_pending_events() {
        return;
    }
    let core = std::sync::Arc::clone(core);
    if let Err(error) = run_blocking(move || Ok(flush_pending_events_impl(&core))).await {
        tracing::warn!(%error, "pending-events retry did not run");
    }
}

/// The blocking half of [`retry_pending_events`]: drain and log the outcome.
/// Returns how many staged events reached the log.
fn flush_pending_events_impl(core: &CoreHandle) -> usize {
    match core.flush_pending_events() {
        Ok(emitted) => {
            tracing::info!(emitted, "pending events flushed to the event log");
            emitted
        }
        Err(error) => {
            tracing::warn!(%error, "pending events still cannot be flushed; retrying next tick");
            0
        }
    }
}

/// One scheduled sweep, guarded so two ticks never overlap. Emits the same
/// events a manual refresh does; a fetch/parse failure is an outcome (logged
/// in the feed-health log), never a panic.
async fn run_once(handle: &AppHandle, core: &SharedCore, scheduler: &RefreshScheduler) {
    if !scheduler.try_acquire() {
        return;
    }
    match refresh_all_and_emit(handle, core).await {
        // A background sweep is the moment to notify (foreground refreshes are
        // reported in-app by the refresh status line); the notify layer decides
        // whether the user's prefs + OS permission actually surface anything.
        Ok(outcomes) => crate::notify::notify_refresh(handle, core, &outcomes),
        Err(error) => tracing::warn!(%error, "scheduled refresh failed"),
    }
    scheduler.mark_refreshed();
    scheduler.release();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};

    /// The timer's drain: nothing while the events dir is unwritable, the
    /// whole backlog once it is back (#118).
    #[test]
    #[allow(clippy::unwrap_used)] // test assertions
    fn the_retry_drains_the_backlog_once_the_log_is_writable_again() {
        let (_dir, core) = temp_core();
        let id = curio_core::model::ArticleId(seed_article(&core, "starred"));
        let events_dir = core.profile_dir().join(".curio").join("events");
        std::fs::remove_dir_all(&events_dir).unwrap();
        std::fs::write(&events_dir, b"not a directory").unwrap();

        assert!(core.set_starred(id, true).unwrap(), "the mutation succeeds");
        assert!(core.has_pending_events());
        assert_eq!(flush_pending_events_impl(&core), 0);
        assert!(core.has_pending_events());

        std::fs::remove_file(&events_dir).unwrap();
        std::fs::create_dir(&events_dir).unwrap();
        assert_eq!(flush_pending_events_impl(&core), 1);
        assert!(!core.has_pending_events());
        assert_eq!(core.pending_intent_count().unwrap(), 0);
    }

    #[test]
    fn a_zero_interval_is_never_due() {
        assert!(!refresh_due(0, Duration::from_secs(0)));
        assert!(!refresh_due(0, Duration::from_hours(24)));
    }

    #[test]
    fn a_refresh_is_due_only_once_the_interval_has_elapsed() {
        // 30-minute interval: not due at 29 minutes, due at exactly 30.
        assert!(!refresh_due(30, Duration::from_mins(29)));
        assert!(refresh_due(30, Duration::from_mins(30)));
        assert!(refresh_due(30, Duration::from_mins(31)));
    }

    #[test]
    fn mark_refreshed_resets_the_clock() {
        let scheduler = RefreshScheduler::new();
        // A fresh scheduler's clock started ~now, so barely any time has passed.
        assert!(scheduler.since_last() < Duration::from_secs(1));
        scheduler.mark_refreshed();
        assert!(scheduler.since_last() < Duration::from_secs(1));
    }

    #[test]
    fn the_in_flight_flag_admits_one_holder_at_a_time() {
        let scheduler = RefreshScheduler::new();
        assert!(scheduler.try_acquire(), "first acquire succeeds");
        assert!(
            !scheduler.try_acquire(),
            "a second is refused while in flight"
        );
        scheduler.release();
        assert!(
            scheduler.try_acquire(),
            "released, so it can be claimed again"
        );
    }
}
