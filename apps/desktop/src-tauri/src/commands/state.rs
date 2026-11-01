//! Read-state / star / read-later / archive commands.
//!
//! The `bool` returns are core's "state actually changed" idempotency
//! signal — the frontend query cache skips invalidation on `false`, and
//! this layer mirrors that by emitting [`ArticlesChanged`] only on `true`.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::{ArticleId, FeedId};
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
    let changed =
        run_blocking("mark_read", move || mark_read_impl(&core, article_id, read)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

/// Marks every unread article read — one feed (`Some`) or the whole
/// library (`None`). Returns the ids that changed (ascending; the length is
/// the count), which the UI keeps for an exact undo via [`mark_read_many`].
/// Invalidates only if something changed (the `ArticlesChanged` refresh also
/// updates the unread badges).
#[tauri::command]
#[specta::specta]
pub async fn mark_all_read(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: Option<i64>,
) -> Result<Vec<i64>, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking("mark_all_read", move || mark_all_read_impl(&core, feed_id)).await?;
    if !changed.is_empty() {
        emit_or_log(&app, &ArticlesChanged { feed_id });
    }
    Ok(changed)
}

/// Marks many articles read/unread in one transaction — undo for a
/// mark-all-read, and the batch for bulk actions. Ids that no longer exist
/// are skipped. Returns how many changed; invalidates only if any did.
#[tauri::command]
#[specta::specta]
pub async fn mark_read_many(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_ids: Vec<i64>,
    read: bool,
) -> Result<u64, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking("mark_read_many", move || {
        mark_read_many_impl(&core, &article_ids, read)
    })
    .await?;
    if changed > 0 {
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
    run_blocking("record_opened", move || {
        record_opened_impl(&core, article_id, dwell_ms)
    })
    .await
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
    let changed = run_blocking("set_archived", move || {
        set_archived_impl(&core, article_id, archived)
    })
    .await?;
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
    let changed = run_blocking("set_starred", move || {
        set_starred_impl(&core, article_id, starred)
    })
    .await?;
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
    let changed = run_blocking("set_read_later", move || {
        set_read_later_impl(&core, article_id, read_later)
    })
    .await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

// ------------------------------------------------------------------ impls

fn mark_read_impl(core: &CoreHandle, article_id: i64, read: bool) -> Result<bool, CommandError> {
    Ok(core.mark_read(ArticleId(article_id), read)?)
}

fn mark_all_read_impl(core: &CoreHandle, feed_id: Option<i64>) -> Result<Vec<i64>, CommandError> {
    let changed = core.mark_all_read(feed_id.map(FeedId))?;
    Ok(changed.into_iter().map(|id| id.0).collect())
}

fn mark_read_many_impl(
    core: &CoreHandle,
    article_ids: &[i64],
    read: bool,
) -> Result<u64, CommandError> {
    let ids: Vec<ArticleId> = article_ids.iter().copied().map(ArticleId).collect();
    Ok(core.mark_read_many(&ids, read)?)
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
    fn mark_all_read_flips_every_unread_once() {
        let (_dir, core) = temp_core();
        let a = seed_article(&core, "one");
        let b = seed_article(&core, "two");
        // `a` is already read; only the still-unread `b` should flip.
        assert!(mark_read_impl(&core, a, true).unwrap());

        assert_eq!(
            mark_all_read_impl(&core, None).unwrap(),
            vec![b],
            "only the still-unread article is returned"
        );
        assert!(article_state_of(&core, a).read);
        assert!(article_state_of(&core, b).read);

        // Nothing left unread → a second sweep changes nothing.
        assert_eq!(mark_all_read_impl(&core, None).unwrap(), Vec::<i64>::new());
    }

    #[test]
    fn mark_read_many_undoes_exactly_the_mark_all_read_snapshot() {
        let (_dir, core) = temp_core();
        let a = seed_article(&core, "read-before");
        let b = seed_article(&core, "unread-one");
        let c = seed_article(&core, "unread-two");
        assert!(mark_read_impl(&core, a, true).unwrap());

        let snapshot = mark_all_read_impl(&core, None).unwrap();
        assert_eq!(mark_read_many_impl(&core, &snapshot, false).unwrap(), 2);
        assert!(article_state_of(&core, a).read, "was read before the sweep");
        assert!(!article_state_of(&core, b).read);
        assert!(!article_state_of(&core, c).read);

        // Unknown ids are skipped rather than failing the whole batch.
        assert_eq!(mark_read_many_impl(&core, &[424_242, b], true).unwrap(), 1);
        assert!(article_state_of(&core, b).read);
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
