//! Podcast-related commands.

use tauri::State;
use uuid::Uuid;

use crate::commands::AppState;
use crate::error::CommandError;

/// Download a podcast episode
#[tauri::command]
pub async fn download_episode(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<String, CommandError> {
    let id = Uuid::parse_str(&article_id)
        .map_err(|e| CommandError::validation(format!("Invalid article ID: {}", e)))?;

    // Get article to find media attachment
    let article = state
        .db
        .get_article(id)?
        .ok_or_else(|| CommandError::not_found("Article not found"))?;

    // Find audio attachment
    let media = state.db.get_media_attachments(id)?;
    let audio = media
        .iter()
        .find(|m| m.mime_type.starts_with("audio/"))
        .ok_or_else(|| CommandError::not_found("No audio attachment found"))?;

    // Create downloads directory
    let downloads_dir = state
        .config_dir
        .join("downloads")
        .join(article.feed_id.to_string());
    std::fs::create_dir_all(&downloads_dir)
        .map_err(|e| CommandError::io(format!("Failed to create downloads dir: {}", e)))?;

    // Download file
    let client = reqwest::Client::new();
    let response = client
        .get(&audio.url)
        .header("User-Agent", "Curio Reader/1.0")
        .send()
        .await
        .map_err(|e| CommandError::io(format!("Download failed: {}", e)))?;

    if !response.status().is_success() {
        return Err(CommandError::io(format!(
            "Download failed: {}",
            response.status()
        )));
    }

    // Determine filename
    let ext = if audio.mime_type.contains("mpeg") {
        "mp3"
    } else if audio.mime_type.contains("ogg") {
        "ogg"
    } else if audio.mime_type.contains("m4a") {
        "m4a"
    } else {
        "mp3"
    };

    let filename = format!("{}.{}", id, ext);
    let file_path = downloads_dir.join(&filename);

    let bytes = response
        .bytes()
        .await
        .map_err(|e| CommandError::io(format!("Failed to read response: {}", e)))?;

    tokio::fs::write(&file_path, &bytes)
        .await
        .map_err(|e| CommandError::io(format!("Failed to write file: {}", e)))?;

    // Update media attachment to mark as downloaded
    state
        .db
        .mark_media_downloaded(audio.id, &file_path.to_string_lossy())?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Delete a downloaded episode
#[tauri::command]
pub async fn delete_download(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<(), CommandError> {
    let id = Uuid::parse_str(&article_id)
        .map_err(|e| CommandError::validation(format!("Invalid article ID: {}", e)))?;

    // Find downloaded media
    let media = state.db.get_media_attachments(id)?;
    let audio = media
        .iter()
        .find(|m| m.is_downloaded && m.mime_type.starts_with("audio/"));

    if let Some(download) = audio {
        if let Some(ref path) = download.local_path {
            // Delete file
            if std::path::Path::new(path).exists() {
                tokio::fs::remove_file(path)
                    .await
                    .map_err(|e| CommandError::io(format!("Failed to delete file: {}", e)))?;
            }
        }

        // Update database
        state.db.mark_media_not_downloaded(download.id)?;
    }

    Ok(())
}

/// Update playback progress for a podcast episode
#[tauri::command]
pub async fn update_playback_progress(
    state: State<'_, AppState>,
    article_id: String,
    progress_seconds: i32,
) -> Result<(), CommandError> {
    let id = Uuid::parse_str(&article_id)
        .map_err(|e| CommandError::validation(format!("Invalid article ID: {}", e)))?;

    state.db.update_podcast_progress(id, progress_seconds)?;

    Ok(())
}

/// Get playback progress for a podcast episode
#[tauri::command]
pub async fn get_playback_progress(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<Option<i32>, CommandError> {
    let id = Uuid::parse_str(&article_id)
        .map_err(|e| CommandError::validation(format!("Invalid article ID: {}", e)))?;

    let article = state.db.get_article(id)?;

    Ok(article.and_then(|a| a.podcast_progress))
}

/// Check if an episode is downloaded
#[tauri::command]
pub async fn is_episode_downloaded(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<bool, CommandError> {
    let id = Uuid::parse_str(&article_id)
        .map_err(|e| CommandError::validation(format!("Invalid article ID: {}", e)))?;

    let media = state.db.get_media_attachments(id)?;
    let downloaded = media
        .iter()
        .any(|m| m.is_downloaded && m.mime_type.starts_with("audio/"));

    Ok(downloaded)
}
