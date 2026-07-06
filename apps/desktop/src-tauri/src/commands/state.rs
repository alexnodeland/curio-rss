//! Read-state / star / read-later / archive commands.
//!
//! The `bool` returns are core's "state actually changed" idempotency
//! signal — the frontend query cache skips invalidation on `false`, and
//! this layer mirrors that by emitting [`ArticlesChanged`] only on `true`.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::ArticleId;
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::error::CommandError;
use crate::events::{ArticlesChanged, emit_or_log};

/// Marks read/unread. Returns whether anything changed.
#[tauri::command]
#[specta::specta]
pub async fn mark_read(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    read: bool,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking(move || mark_read_impl(&core, article_id, read)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

/// Records an open (emits the `article.opened` contract event with the
/// optional dwell time). No invalidation — opening changes no list.
#[tauri::command]
#[specta::specta]
pub async fn record_opened(
    core: State<'_, SharedCore>,
    article_id: i64,
    dwell_ms: Option<u64>,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || record_opened_impl(&core, article_id, dwell_ms)).await
}

/// Archives / unarchives. Returns whether anything changed.
#[tauri::command]
#[specta::specta]
pub async fn set_archived(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    archived: bool,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking(move || set_archived_impl(&core, article_id, archived)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

/// Stars / unstars. Returns whether anything changed.
#[tauri::command]
#[specta::specta]
pub async fn set_starred(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    starred: bool,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking(move || set_starred_impl(&core, article_id, starred)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

/// Adds to / removes from the read-later queue. Returns whether anything
/// changed.
#[tauri::command]
#[specta::specta]
pub async fn set_read_later(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    read_later: bool,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking(move || set_read_later_impl(&core, article_id, read_later)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

// ------------------------------------------------------------------ impls

fn mark_read_impl(core: &CoreHandle, article_id: i64, read: bool) -> Result<bool, CommandError> {
    Ok(core.mark_read(ArticleId(article_id), read)?)
}

fn record_opened_impl(
    core: &CoreHandle,
    article_id: i64,
    dwell_ms: Option<u64>,
) -> Result<(), CommandError> {
    Ok(core.record_opened(ArticleId(article_id), dwell_ms)?)
}

fn set_archived_impl(
    core: &CoreHandle,
    article_id: i64,
    archived: bool,
) -> Result<bool, CommandError> {
    Ok(core.set_archived(ArticleId(article_id), archived)?)
}

fn set_starred_impl(
    core: &CoreHandle,
    article_id: i64,
    starred: bool,
) -> Result<bool, CommandError> {
    Ok(core.set_starred(ArticleId(article_id), starred)?)
}

fn set_read_later_impl(
    core: &CoreHandle,
    article_id: i64,
    read_later: bool,
) -> Result<bool, CommandError> {
    Ok(core.set_read_later(ArticleId(article_id), read_later)?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};
    use crate::error::{ErrorCode, ErrorKind};

    fn article_state_of(core: &CoreHandle, id: i64) -> curio_core::model::ArticleState {
        core.article_state(ArticleId(id)).unwrap()
    }

    #[test]
    fn flag_flips_report_change_and_are_idempotent() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "flags");

        assert!(mark_read_impl(&core, id, true).unwrap());
        assert!(
            !mark_read_impl(&core, id, true).unwrap(),
            "second flip is a no-op"
        );
        assert!(set_starred_impl(&core, id, true).unwrap());
        assert!(set_read_later_impl(&core, id, true).unwrap());
        assert!(set_archived_impl(&core, id, true).unwrap());

        let state = article_state_of(&core, id);
        assert!(state.read && state.starred && state.read_later && state.archived);

        assert!(set_starred_impl(&core, id, false).unwrap());
        assert!(!article_state_of(&core, id).starred);
    }

    #[test]
    fn record_opened_accepts_optional_dwell() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "opened");
        record_opened_impl(&core, id, None).unwrap();
        record_opened_impl(&core, id, Some(1500)).unwrap();
    }

    #[test]
    fn missing_articles_are_user_not_found() {
        let (_dir, core) = temp_core();
        let error = set_starred_impl(&core, 424_242, true).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }
}
