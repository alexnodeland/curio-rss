//! Upkeep: `curio maintenance`, `curio retention` and `curio db`.

use std::num::NonZeroU32;
use std::process::ExitCode;

use curio_core::MaintenanceOptions;
use curio_core::model::RetentionPolicy;
use serde::Serialize;

use crate::app::App;
use crate::cli::{DbCommand, RetentionArgs};
use crate::output::emit_json;

/// The exit status of a `curio maintenance` run that asked for a reap
/// but deleted nothing because the exported set was unreadable (an
/// unmounted vault): distinct from success, so a scheduled run notices,
/// and from the usage error (2), busy (75) and failure (1) statuses.
const REAP_SKIPPED_STATUS: u8 = 3;

/// What one `curio maintenance` run did, as printed.
#[derive(Debug, Serialize)]
struct MaintenanceView {
    articles_reaped: u64,
    orphans_purged: u64,
    tags_dropped: u64,
    tombstones_pruned: u64,
    fetch_log_trimmed: u64,
    event_files_removed: Vec<String>,
    reap_skipped: Option<String>,
    event_sweep_skipped: Option<String>,
    upkeep_skipped: Option<String>,
}

pub(crate) fn run(app: &App, purge_orphans: bool) -> anyhow::Result<ExitCode> {
    let report = app.core.maintenance(MaintenanceOptions { purge_orphans })?;
    let view = MaintenanceView {
        articles_reaped: report.articles_reaped,
        orphans_purged: report.orphans_purged,
        tags_dropped: report.tags_dropped,
        tombstones_pruned: report.tombstones_pruned,
        fetch_log_trimmed: report.fetch_log_trimmed,
        event_files_removed: report
            .event_files_removed
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        reap_skipped: report.reap_skipped,
        event_sweep_skipped: report.event_sweep_skipped,
        upkeep_skipped: report.upkeep_skipped,
    };
    let status = if view.reap_skipped.is_some() {
        ExitCode::from(REAP_SKIPPED_STATUS)
    } else {
        ExitCode::SUCCESS
    };
    if app.json {
        emit_json(&view)?;
        return Ok(status);
    }
    println!(
        "reaped {} article(s) by retention, purged {} orphan(s)",
        view.articles_reaped, view.orphans_purged
    );
    println!(
        "dropped {} unused tag(s), trimmed {} fetch-log row(s), forgot {} tombstone(s)",
        view.tags_dropped, view.fetch_log_trimmed, view.tombstones_pruned
    );
    println!(
        "removed {} expired event-log file(s)",
        view.event_files_removed.len()
    );
    if let Some(why) = &view.reap_skipped {
        eprintln!("warning: no article was deleted — the exported set is unreadable: {why}");
    }
    if let Some(why) = &view.event_sweep_skipped {
        eprintln!("warning: the event-log sweep did not run: {why}");
    }
    if let Some(why) = &view.upkeep_skipped {
        eprintln!("warning: the database upkeep stopped short (the next run catches up): {why}");
    }
    Ok(status)
}

/// The retention policy, as printed.
#[derive(Debug, Serialize)]
struct RetentionView {
    enabled: bool,
    max_age_days: Option<u32>,
    max_per_feed: Option<u32>,
    keep_unread: bool,
}

pub(crate) fn retention(app: &App, args: &RetentionArgs) -> anyhow::Result<ExitCode> {
    let mut policy = app.core.retention_policy()?;
    let changes =
        args.max_age_days.is_some() || args.max_per_feed.is_some() || args.keep_unread.is_some();
    if let Some(days) = args.max_age_days {
        policy.max_age_days = NonZeroU32::new(days);
    }
    if let Some(count) = args.max_per_feed {
        policy.max_per_feed = NonZeroU32::new(count);
    }
    if let Some(mode) = args.keep_unread {
        policy.keep_unread = mode.as_bool();
    }
    if changes {
        app.core.set_retention_policy(policy)?;
    }
    let view = retention_view(policy);
    if app.json {
        emit_json(&view)?;
        return Ok(ExitCode::SUCCESS);
    }
    if view.enabled {
        let mut limits = Vec::new();
        if let Some(days) = view.max_age_days {
            limits.push(format!("articles first stored over {days} day(s) ago"));
        }
        if let Some(count) = view.max_per_feed {
            limits.push(format!("all but the newest {count} per feed"));
        }
        println!(
            "retention: on — curio maintenance deletes {}",
            limits.join(" and ")
        );
    } else {
        println!("retention: off — every article is kept");
    }
    println!(
        "never deleted: starred, read-later, tagged, exported{}",
        if view.keep_unread { ", unread" } else { "" }
    );
    Ok(ExitCode::SUCCESS)
}

fn retention_view(policy: RetentionPolicy) -> RetentionView {
    RetentionView {
        enabled: policy.is_enabled(),
        max_age_days: policy.max_age_days.map(NonZeroU32::get),
        max_per_feed: policy.max_per_feed.map(NonZeroU32::get),
        keep_unread: policy.keep_unread,
    }
}

#[derive(Debug, Serialize)]
struct RebuildView {
    rebuilt: bool,
}

pub(crate) fn db(app: &App, command: &DbCommand) -> anyhow::Result<ExitCode> {
    let DbCommand::RebuildFts = command;
    app.core.rebuild_search_index()?;
    app.core.fts_integrity_check()?;
    if app.json {
        emit_json(&RebuildView { rebuilt: true })?;
    } else {
        println!("search index rebuilt; it matches the articles table");
    }
    Ok(ExitCode::SUCCESS)
}
