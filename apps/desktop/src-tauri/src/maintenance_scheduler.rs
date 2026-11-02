//! Background maintenance (#137): the routine [`CoreHandle::maintenance`]
//! pass — article retention, unused tags, the fetch log, index upkeep and
//! the event-log retention sweep — run shortly after launch and about once
//! a day after that, so neither the database nor the event log grows
//! forever without the user opening the doctor.
//!
//! The clock is the wall-clock time of the last pass, persisted under
//! `ui.maintenance.last-at` (a doctor "Run maintenance" resets it too): a launch
//! within a day of the last pass skips it, and a laptop that slept through
//! the day runs it on the first hourly check after waking. The routine
//! pass never purges orphaned articles — that stays the doctor's explicit
//! action, as with `curio maintenance --purge-orphans`.
//!
//! [`CoreHandle::maintenance`]: curio_core::CoreHandle::maintenance

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use curio_core::CoreHandle;
use tauri::AppHandle;

use crate::commands::maintenance::{emit_after_maintenance, last_run, run_maintenance_impl};
use crate::commands::{SharedCore, blocking};
use crate::dto::MaintenanceReportDto;

/// How long after the last pass the next one is due.
const PERIOD: Duration = Duration::from_hours(24);
/// The first check waits for the launch refresh to get going, so the pass
/// never competes with first paint or the on-launch sweep's first writes.
const LAUNCH_SETTLE: Duration = Duration::from_mins(2);
/// How often the loop wakes to check whether a pass is due.
const TICK: Duration = Duration::from_hours(1);

/// Whether a pass is due: never run, at least a day of wall-clock
/// time since the last, or a last run in the future (the clock was set
/// back — one pass re-anchors it). Pure — unit-tested without a clock.
#[must_use]
pub fn maintenance_due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        !now.duration_since(last)
            .is_ok_and(|elapsed| elapsed < PERIOD)
    })
}

/// Runs the routine pass when it is due, logging what it did. `None` when
/// it was not due or failed (logged); the next check retries a failure.
fn run_if_due(core: &CoreHandle, now: SystemTime) -> Option<MaintenanceReportDto> {
    if !maintenance_due(last_run(core), now) {
        return None;
    }
    match run_maintenance_impl(core, false, now) {
        Ok(report) => {
            tracing::info!(
                articles_reaped = report.articles_reaped,
                tags_dropped = report.tags_dropped,
                fetch_log_trimmed = report.fetch_log_trimmed,
                event_files_removed = report.event_files_removed,
                "scheduled maintenance finished"
            );
            for (step, reason) in [
                ("article retention", &report.reap_skipped),
                ("event-log sweep", &report.event_sweep_skipped),
                ("database upkeep", &report.upkeep_skipped),
            ] {
                if let Some(reason) = reason {
                    tracing::warn!(step, %reason, "scheduled maintenance skipped a step");
                }
            }
            Some(report)
        }
        Err(error) => {
            tracing::warn!(%error, "scheduled maintenance failed; retrying at the next check");
            None
        }
    }
}

/// Spawns the maintenance loop onto the Tauri async runtime: a check
/// two minutes after launch, then hourly. The pass runs on
/// the blocking pool; a pass that deleted articles or tags emits the same
/// invalidations as the doctor's action. The task lives for the process.
pub fn spawn(handle: AppHandle, core: SharedCore) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(LAUNCH_SETTLE).await;
        loop {
            let pass_core = Arc::clone(&core);
            match blocking(move || Ok(run_if_due(&pass_core, SystemTime::now()))).await {
                Ok(Some(report)) => emit_after_maintenance(&handle, &report),
                Ok(None) => {}
                Err(error) => tracing::warn!(%error, "scheduled maintenance did not run"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::UNIX_EPOCH;

    use super::*;
    use crate::commands::maintenance::LAST_RUN_KEY;
    use crate::commands::testutil::temp_core;

    const HOUR: Duration = Duration::from_hours(1);

    #[test]
    fn a_pass_is_due_once_a_day_and_on_a_first_launch() {
        let now = UNIX_EPOCH + Duration::from_hours(500_000);
        assert!(maintenance_due(None, now), "never run");
        assert!(!maintenance_due(Some(now - HOUR), now), "ran an hour ago");
        assert!(!maintenance_due(Some(now - 23 * HOUR), now));
        assert!(maintenance_due(Some(now - 24 * HOUR), now), "a day ago");
        assert!(
            maintenance_due(Some(now - 72 * HOUR), now),
            "slept for days"
        );
        assert!(
            maintenance_due(Some(now + HOUR), now),
            "a last run in the future means the clock went back"
        );
    }

    #[test]
    fn the_first_check_runs_the_pass_and_the_next_one_waits_a_day() {
        let (_dir, core) = temp_core();
        let launch = SystemTime::now();

        let report = run_if_due(&core, launch);
        assert!(report.is_some(), "a first launch is due");
        let report = report.unwrap();
        assert_eq!(report.orphans_purged, 0, "the routine pass never purges");
        assert_eq!(
            core.get_setting(LAST_RUN_KEY).unwrap().as_deref(),
            Some(
                launch
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_secs()
                    .to_string()
                    .as_str()
            )
        );

        assert!(run_if_due(&core, launch + HOUR).is_none(), "an hour later");
        assert!(
            run_if_due(&core, launch + 25 * HOUR).is_some(),
            "a day later"
        );
    }

    #[test]
    fn a_relaunch_within_a_day_of_the_last_pass_skips_it() {
        let (_dir, core) = temp_core();
        let earlier = SystemTime::now() - 2 * HOUR;
        run_maintenance_impl(&core, false, earlier).unwrap();
        assert!(run_if_due(&core, SystemTime::now()).is_none());
    }
}
