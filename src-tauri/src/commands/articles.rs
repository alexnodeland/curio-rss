//! Article management commands.

use tauri::State;
use uuid::Uuid;

use crate::commands::AppState;
use crate::core::models::{Article, ArticleFilter, ArticlePage, Pagination};
use crate::error::CommandError;

/// Get articles with filtering and pagination
#[tauri::command]
pub async fn get_articles(
    state: State<'_, AppState>,
    filter: ArticleFilter,
    pagination: Pagination,
) -> Result<ArticlePage, CommandError> {
    let page = state.db.get_articles(&filter, &pagination)?;
    Ok(page)
}

/// Get a single article by ID
#[tauri::command]
pub async fn get_article(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<Article, CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    state
        .db
        .get_article(id)?
        .ok_or_else(|| CommandError::not_found("Article not found"))
}

/// Mark articles as read
#[tauri::command]
pub async fn mark_read(
    state: State<'_, AppState>,
    article_ids: Vec<String>,
) -> Result<(), CommandError> {
    let ids: Result<Vec<Uuid>, _> = article_ids.iter().map(|s| Uuid::parse_str(s)).collect();
    let ids = ids.map_err(|_| CommandError::validation("Invalid article ID"))?;

    state.db.mark_read(&ids)?;
    Ok(())
}

/// Mark articles as unread
#[tauri::command]
pub async fn mark_unread(
    state: State<'_, AppState>,
    article_ids: Vec<String>,
) -> Result<(), CommandError> {
    let ids: Result<Vec<Uuid>, _> = article_ids.iter().map(|s| Uuid::parse_str(s)).collect();
    let ids = ids.map_err(|_| CommandError::validation("Invalid article ID"))?;

    state.db.mark_unread(&ids)?;
    Ok(())
}

/// Toggle starred status for an article
#[tauri::command]
pub async fn toggle_starred(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<bool, CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    let is_starred = state.db.toggle_starred(id)?;
    Ok(is_starred)
}

/// Search articles
#[tauri::command]
pub async fn search_articles(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i32>,
) -> Result<Vec<Article>, CommandError> {
    if query.trim().is_empty() {
        return Err(CommandError::validation("Search query cannot be empty"));
    }

    let articles = state.db.search_articles(&query, limit.unwrap_or(50))?;
    Ok(articles)
}

/// Add article to read later queue
#[tauri::command]
pub async fn add_to_read_later(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<i32, CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    let position = state.db.add_to_read_later(id)?;
    Ok(position)
}

/// Remove article from read later queue
#[tauri::command]
pub async fn remove_from_read_later(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<(), CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    state.db.remove_from_read_later(id)?;
    Ok(())
}

/// Get all articles in read later queue
#[tauri::command]
pub async fn get_read_later(state: State<'_, AppState>) -> Result<Vec<Article>, CommandError> {
    let articles = state.db.get_read_later()?;
    Ok(articles)
}

/// Reorder read later queue
#[tauri::command]
pub async fn reorder_read_later(
    state: State<'_, AppState>,
    article_ids: Vec<String>,
) -> Result<(), CommandError> {
    let ids: Result<Vec<Uuid>, _> = article_ids.iter().map(|s| Uuid::parse_str(s)).collect();
    let ids = ids.map_err(|_| CommandError::validation("Invalid article ID"))?;

    state.db.reorder_read_later(&ids)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;

    // Integration tests would use a mock state
}
