//! Export commands for articles.

use std::path::PathBuf;
use tauri::State;
use uuid::Uuid;

use crate::commands::AppState;
use crate::core::content::{article_to_markdown, generate_filename};
use crate::error::CommandError;

/// Export an article to Markdown format
#[tauri::command]
pub async fn export_to_markdown(
    state: State<'_, AppState>,
    article_id: String,
    destination_path: Option<String>,
) -> Result<String, CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    let article = state
        .db
        .get_article(id)?
        .ok_or_else(|| CommandError::not_found("Article not found"))?;

    let settings = state.settings.read().await;
    let export_settings = &settings.export_settings;

    let markdown = article_to_markdown(&article, export_settings);

    // If destination path provided, write to file
    if let Some(dest) = destination_path {
        let path = PathBuf::from(&dest);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| CommandError::io(format!("Failed to create directory: {}", e)))?;
        }

        std::fs::write(&path, &markdown)
            .map_err(|e| CommandError::io(format!("Failed to write file: {}", e)))?;

        Ok(dest)
    } else {
        // Return the markdown content
        Ok(markdown)
    }
}

/// Copy article as Markdown to clipboard
#[tauri::command]
pub async fn copy_as_markdown(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<String, CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    let article = state
        .db
        .get_article(id)?
        .ok_or_else(|| CommandError::not_found("Article not found"))?;

    let settings = state.settings.read().await;
    let export_settings = &settings.export_settings;

    let markdown = article_to_markdown(&article, export_settings);

    // Note: Actual clipboard access requires tauri-plugin-clipboard
    // For now, we return the markdown and let the frontend handle clipboard
    Ok(markdown)
}

/// Generate a filename for an article export
#[tauri::command]
pub async fn generate_export_filename(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<String, CommandError> {
    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    let article = state
        .db
        .get_article(id)?
        .ok_or_else(|| CommandError::not_found("Article not found"))?;

    let settings = state.settings.read().await;
    let filename = generate_filename(&article, &settings.export_settings.filename_template);

    Ok(format!("{}.md", filename))
}

/// Export to Obsidian vault
#[tauri::command]
pub async fn export_to_obsidian(
    state: State<'_, AppState>,
    article_id: String,
) -> Result<String, CommandError> {
    let settings = state.settings.read().await;

    let vault_path = settings
        .obsidian_vault_path
        .as_ref()
        .ok_or_else(|| CommandError::validation("Obsidian vault path not configured"))?;

    let id =
        Uuid::parse_str(&article_id).map_err(|_| CommandError::validation("Invalid article ID"))?;

    let article = state
        .db
        .get_article(id)?
        .ok_or_else(|| CommandError::not_found("Article not found"))?;

    let export_settings = &settings.export_settings;
    let markdown = article_to_markdown(&article, export_settings);
    let filename = generate_filename(&article, &export_settings.filename_template);

    let file_path = PathBuf::from(vault_path).join(format!("{}.md", filename));

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| CommandError::io(format!("Failed to create directory: {}", e)))?;
    }

    std::fs::write(&file_path, &markdown)
        .map_err(|e| CommandError::io(format!("Failed to write file: {}", e)))?;

    Ok(file_path.to_string_lossy().to_string())
}

/// Batch export multiple articles
#[tauri::command]
pub async fn batch_export_markdown(
    state: State<'_, AppState>,
    article_ids: Vec<String>,
    destination_dir: String,
) -> Result<Vec<String>, CommandError> {
    let ids: Result<Vec<Uuid>, _> = article_ids.iter().map(|s| Uuid::parse_str(s)).collect();
    let ids = ids.map_err(|_| CommandError::validation("Invalid article ID"))?;

    let settings = state.settings.read().await;
    let export_settings = &settings.export_settings;
    let dest_path = PathBuf::from(&destination_dir);

    // Ensure destination exists
    std::fs::create_dir_all(&dest_path)
        .map_err(|e| CommandError::io(format!("Failed to create directory: {}", e)))?;

    let mut exported_files = Vec::new();

    for id in ids {
        if let Ok(Some(article)) = state.db.get_article(id) {
            let markdown = article_to_markdown(&article, export_settings);
            let filename = generate_filename(&article, &export_settings.filename_template);
            let file_path = dest_path.join(format!("{}.md", filename));

            if std::fs::write(&file_path, &markdown).is_ok() {
                exported_files.push(file_path.to_string_lossy().to_string());
            }
        }
    }

    Ok(exported_files)
}

#[cfg(test)]
mod tests {
    // Integration tests would use a mock state
}
