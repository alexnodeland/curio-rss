//! Tag commands. Tags ride the event-sourced write path (`article.tagged`
//! / `article.untagged`); empty or whitespace-only tags are refused at
//! the producer boundary (`StorageError::InvalidTag` → user tier).

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::ArticleId;
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::error::CommandError;
use crate::events::{ArticlesChanged, emit_or_log};

/// Tags an article. Returns whether anything changed (idempotent).
#[tauri::command]
#[specta::specta]
pub async fn tag_article(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    tag: String,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking(move || tag_article_impl(&core, article_id, &tag)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

/// Untags an article. Returns whether anything changed (idempotent).
#[tauri::command]
#[specta::specta]
pub async fn untag_article(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
    tag: String,
) -> Result<bool, CommandError> {
    let core = Arc::clone(core.inner());
    let changed = run_blocking(move || untag_article_impl(&core, article_id, &tag)).await?;
    if changed {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(changed)
}

/// Every known tag name, sorted — the tag-filter sidebar source.
#[tauri::command]
#[specta::specta]
pub async fn list_tags(core: State<'_, SharedCore>) -> Result<Vec<String>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || list_tags_impl(&core)).await
}

// ------------------------------------------------------------------ impls

fn tag_article_impl(core: &CoreHandle, article_id: i64, tag: &str) -> Result<bool, CommandError> {
    Ok(core.tag_article(ArticleId(article_id), tag)?)
}

fn untag_article_impl(core: &CoreHandle, article_id: i64, tag: &str) -> Result<bool, CommandError> {
    Ok(core.untag_article(ArticleId(article_id), tag)?)
}

fn list_tags_impl(core: &CoreHandle) -> Result<Vec<String>, CommandError> {
    Ok(core.storage().list_tags()?)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};
    use crate::error::{ErrorCode, ErrorKind};

    #[test]
    fn tag_untag_and_list_round_trip() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "tagged");
        assert!(tag_article_impl(&core, id, "rust").unwrap());
        assert!(!tag_article_impl(&core, id, "rust").unwrap(), "idempotent");
        assert!(tag_article_impl(&core, id, "reading").unwrap());
        assert_eq!(list_tags_impl(&core).unwrap(), vec!["reading", "rust"]);

        assert!(untag_article_impl(&core, id, "rust").unwrap());
        assert!(!untag_article_impl(&core, id, "rust").unwrap());
    }

    #[test]
    fn whitespace_tags_are_a_recoverable_user_error() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "strict");
        let error = tag_article_impl(&core, id, "   ").unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.recoverable);
    }
}
