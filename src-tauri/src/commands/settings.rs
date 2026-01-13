//! Settings management commands.

use std::path::PathBuf;
use tauri::State;

use crate::commands::AppState;
use crate::core::models::Settings;
use crate::core::themes::{get_builtin_themes, get_theme, Theme};
use crate::error::CommandError;
use crate::services::cache::{CacheStats, ImageCache};

/// Get current settings
#[tauri::command]
pub async fn get_settings(state: State<'_, AppState>) -> Result<Settings, CommandError> {
    let settings = state.settings.read().await;
    Ok(settings.clone())
}

/// Update settings
#[tauri::command]
pub async fn update_settings(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), CommandError> {
    // Write to file
    let settings_path = state.config_dir.join("settings.json");
    let json = serde_json::to_string_pretty(&settings)
        .map_err(|e| CommandError::internal(format!("Failed to serialize settings: {}", e)))?;

    tokio::fs::write(&settings_path, json)
        .await
        .map_err(|e| CommandError::io(format!("Failed to write settings: {}", e)))?;

    // Update in-memory settings
    let mut current = state.settings.write().await;
    *current = settings;

    Ok(())
}

/// Load settings from disk
#[tauri::command]
pub async fn load_settings(state: State<'_, AppState>) -> Result<Settings, CommandError> {
    let settings_path = state.config_dir.join("settings.json");

    if settings_path.exists() {
        let json = tokio::fs::read_to_string(&settings_path)
            .await
            .map_err(|e| CommandError::io(format!("Failed to read settings: {}", e)))?;

        let settings: Settings = serde_json::from_str(&json)
            .map_err(|e| CommandError::internal(format!("Failed to parse settings: {}", e)))?;

        // Update in-memory settings
        let mut current = state.settings.write().await;
        *current = settings.clone();

        Ok(settings)
    } else {
        Ok(Settings::default())
    }
}

/// Get all available themes
#[tauri::command]
pub async fn get_themes() -> Result<Vec<Theme>, CommandError> {
    Ok(get_builtin_themes())
}

/// Get a specific theme by ID
#[tauri::command]
pub async fn get_theme_by_id(theme_id: String) -> Result<Theme, CommandError> {
    get_theme(&theme_id)
        .ok_or_else(|| CommandError::not_found(format!("Theme not found: {}", theme_id)))
}

/// Save a custom theme
#[tauri::command]
pub async fn save_custom_theme(
    state: State<'_, AppState>,
    theme: Theme,
) -> Result<(), CommandError> {
    let themes_dir = state.config_dir.join("themes");
    std::fs::create_dir_all(&themes_dir)
        .map_err(|e| CommandError::io(format!("Failed to create themes directory: {}", e)))?;

    let theme_path = themes_dir.join(format!("{}.json", theme.id));
    let json = serde_json::to_string_pretty(&theme)
        .map_err(|e| CommandError::internal(format!("Failed to serialize theme: {}", e)))?;

    tokio::fs::write(&theme_path, json)
        .await
        .map_err(|e| CommandError::io(format!("Failed to write theme: {}", e)))?;

    Ok(())
}

/// Delete a custom theme
#[tauri::command]
pub async fn delete_custom_theme(
    state: State<'_, AppState>,
    theme_id: String,
) -> Result<(), CommandError> {
    // Don't allow deleting built-in themes
    if get_theme(&theme_id).is_some() {
        return Err(CommandError::validation("Cannot delete built-in themes"));
    }

    let theme_path = state
        .config_dir
        .join("themes")
        .join(format!("{}.json", theme_id));

    if theme_path.exists() {
        tokio::fs::remove_file(&theme_path)
            .await
            .map_err(|e| CommandError::io(format!("Failed to delete theme: {}", e)))?;
    }

    Ok(())
}

/// Get cache statistics
#[tauri::command]
pub async fn get_cache_stats(state: State<'_, AppState>) -> Result<CacheStats, CommandError> {
    let cache = ImageCache::new(state.config_dir.join("cache"));
    cache.get_stats().map_err(CommandError::from)
}

/// Clear image cache
#[tauri::command]
pub async fn clear_image_cache(
    state: State<'_, AppState>,
    cache_type: Option<String>,
) -> Result<usize, CommandError> {
    let cache = ImageCache::new(state.config_dir.join("cache"));
    cache
        .clear(cache_type.as_deref())
        .map_err(CommandError::from)
}
