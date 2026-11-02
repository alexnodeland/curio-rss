//! Article retention and the routine maintenance pass (#137): the policy
//! Settings → Data edits, and the doctor's "Run maintenance" / "Purge
//! orphaned articles" actions. The same pass also runs on its own at
//! launch and about daily ([`crate::maintenance_scheduler`]); both record
//! when it last ran, so a manual pass also resets the daily clock.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curio_core::{CoreHandle, MaintenanceOptions};
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::dto::{MaintenanceReportDto, RetentionPolicyDto};
use crate::error::CommandError;
use crate::events::{ArticlesChanged, FeedsChanged, emit_or_log};

/// The wall-clock time (Unix seconds) the last maintenance pass started —
/// the scheduler's daily clock, persisted so a relaunch remembers it. The
/// start (the `now` a caller takes before the pass) is recorded, not the
/// finish; a pass takes seconds, which a daily clock never notices.
pub(crate) const LAST_RUN_KEY: &str = "ui.maintenance.last-at";

/// The persisted article-retention policy (off until one is set).
#[tauri::command]
#[specta::specta]
pub async fn retention_policy(
    core: State<'_, SharedCore>,
) -> Result<RetentionPolicyDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking("retention_policy", move || retention_policy_impl(&core)).await
}

/// Stores the article-retention policy and returns it as stored (a `0`
/// limit reads back as none). Nothing is deleted until the next
/// maintenance pass.
#[tauri::command]
#[specta::specta]
pub async fn set_retention_policy(
    core: State<'_, SharedCore>,
    policy: RetentionPolicyDto,
) -> Result<RetentionPolicyDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking("set_retention_policy", move || {
        set_retention_policy_impl(&core, policy)
    })
    .await
}

/// Runs the maintenance pass now — retention, unused tags, fetch log,
/// index upkeep, expired event logs — and, with `purge_orphans`, deletes
/// the unprotected articles of removed feeds. Emits the invalidations
/// when anything a list shows went away.
#[tauri::command]
#[specta::specta]
pub async fn run_maintenance(
    app: AppHandle,
    core: State<'_, SharedCore>,
    purge_orphans: bool,
) -> Result<MaintenanceReportDto, CommandError> {
    let core = Arc::clone(core.inner());
    let report = run_blocking("run_maintenance", move || {
        run_maintenance_impl(&core, purge_orphans, SystemTime::now())
    })
    .await?;
    emit_after_maintenance(&app, &report);
    Ok(report)
}

// ------------------------------------------------------------------ impls

fn retention_policy_impl(core: &CoreHandle) -> Result<RetentionPolicyDto, CommandError> {
    Ok(core.retention_policy()?.into())
}

fn set_retention_policy_impl(
    core: &CoreHandle,
    policy: RetentionPolicyDto,
) -> Result<RetentionPolicyDto, CommandError> {
    core.set_retention_policy(policy.into())?;
    retention_policy_impl(core)
}

/// One maintenance pass, then `now` — the time the caller took just before
/// it — recorded under [`LAST_RUN_KEY`] as when it ran (best-effort: a failed write only means the next
/// scheduled pass comes sooner).
pub(crate) fn run_maintenance_impl(
    core: &CoreHandle,
    purge_orphans: bool,
    now: SystemTime,
) -> Result<MaintenanceReportDto, CommandError> {
    let report = core.maintenance(MaintenanceOptions { purge_orphans })?;
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    if let Err(error) = core.set_setting(LAST_RUN_KEY, &secs.to_string()) {
        tracing::warn!(%error, "could not record the maintenance time");
    }
    Ok(report.into())
}

/// When the last maintenance pass started, if one was recorded (and the
/// value parses).
pub(crate) fn last_run(core: &CoreHandle) -> Option<SystemTime> {
    core.get_setting(LAST_RUN_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|secs| UNIX_EPOCH + Duration::from_secs(secs))
}

/// What a pass's report means for the UI's caches: deleted articles move
/// lists, counts and the feed tree; dropped tags only the tag lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Invalidation {
    /// Emit `FeedsChanged` (unread counts per feed moved).
    pub feeds: bool,
    /// Emit `ArticlesChanged` (rows or tags went away).
    pub articles: bool,
}

/// Which invalidations `report` calls for.
pub(crate) fn invalidation(report: &MaintenanceReportDto) -> Invalidation {
    let deleted = report.articles_reaped + report.orphans_purged > 0;
    Invalidation {
        feeds: deleted,
        articles: deleted || report.tags_dropped > 0,
    }
}

