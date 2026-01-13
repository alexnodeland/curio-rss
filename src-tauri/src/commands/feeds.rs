//! Feed management commands.

use tauri::State;
use uuid::Uuid;

use crate::commands::AppState;
use crate::core::feeds::{detect_view_mode, parse_feed, validate_feed_url};
use crate::core::models::{Feed, FeedUpdate, RefreshResult, ViewMode};
use crate::error::CommandError;

/// Add a new feed
#[tauri::command]
pub async fn add_feed(
    state: State<'_, AppState>,
    url: String,
    folder_id: Option<String>,
    tags: Option<Vec<String>>,
) -> Result<Feed, CommandError> {
    // Validate URL
    let validated_url = validate_feed_url(&url)?;

    // Check for duplicate
    if state.db.get_feed_by_url(&validated_url)?.is_some() {
        return Err(CommandError::validation("Feed already exists"));
    }

    // Detect view mode
    let view_mode = detect_view_mode(&validated_url);

    // Create feed (we'll fetch metadata on first refresh)
    let mut feed = Feed::new(&validated_url, &validated_url);
    feed.view_mode = view_mode;

    if let Some(ref folder_str) = folder_id {
        feed.folder_id = Uuid::parse_str(folder_str).ok();
    }

    if let Some(ref t) = tags {
        feed.tags = t.clone();
    }

    // Insert into database
    state.db.insert_feed(&feed)?;

    Ok(feed)
}

/// Get all feeds
#[tauri::command]
pub async fn get_feeds(state: State<'_, AppState>) -> Result<Vec<Feed>, CommandError> {
    let feeds = state.db.get_all_feeds()?;
    Ok(feeds)
}

/// Get a single feed by ID
#[tauri::command]
pub async fn get_feed(
    state: State<'_, AppState>,
    feed_id: String,
) -> Result<Feed, CommandError> {
    let id = Uuid::parse_str(&feed_id).map_err(|_| CommandError::validation("Invalid feed ID"))?;

    state
        .db
        .get_feed(id)?
        .ok_or_else(|| CommandError::not_found("Feed not found"))
}

/// Update a feed
#[tauri::command]
pub async fn update_feed(
    state: State<'_, AppState>,
    feed_id: String,
    updates: FeedUpdate,
) -> Result<Feed, CommandError> {
    let id = Uuid::parse_str(&feed_id).map_err(|_| CommandError::validation("Invalid feed ID"))?;

    // Verify feed exists
    if state.db.get_feed(id)?.is_none() {
        return Err(CommandError::not_found("Feed not found"));
    }

    state.db.update_feed(id, &updates)?;

    state
        .db
        .get_feed(id)?
        .ok_or_else(|| CommandError::not_found("Feed not found"))
}

/// Delete a feed
#[tauri::command]
pub async fn delete_feed(
    state: State<'_, AppState>,
    feed_id: String,
) -> Result<(), CommandError> {
    let id = Uuid::parse_str(&feed_id).map_err(|_| CommandError::validation("Invalid feed ID"))?;

    state.db.delete_feed(id)?;
    Ok(())
}

/// Refresh a single feed
#[tauri::command]
pub async fn refresh_feed(
    state: State<'_, AppState>,
    feed_id: String,
) -> Result<RefreshResult, CommandError> {
    let id = Uuid::parse_str(&feed_id).map_err(|_| CommandError::validation("Invalid feed ID"))?;

    let feed = state
        .db
        .get_feed(id)?
        .ok_or_else(|| CommandError::not_found("Feed not found"))?;

    // Fetch feed content
    let client = reqwest::Client::new();
    let mut request = client.get(&feed.url);

    // Add conditional headers if available
    if let Some(ref etag) = feed.etag {
        request = request.header("If-None-Match", etag);
    }
    if let Some(ref last_modified) = feed.last_modified {
        request = request.header("If-Modified-Since", last_modified);
    }

    let response = request
        .header("User-Agent", "Curio Reader/0.1 (RSS Desktop Client)")
        .send()
        .await
        .map_err(|e| CommandError::internal(e))?;

    // Handle 304 Not Modified
    if response.status() == reqwest::StatusCode::NOT_MODIFIED {
        state.db.update_feed_fetch(id, feed.etag.as_deref(), feed.last_modified.as_deref(), None)?;

        return Ok(RefreshResult {
            feed_id: id,
            new_count: 0,
            updated_count: 0,
            unchanged: true,
            error: None,
        });
    }

    if !response.status().is_success() {
        let error_msg = format!("HTTP {}", response.status());
        state.db.update_feed_fetch(id, None, None, Some(&error_msg))?;

        return Ok(RefreshResult {
            feed_id: id,
            new_count: 0,
            updated_count: 0,
            unchanged: false,
            error: Some(error_msg),
        });
    }

    // Get new ETag and Last-Modified
    let new_etag = response
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let new_last_modified = response
        .headers()
        .get("last-modified")
        .and_then(|v| v.to_str().ok())
        .map(String::from);

    // Parse feed content
    let content = response.text().await.map_err(|e| CommandError::internal(e))?;
    let parsed = parse_feed(&content, id).map_err(|e| CommandError::internal(e))?;

    // Update feed metadata if changed
    if feed.title != parsed.title || feed.description != parsed.description {
        let update = FeedUpdate {
            title: Some(parsed.title.clone()),
            description: parsed.description.clone(),
            site_url: parsed.site_url.clone(),
            icon_url: parsed.icon_url.clone(),
            ..Default::default()
        };
        state.db.update_feed(id, &update)?;
    }

    // Upsert articles
    let (new_count, updated_count) = state.db.upsert_articles(&parsed.articles)?;

    // Update fetch metadata
    state.db.update_feed_fetch(id, new_etag.as_deref(), new_last_modified.as_deref(), None)?;

    Ok(RefreshResult {
        feed_id: id,
        new_count,
        updated_count,
        unchanged: new_count == 0 && updated_count == 0,
        error: None,
    })
}

/// Refresh all feeds
#[tauri::command]
pub async fn refresh_all_feeds(
    state: State<'_, AppState>,
) -> Result<Vec<RefreshResult>, CommandError> {
    let feeds = state.db.get_all_feeds()?;
    let mut results = Vec::new();

    // TODO: Parallel refresh with concurrency control
    for feed in feeds {
        match refresh_feed(state.clone(), feed.id.to_string()).await {
            Ok(result) => results.push(result),
            Err(e) => {
                results.push(RefreshResult {
                    feed_id: feed.id,
                    new_count: 0,
                    updated_count: 0,
                    unchanged: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would go here
    // For now, the core logic is tested in the storage module
}
