//! Doctor-panel commands: integrity checks, vacuum, stats, event-log
//! retention sweep. All read the storage escape hatch except the sweep,
//! which is facade-owned (single-writer event log — the head must never
//! open its own `EventLog`).

use std::sync::Arc;

use curio_core::CoreHandle;
use tauri::State;

use super::{SharedCore, run_blocking};
use crate::dto::DbStatsDto;
use crate::error::CommandError;

/// `PRAGMA integrity_check` — `Ok(())` or an internal error with detail.
#[tauri::command]
#[specta::specta]
pub async fn integrity_check(core: State<'_, SharedCore>) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || integrity_check_impl(&core)).await
}

/// FTS index ↔ content-table consistency check.
#[tauri::command]
#[specta::specta]
pub async fn fts_integrity_check(core: State<'_, SharedCore>) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || fts_integrity_check_impl(&core)).await
}

/// `VACUUM` — reclaims space after bulk deletes.
#[tauri::command]
#[specta::specta]
pub async fn vacuum_db(core: State<'_, SharedCore>) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || vacuum_db_impl(&core)).await
}

/// Article count, schema version, and database path.
#[tauri::command]
#[specta::specta]
pub async fn db_stats(core: State<'_, SharedCore>) -> Result<DbStatsDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || db_stats_impl(&core)).await
}

/// Deletes event-log files past the ≥90-day retention floor. Returns the
/// removed paths (display form).
#[tauri::command]
#[specta::specta]
pub async fn sweep_event_retention(
    core: State<'_, SharedCore>,
) -> Result<Vec<String>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || sweep_event_retention_impl(&core)).await
}

// ------------------------------------------------------------------ impls

fn integrity_check_impl(core: &CoreHandle) -> Result<(), CommandError> {
    Ok(core.storage().integrity_check()?)
}

fn fts_integrity_check_impl(core: &CoreHandle) -> Result<(), CommandError> {
    Ok(core.storage().fts_integrity_check()?)
}

fn vacuum_db_impl(core: &CoreHandle) -> Result<(), CommandError> {
    Ok(core.storage().vacuum()?)
}

fn db_stats_impl(core: &CoreHandle) -> Result<DbStatsDto, CommandError> {
    Ok(DbStatsDto {
        article_count: core.storage().count_articles()?,
        schema_version: core.storage().db_schema_version()?,
        db_path: core.storage().path().display().to_string(),
    })
}

fn sweep_event_retention_impl(core: &CoreHandle) -> Result<Vec<String>, CommandError> {
    Ok(core
        .sweep_event_retention()?
        .into_iter()
        .map(|path| path.display().to_string())
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};

    #[test]
    fn a_fresh_profile_passes_every_check() {
        let (_dir, core) = temp_core();
        integrity_check_impl(&core).unwrap();
        fts_integrity_check_impl(&core).unwrap();
        vacuum_db_impl(&core).unwrap();
    }

    #[test]
    fn stats_reflect_stored_articles() {
        let (_dir, core) = temp_core();
        seed_article(&core, "counted");
        let stats = db_stats_impl(&core).unwrap();
        assert_eq!(stats.article_count, 1);
        assert!(stats.db_path.ends_with("curio.db"));
        assert!(stats.schema_version >= 1);
    }

    #[test]
    fn retention_sweep_on_a_young_log_removes_nothing() {
        let (_dir, core) = temp_core();
        seed_article(&core, "young"); // forces the events dir into existence
        let removed = sweep_event_retention_impl(&core).unwrap();
        assert!(removed.is_empty(), "today's log is inside the window");
    }
}