/// Emits the invalidations `report` calls for (nothing when the pass only
/// tidied bookkeeping no view shows).
pub(crate) fn emit_after_maintenance(app: &AppHandle, report: &MaintenanceReportDto) {
    let Invalidation { feeds, articles } = invalidation(report);
    if feeds {
        emit_or_log(app, &FeedsChanged);
    }
    if articles {
        emit_or_log(app, &ArticlesChanged::rows(None));
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use curio_core::model::{ArticleContent, NewArticle, NewFeed};
    use curio_types::Timestamp;

    use super::*;
    use crate::commands::testutil::temp_core;

    /// Subscribes to a (never fetched) feed and stores `titles` in it.
    fn feed_with_articles(core: &CoreHandle, titles: &[&str]) -> curio_core::model::FeedId {
        let feed = core
            .add_feed(NewFeed {
                url: "https://example.test/feed.xml".to_owned(),
                title: None,
                tags: Vec::new(),
            })
            .unwrap();
        let articles = titles
            .iter()
            .map(|title| NewArticle {
                feed_id: Some(feed.id),
                dedupe_key: format!("test:{title}"),
                title: (*title).to_owned(),
                source_url: format!("https://example.test/{title}"),
                author: None,
                published_at: None,
                content: ArticleContent {
                    html: format!("<p>{title}</p>"),
                    text: (*title).to_owned(),
                },
                lang: None,
                word_count: Some(1),
                source_updated_at: Some(Timestamp::now()),
                lead_image: None,
            })
            .collect();
        core.storage().upsert_articles(articles).unwrap();
        feed.id
    }

    #[test]
    fn the_policy_starts_off_and_round_trips_with_zero_meaning_no_limit() {
        let (_dir, core) = temp_core();
        let initial = retention_policy_impl(&core).unwrap();
        assert_eq!(
            initial,
            RetentionPolicyDto {
                max_age_days: None,
                max_per_feed: None,
                keep_unread: true,
            }
        );

        let stored = set_retention_policy_impl(
            &core,
            RetentionPolicyDto {
                max_age_days: Some(90),
                max_per_feed: Some(0),
                keep_unread: false,
            },
        )
        .unwrap();
        assert_eq!(
            stored,
            RetentionPolicyDto {
                max_age_days: Some(90),
                max_per_feed: None,
                keep_unread: false,
            }
        );
        assert_eq!(retention_policy_impl(&core).unwrap(), stored);
        // The engine reads the same policy the CLI's `curio retention` shows.
        assert_eq!(
            core.retention_policy()
                .unwrap()
                .max_age_days
                .map(std::num::NonZeroU32::get),
            Some(90)
        );
    }

    #[test]
    fn a_pass_applies_the_policy_and_records_when_it_ran() {
        let (_dir, core) = temp_core();
        feed_with_articles(&core, &["old", "new"]);
        set_retention_policy_impl(
            &core,
            RetentionPolicyDto {
                max_age_days: None,
                max_per_feed: Some(1),
                keep_unread: false,
            },
        )
        .unwrap();
        assert!(last_run(&core).is_none());

        let at = UNIX_EPOCH + Duration::from_hours(500_000);
        let report = run_maintenance_impl(&core, false, at).unwrap();

        assert_eq!(report.articles_reaped, 1);
        assert_eq!(report.orphans_purged, 0);
        assert_eq!(core.count_articles().unwrap(), 1);
        assert_eq!(last_run(&core), Some(at));
        assert_eq!(
            invalidation(&report),
            Invalidation {
                feeds: true,
                articles: true
            }
        );
        core.fts_integrity_check().unwrap();
    }

    #[test]
    fn orphans_are_purged_only_on_request() {
        let (_dir, core) = temp_core();
        let feed = feed_with_articles(&core, &["left-behind"]);
        core.remove_feed(feed).unwrap();
        assert_eq!(core.count_articles().unwrap(), 1, "removal keeps articles");

        let routine = run_maintenance_impl(&core, false, SystemTime::now()).unwrap();
        assert_eq!(routine.orphans_purged, 0);
        assert_eq!(core.count_articles().unwrap(), 1);
        assert_eq!(
            invalidation(&routine),
            Invalidation {
                feeds: false,
                articles: false
            },
            "a pass that deleted nothing invalidates nothing"
        );

        let purge = run_maintenance_impl(&core, true, SystemTime::now()).unwrap();
        assert_eq!(purge.orphans_purged, 1);
        assert_eq!(core.count_articles().unwrap(), 0);
        assert!(invalidation(&purge).feeds);
    }

    #[test]
    fn dropped_tags_only_invalidate_articles() {
        let report = MaintenanceReportDto {
            articles_reaped: 0,
            orphans_purged: 0,
            tags_dropped: 2,
            fetch_log_trimmed: 9,
            event_files_removed: 1,
            reap_skipped: None,
            event_sweep_skipped: None,
            upkeep_skipped: None,
        };
        assert_eq!(
            invalidation(&report),
            Invalidation {
                feeds: false,
                articles: true
            }
        );
    }

    #[test]
    fn an_unparseable_last_run_reads_as_never() {
        let (_dir, core) = temp_core();
        core.set_setting(LAST_RUN_KEY, "yesterday").unwrap();
        assert!(last_run(&core).is_none());
    }
}
