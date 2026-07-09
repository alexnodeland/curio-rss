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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use curio_core::CoreHandle;
use tauri::AppHandle;

use crate::commands::SharedCore;
use crate::commands::feeds::refresh_all_and_emit;

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
    core.storage()
        .get_setting(INTERVAL_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
}

/// Whether to refresh once shortly after launch (defaults to on).
fn refresh_on_launch(core: &CoreHandle) -> bool {
    core.storage()
        .get_setting(ON_LAUNCH_KEY)
        .ok()
        .flatten()
        .is_none_or(|value| value != "false")
}

/// Spawns the background refresh loop onto the Tauri async runtime: an
/// on-launch sweep (if enabled) after a short settle, then a due-check every
/// `TICK` against the configured interval. The task lives for the process.
pub fn spawn(handle: AppHandle, core: SharedCore, scheduler: std::sync::Arc<RefreshScheduler>) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LAUNCH_SETTLE).await;
        if refresh_on_launch(&core) {
            run_once(&handle, &core, &scheduler).await;
        }
        loop {
            tokio::time::sleep(TICK).await;
            if refresh_due(interval_minutes(&core), scheduler.since_last()) {
                run_once(&handle, &core, &scheduler).await;
            }
        }
    });
}

/// One scheduled sweep, guarded so two ticks never overlap. Emits the same
/// events a manual refresh does; a fetch/parse failure is an outcome (logged
/// in the feed-health log), never a panic.
async fn run_once(handle: &AppHandle, core: &SharedCore, scheduler: &RefreshScheduler) {
    if !scheduler.try_acquire() {
        return;
    }
    if let Err(error) = refresh_all_and_emit(handle, core).await {
        tracing::warn!(%error, "scheduled refresh failed");
    }
    scheduler.mark_refreshed();
    scheduler.release();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_interval_is_never_due() {
        assert!(!refresh_due(0, Duration::from_secs(0)));
        assert!(!refresh_due(0, Duration::from_secs(60 * 60 * 24)));
    }

    #[test]
    fn a_refresh_is_due_only_once_the_interval_has_elapsed() {
        // 30-minute interval: not due at 29 minutes, due at exactly 30.
        assert!(!refresh_due(30, Duration::from_secs(29 * 60)));
        assert!(refresh_due(30, Duration::from_secs(30 * 60)));
        assert!(refresh_due(30, Duration::from_secs(31 * 60)));
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
