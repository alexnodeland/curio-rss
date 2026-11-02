//! Read-state / star / read-later / archive commands.
//!
//! The `bool` returns are core's "state actually changed" idempotency
//! signal — the frontend query cache skips invalidation on `false`, and
//! this layer mirrors that by emitting [`ArticlesChanged`] only on `true`.
//! Every flip here is a pure flag change, so the event names the articles,
//! the flag and its new value (`ArticlesChanged::flag`): the cache patches
//! those rows in place instead of refetching everything (#135).

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::{ArticleId, FeedId, ReadProgress};
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::dto::ArticleFlagDto;
use crate::error::CommandError;
use crate::events::{ArticlesChanged, FlagField, ReadProgressChanged, emit_or_log};

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
        emit_or_log(
            &app,
            &ArticlesChanged::flag(None, FlagField::Read, vec![article_id], read),
        );
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
        emit_or_log(
            &app,
            &ArticlesChanged::flag(feed_id, FlagField::Read, changed.clone(), true),
        );
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
    let ids = article_ids.clone();
    let changed = run_blocking("mark_read_many", move || {
        mark_read_many_impl(&core, &ids, read)
    })
    .await?;
    if changed > 0 {
        // Every id that exists now holds `read` (the unchanged ones already
        // did), so the whole request is the patch.
        emit_or_log(
            &app,
            &ArticlesChanged::flag(None, FlagField::Read, article_ids, read),
        );
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

/// Stores how far through the body the reader is, in thousandths of its
/// scroll extent (`0`–`1000`; anything past `1000` is the end). Returns
/// whether the stored position moved, and only then emits
/// [`ReadProgressChanged`] — that article's state alone, no list refresh.
#[tauri::command]
#[specta::specta]
pub async fn set_read_progress(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    read_permille: u16,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking("set_read_progress", move || {
        set_read_progress_impl(&core, article_id, read_permille)
    })
    .await?;
    if changed {
        emit_or_log(&app, &ReadProgressChanged { article_id });
    }
    Ok(changed)
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
        emit_or_log(
            &app,
            &ArticlesChanged::flag(None, FlagField::Archived, vec![article_id], archived),
        );
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
        emit_or_log(
            &app,
            &ArticlesChanged::flag(None, FlagField::Starred, vec![article_id], starred),
        );
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
        emit_or_log(
            &app,
            &ArticlesChanged::flag(None, FlagField::ReadLater, vec![article_id], read_later),
        );
    }
    Ok(changed)
}

/// Sets one flag on many articles in one transaction — the batch behind
/// the bulk actions (one IPC call, one commit, each changed article's event
/// staged inside it). Ids that no longer exist are skipped. Returns the ids
/// that changed, in the order given, which the UI keeps for an exact undo
/// (the same call with the opposite `value`). Invalidates only if any
/// changed.
#[tauri::command]
#[specta::specta]
pub async fn set_flag_many(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_ids: Vec<i64>,
    flag: ArticleFlagDto,
    value: bool,
) -> Result<Vec<i64>, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking("set_flag_many", move || {
        set_flag_many_impl(&core, &article_ids, flag, value)
    })
    .await?;
    if !changed.is_empty() {
        emit_or_log(
            &app,
            &ArticlesChanged::flag(None, flag.into(), changed.clone(), value),
        );
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

fn set_flag_many_impl(
    core: &CoreHandle,
    article_ids: &[i64],
    flag: ArticleFlagDto,
    value: bool,
) -> Result<Vec<i64>, CommandError> {
    let ids: Vec<ArticleId> = article_ids.iter().copied().map(ArticleId).collect();
    let changed = core.set_flag_many(&ids, flag.into(), value)?;
    Ok(changed.into_iter().map(|id| id.0).collect())
}

fn record_opened_impl(
    core: &CoreHandle,
    article_id: i64,
    dwell_ms: Option<u64>,
) -> Result<(), CommandError> {
    Ok(core.record_opened(ArticleId(article_id), dwell_ms)?)
}

fn set_read_progress_impl(
    core: &CoreHandle,
    article_id: i64,
    read_permille: u16,
) -> Result<bool, CommandError> {
    let progress = ReadProgress::from_permille(read_permille).unwrap_or(ReadProgress::END);
    Ok(core.set_read_progress(ArticleId(article_id), progress)?)
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
    fn set_flag_many_maps_each_flag_and_returns_the_changed_ids() {
        let (_dir, core) = temp_core();
        let a = seed_article(&core, "bulk-a");
        let b = seed_article(&core, "bulk-b");
        assert!(set_starred_impl(&core, a, true).unwrap());

        let ids = [a, 424_242, b];
        assert_eq!(
            set_flag_many_impl(&core, &ids, ArticleFlagDto::Starred, true).unwrap(),
            [b],
            "a was starred already; the unknown id is skipped"
        );
        assert_eq!(
            set_flag_many_impl(&core, &ids, ArticleFlagDto::Read, true).unwrap(),
            [a, b]
        );
        assert_eq!(
            set_flag_many_impl(&core, &ids, ArticleFlagDto::ReadLater, true).unwrap(),
            [a, b]
        );
        assert_eq!(
            set_flag_many_impl(&core, &ids, ArticleFlagDto::Archived, true).unwrap(),
            [a, b]
        );
        let state = article_state_of(&core, b);
        assert!(state.read && state.starred && state.read_later && state.archived);

        // The undo: the changed ids with the opposite value.
        assert_eq!(
            set_flag_many_impl(&core, &[a, b], ArticleFlagDto::Archived, false).unwrap(),
            [a, b]
        );
        assert!(!article_state_of(&core, a).archived);
        assert_eq!(
            set_flag_many_impl(&core, &[], ArticleFlagDto::Read, false).unwrap(),
            [] as [i64; 0]
        );
    }

    #[test]
    fn record_opened_accepts_optional_dwell() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "opened");
        record_opened_impl(&core, id, None).unwrap();
        record_opened_impl(&core, id, Some(1500)).unwrap();
    }

    #[test]
    fn read_progress_reports_change_clamps_and_round_trips_through_the_dto() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "progress");

        assert!(set_read_progress_impl(&core, id, 457).unwrap());
        assert!(
            !set_read_progress_impl(&core, id, 457).unwrap(),
            "an unmoved position is a no-op (no event)"
        );
        let dto = crate::dto::ArticleStateDto::from(article_state_of(&core, id));
        assert_eq!(dto.read_permille, 457);

        // Past the end is the end, not a failed save.
        assert!(set_read_progress_impl(&core, id, 7_000).unwrap());
        assert_eq!(article_state_of(&core, id).progress, ReadProgress::END);
        assert!(set_read_progress_impl(&core, id, 0).unwrap());
        assert_eq!(article_state_of(&core, id).progress, ReadProgress::START);

        let error = set_read_progress_impl(&core, 424_242, 500).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn missing_articles_are_user_not_found() {
        let (_dir, core) = temp_core();
        let error = set_starred_impl(&core, 424_242, true).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }
}
