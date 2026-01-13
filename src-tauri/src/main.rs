//! Curio Reader - Main entry point

#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use curio_reader_lib::commands::{self, AppState};

fn main() {
    // Initialize logging
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "curio_reader=debug,info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("Starting Curio Reader");

    // Get config directory
    let config_dir = dirs_config_dir().unwrap_or_else(|| PathBuf::from("."));

    // Initialize application state
    let app_state = AppState::new(config_dir).expect("Failed to initialize application");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            // Feed commands
            commands::add_feed,
            commands::get_feeds,
            commands::get_feed,
            commands::update_feed,
            commands::delete_feed,
            commands::refresh_feed,
            commands::refresh_all_feeds,
            // Article commands
            commands::get_articles,
            commands::get_article,
            commands::mark_read,
            commands::mark_unread,
            commands::toggle_starred,
            commands::search_articles,
            commands::add_to_read_later,
            commands::remove_from_read_later,
            commands::get_read_later,
            commands::reorder_read_later,
            // Folder commands
            commands::create_folder,
            commands::get_folders,
            commands::get_folder_tree,
            commands::delete_folder,
            // Reddit commands
            commands::fetch_reddit_thread,
            commands::parse_reddit_url,
            // YouTube commands
            commands::fetch_youtube_metadata,
            commands::fetch_youtube_comments,
            commands::check_ytdlp_version,
            commands::parse_youtube_url,
            // Export commands
            commands::export_to_markdown,
            commands::copy_as_markdown,
            commands::generate_export_filename,
            commands::export_to_obsidian,
            commands::batch_export_markdown,
            // OPML commands
            commands::import_opml,
            commands::export_opml_file,
            commands::export_opml_string,
            // Settings commands
            commands::get_settings,
            commands::update_settings,
            commands::load_settings,
            commands::get_themes,
            commands::get_theme_by_id,
            commands::save_custom_theme,
            commands::delete_custom_theme,
            commands::get_cache_stats,
            commands::clear_image_cache,
            // Podcast commands
            commands::download_episode,
            commands::delete_download,
            commands::update_playback_progress,
            commands::get_playback_progress,
            commands::is_episode_downloaded,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Get the config directory for Curio Reader
fn dirs_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("curio-reader"))
}

/// Alternative for systems without dirs
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn dirs_config_dir() -> Option<PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| PathBuf::from(h).join(".config").join("curio-reader"))
}
