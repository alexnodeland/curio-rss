//! Article commands: backend-filtered listing (keyset-paginated), full
//! reads, counts, search, delete.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::ArticleId;
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::dto::{
    ArticleDto, ArticleStateDto, ArticleSummaryDto, ListArticlesDto, UnreadCountsDto,
};
use crate::error::CommandError;
use crate::events::{ArticlesChanged, emit_or_log};

/// Lists article summaries. Filters and order are backend-owned — the
/// frontend passes them through and never re-filters or re-sorts.
#[tauri::command]
#[specta::specta]
pub async fn list_articles(
    core: State<'_, SharedCore>,
    params: ListArticlesDto,
) -> Result<Vec<ArticleSummaryDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || list_articles_impl(&core, params)).await
}

/// One full article (sanitized HTML included), if it exists.
#[tauri::command]
#[specta::specta]
pub async fn get_article(
    core: State<'_, SharedCore>,
    article_id: i64,
) -> Result<Option<ArticleDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_article_impl(&core, article_id)).await
}

/// The read/starred/read-later/archived flag projection.
#[tauri::command]
#[specta::specta]
pub async fn get_article_state(
    core: State<'_, SharedCore>,
    article_id: i64,
) -> Result<ArticleStateDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_article_state_impl(&core, article_id)).await
}

/// The article's tags, sorted.
#[tauri::command]
#[specta::specta]
pub async fn get_article_tags(
    core: State<'_, SharedCore>,
    article_id: i64,
) -> Result<Vec<String>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_article_tags_impl(&core, article_id)).await
}

/// Backend-owned unread counts (total + per feed).
#[tauri::command]
#[specta::specta]
pub async fn get_unread_counts(
    core: State<'_, SharedCore>,
) -> Result<UnreadCountsDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_unread_counts_impl(&core)).await
}

/// Total stored articles.
#[tauri::command]
#[specta::specta]
pub async fn count_articles(core: State<'_, SharedCore>) -> Result<u64, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || count_articles_impl(&core)).await
}

/// Deletes an article (cascades state/tags/FTS).
#[tauri::command]
#[specta::specta]
pub async fn delete_article(
    app: AppHandle,
    core: State<'_, SharedCore>,
    article_id: i64,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || delete_article_impl(&core, article_id)).await?;
    emit_or_log(&app, &ArticlesChanged { feed_id: None });
    Ok(())
}

/// FTS5 search over title + extracted text (query is phrase-escaped in
/// core — hostile input cannot break the MATCH grammar).
#[tauri::command]
#[specta::specta]
pub async fn search_articles(
    core: State<'_, SharedCore>,
    query: String,
    limit: u32,
) -> Result<Vec<ArticleSummaryDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || search_articles_impl(&core, &query, limit)).await
}

// ------------------------------------------------------------------ impls

fn list_articles_impl(
    core: &CoreHandle,
    params: ListArticlesDto,
) -> Result<Vec<ArticleSummaryDto>, CommandError> {
    Ok(core
        .list_articles(params.into())?
        .into_iter()
        .map(ArticleSummaryDto::from)
        .collect())
}

fn get_article_impl(
    core: &CoreHandle,
    article_id: i64,
) -> Result<Option<ArticleDto>, CommandError> {
    Ok(core
        .get_article(ArticleId(article_id))?
        .map(ArticleDto::from))
}

fn get_article_state_impl(
    core: &CoreHandle,
    article_id: i64,
) -> Result<ArticleStateDto, CommandError> {
    Ok(core.article_state(ArticleId(article_id))?.into())
}

fn get_article_tags_impl(core: &CoreHandle, article_id: i64) -> Result<Vec<String>, CommandError> {
    Ok(core.article_tags(ArticleId(article_id))?)
}

fn get_unread_counts_impl(core: &CoreHandle) -> Result<UnreadCountsDto, CommandError> {
    Ok(core.unread_counts()?.into())
}

fn count_articles_impl(core: &CoreHandle) -> Result<u64, CommandError> {
    Ok(core.storage().count_articles()?)
}

fn delete_article_impl(core: &CoreHandle, article_id: i64) -> Result<(), CommandError> {
    Ok(core.storage().delete_article(ArticleId(article_id))?)
}

fn search_articles_impl(
    core: &CoreHandle,
    query: &str,
    limit: u32,
) -> Result<Vec<ArticleSummaryDto>, CommandError> {
    Ok(core
        .search_articles(query, limit)?
        .into_iter()
        .map(ArticleSummaryDto::from)
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};
    use crate::error::{ErrorCode, ErrorKind};

    fn default_params() -> ListArticlesDto {
        ListArticlesDto {
            feed_id: None,
            before: None,
            limit: 50,
            read: None,
            starred: None,
            read_later: None,
            archived: None,
            tag: None,
            feed_tag: None,
        }
    }

    #[test]
    fn list_is_newest_first_with_keyset_pagination() {
        let (_dir, core) = temp_core();
        let first = seed_article(&core, "alpha");
        let second = seed_article(&core, "beta");
        assert!(second > first);

        let page = list_articles_impl(&core, default_params()).unwrap();
        assert_eq!(page.len(), 2);
        assert_eq!(page[0].id, second, "newest row first");
        assert!(page[0].published_at.is_none());

        let next = list_articles_impl(
            &core,
            ListArticlesDto {
                before: Some(second),
                ..default_params()
            },
        )
        .unwrap();
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].id, first);
    }

    #[test]
    fn read_filter_is_applied_backend_side() {
        let (_dir, core) = temp_core();
        let read_id = seed_article(&core, "seen");
        let unread_id = seed_article(&core, "fresh");
        core.mark_read(ArticleId(read_id), true).unwrap();

        let unread = list_articles_impl(
            &core,
            ListArticlesDto {
                read: Some(false),
                ..default_params()
            },
        )
        .unwrap();
        assert_eq!(unread.len(), 1);
        assert_eq!(unread[0].id, unread_id);

        let counts = get_unread_counts_impl(&core).unwrap();
        assert_eq!(counts.total, 1);
        assert!(
            counts.by_feed.is_empty(),
            "feedless bucket stays out of by_feed"
        );
    }

    #[test]
    fn full_article_carries_content_and_summary_does_not() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "reader");
        let full = get_article_impl(&core, id).unwrap().unwrap();
        assert_eq!(full.content_html, "<p>reader</p>");
        assert!(!full.curio_id.is_empty());
        assert!(get_article_impl(&core, id + 999).unwrap().is_none());
    }

    #[test]
    fn state_defaults_are_all_false_and_search_finds_titles() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "quenched");
        let state = get_article_state_impl(&core, id).unwrap();
        assert_eq!(state, crate::dto::ArticleStateDto::default());

        let hits = search_articles_impl(&core, "quenched", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, id);
        assert!(search_articles_impl(&core, "   ", 10).unwrap().is_empty());
    }

    #[test]
    fn delete_cascades_and_missing_rows_are_user_errors() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "doomed");
        assert_eq!(count_articles_impl(&core).unwrap(), 1);
        delete_article_impl(&core, id).unwrap();
        assert_eq!(count_articles_impl(&core).unwrap(), 0);

        let error = delete_article_impl(&core, id).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }
}
