//! OPML import/export commands.

use tauri::State;
use uuid::Uuid;

use crate::commands::AppState;
use crate::core::feeds::{export_opml, parse_opml, OpmlDocument, OpmlOutline};
use crate::core::models::{Feed, Folder, ViewMode};
use crate::error::CommandError;

/// Import result with statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct ImportResult {
    pub feeds_imported: usize,
    pub folders_created: usize,
    pub feeds_skipped: usize,
    pub errors: Vec<String>,
}

/// Import feeds from an OPML file
#[tauri::command]
pub async fn import_opml(
    state: State<'_, AppState>,
    path: String,
    _merge: bool,
) -> Result<ImportResult, CommandError> {
    let content = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| CommandError::io(format!("Failed to read file: {}", e)))?;

    let doc = parse_opml(&content)?;

    let mut result = ImportResult {
        feeds_imported: 0,
        folders_created: 0,
        feeds_skipped: 0,
        errors: Vec::new(),
    };

    // If not merging, we could clear existing feeds first (but that's destructive)
    // For now, we always merge/skip duplicates

    // Process each outline recursively
    for outline in &doc.outlines {
        import_outline(&state, outline, None, &mut result).await;
    }

    Ok(result)
}

/// Recursively import an outline (folder or feed)
async fn import_outline(
    state: &State<'_, AppState>,
    outline: &OpmlOutline,
    parent_folder_id: Option<Uuid>,
    result: &mut ImportResult,
) {
    if outline.is_folder() {
        // Create folder
        match state.db.create_folder(&outline.text, parent_folder_id) {
            Ok(folder) => {
                result.folders_created += 1;

                // Update folder with extended attributes
                if outline.icon.is_some() || outline.color.is_some() || outline.view_mode.is_some()
                {
                    let mut updates: std::collections::HashMap<String, String> =
                        std::collections::HashMap::new();
                    if let Some(ref icon) = outline.icon {
                        updates.insert("icon".to_string(), icon.clone());
                    }
                    if let Some(ref color) = outline.color {
                        updates.insert("color".to_string(), color.clone());
                    }
                    let _ = state.db.update_folder(folder.id, &updates);
                }

                // Process children
                for child in &outline.children {
                    Box::pin(import_outline(state, child, Some(folder.id), result)).await;
                }
            }
            Err(e) => {
                result
                    .errors
                    .push(format!("Failed to create folder '{}': {}", outline.text, e));
            }
        }
    } else if let Some(ref url) = outline.xml_url {
        // Check if feed already exists
        if let Ok(existing) = state.db.get_feeds() {
            if existing.iter().any(|f| &f.url == url) {
                result.feeds_skipped += 1;
                return;
            }
        }

        // Create feed
        let view_mode = outline.view_mode.clone().unwrap_or(ViewMode::Article);
        let tags = outline.tags.clone().unwrap_or_default();

        // Use the text as title initially
        let feed = Feed {
            id: Uuid::new_v4(),
            url: url.clone(),
            title: outline.text.clone(),
            description: None,
            site_url: outline.html_url.clone(),
            icon_url: None,
            view_mode,
            tags,
            folder_id: parent_folder_id,
            etag: None,
            last_modified: None,
            last_fetched: None,
            last_error: None,
            refresh_interval: outline.refresh_interval,
            notify_new: outline.notify.unwrap_or(false),
            position: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        match state.db.create_feed(&feed) {
            Ok(_) => {
                result.feeds_imported += 1;
            }
            Err(e) => {
                result
                    .errors
                    .push(format!("Failed to import feed '{}': {}", outline.text, e));
            }
        }
    }
}

/// Export feeds to an OPML file
#[tauri::command]
pub async fn export_opml_file(
    state: State<'_, AppState>,
    path: String,
    extended: bool,
) -> Result<(), CommandError> {
    let feeds = state.db.get_feeds()?;
    let folders = state.db.get_folders()?;

    // Build document structure
    let doc = build_opml_document(&feeds, &folders);

    let xml = export_opml(&doc, extended)?;

    tokio::fs::write(&path, &xml)
        .await
        .map_err(|e: std::io::Error| CommandError::io(format!("Failed to write file: {}", e)))?;

    Ok(())
}

/// Export feeds to OPML string (for clipboard)
#[tauri::command]
pub async fn export_opml_string(
    state: State<'_, AppState>,
    extended: bool,
) -> Result<String, CommandError> {
    let feeds = state.db.get_feeds()?;
    let folders = state.db.get_folders()?;

    let doc = build_opml_document(&feeds, &folders);
    let xml = export_opml(&doc, extended)?;

    Ok(xml)
}

/// Build OPML document from feeds and folders
fn build_opml_document(feeds: &[Feed], folders: &[Folder]) -> OpmlDocument {
    // Create a map of folder_id -> children
    let mut folder_map: std::collections::HashMap<Option<Uuid>, Vec<OpmlOutline>> =
        std::collections::HashMap::new();

    // First, add all feeds to their parent folders
    for feed in feeds {
        let outline = OpmlOutline {
            text: feed.title.clone(),
            outline_type: Some("rss".to_string()),
            xml_url: Some(feed.url.clone()),
            html_url: feed.site_url.clone(),
            view_mode: Some(feed.view_mode.clone()),
            tags: if feed.tags.is_empty() {
                None
            } else {
                Some(feed.tags.clone())
            },
            icon: None,
            color: None,
            refresh_interval: feed.refresh_interval,
            notify: if feed.notify_new { Some(true) } else { None },
            channel_id: None, // Could extract from URL if YouTube
            subreddit: None,  // Could extract from URL if Reddit
            children: Vec::new(),
        };

        folder_map.entry(feed.folder_id).or_default().push(outline);
    }

    // Sort folders by position to build tree correctly
    let mut sorted_folders = folders.to_vec();
    sorted_folders.sort_by_key(|f| f.position);

    // Build folder outlines (bottom-up)
    let mut folder_outlines: std::collections::HashMap<Uuid, OpmlOutline> =
        std::collections::HashMap::new();

    // First pass: create all folder outlines
    for folder in &sorted_folders {
        let mut outline = OpmlOutline {
            text: folder.name.clone(),
            outline_type: None,
            xml_url: None,
            html_url: None,
            view_mode: folder.view_mode.clone(),
            tags: None,
            icon: folder.icon.clone(),
            color: folder.color.clone(),
            refresh_interval: None,
            notify: None,
            channel_id: None,
            subreddit: None,
            children: Vec::new(),
        };

        // Add feeds that belong to this folder
        if let Some(children) = folder_map.remove(&Some(folder.id)) {
            outline.children.extend(children);
        }

        folder_outlines.insert(folder.id, outline);
    }

    // Second pass: nest folders into their parents (bottom-up)
    for folder in sorted_folders.iter().rev() {
        if let Some(parent_id) = folder.parent_id {
            if let Some(outline) = folder_outlines.remove(&folder.id) {
                if let Some(parent) = folder_outlines.get_mut(&parent_id) {
                    parent.children.push(outline);
                }
            }
        }
    }

    // Collect root-level items
    let mut outlines: Vec<OpmlOutline> = Vec::new();

    // Add root-level folders
    for folder in &sorted_folders {
        if folder.parent_id.is_none() {
            if let Some(outline) = folder_outlines.remove(&folder.id) {
                outlines.push(outline);
            }
        }
    }

    // Add root-level feeds (no folder)
    if let Some(root_feeds) = folder_map.remove(&None) {
        outlines.extend(root_feeds);
    }

    OpmlDocument {
        title: "Curio Reader Feeds".to_string(),
        profile: None,
        outlines,
    }
}
