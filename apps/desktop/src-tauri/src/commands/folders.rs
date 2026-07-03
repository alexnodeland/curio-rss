//! Folder management commands.

use tauri::State;
use uuid::Uuid;

use crate::commands::AppState;
use crate::core::models::{Folder, FolderFeedItem, FolderNode};
use crate::error::CommandError;

/// Create a new folder
#[tauri::command]
pub async fn create_folder(
    state: State<'_, AppState>,
    name: String,
    parent_id: Option<String>,
) -> Result<Folder, CommandError> {
    if name.trim().is_empty() {
        return Err(CommandError::validation("Folder name cannot be empty"));
    }

    let folder = if let Some(ref parent_str) = parent_id {
        let parent_uuid = Uuid::parse_str(parent_str)
            .map_err(|_| CommandError::validation("Invalid parent folder ID"))?;

        // Verify parent exists
        if state.db.get_folder(parent_uuid)?.is_none() {
            return Err(CommandError::not_found("Parent folder not found"));
        }

        Folder::new_child(name.trim(), parent_uuid)
    } else {
        Folder::new(name.trim())
    };

    state.db.insert_folder(&folder)?;
    Ok(folder)
}

/// Get all folders
#[tauri::command]
pub async fn get_folders(state: State<'_, AppState>) -> Result<Vec<Folder>, CommandError> {
    let folders = state.db.get_all_folders()?;
    Ok(folders)
}

/// Get folder tree with feed counts
#[tauri::command]
pub async fn get_folder_tree(state: State<'_, AppState>) -> Result<Vec<FolderNode>, CommandError> {
    let folders = state.db.get_all_folders()?;
    let feeds = state.db.get_all_feeds()?;
    let unread_counts = state.db.get_unread_counts_by_feed()?;

    // Build tree
    let mut root_nodes: Vec<FolderNode> = Vec::new();
    let mut folder_map: std::collections::HashMap<Uuid, FolderNode> =
        std::collections::HashMap::new();

    // Create nodes for all folders
    for folder in &folders {
        folder_map.insert(folder.id, FolderNode::new(folder.clone()));
    }

    // Assign feeds to folders
    for feed in &feeds {
        let feed_item = FolderFeedItem {
            id: feed.id,
            title: feed.title.clone(),
            icon_url: feed.icon_url.clone(),
            unread_count: unread_counts.get(&feed.id).copied().unwrap_or(0),
            position: feed.position,
        };

        if let Some(folder_id) = feed.folder_id {
            if let Some(node) = folder_map.get_mut(&folder_id) {
                node.feeds.push(feed_item);
            }
        }
    }

    // Build hierarchy
    let _folder_ids: Vec<Uuid> = folders.iter().map(|f| f.id).collect();
    for folder in &folders {
        if let Some(parent_id) = folder.parent_id {
            let child_node = folder_map.remove(&folder.id);
            if let Some(child) = child_node {
                if let Some(parent) = folder_map.get_mut(&parent_id) {
                    parent.children.push(child);
                }
            }
        }
    }

    // Collect root folders
    for folder in &folders {
        if folder.parent_id.is_none() {
            if let Some(node) = folder_map.remove(&folder.id) {
                root_nodes.push(node);
            }
        }
    }

    // Add feeds without folders to a virtual "Uncategorized" node
    let orphan_feeds: Vec<FolderFeedItem> = feeds
        .iter()
        .filter(|f| f.folder_id.is_none())
        .map(|f| FolderFeedItem {
            id: f.id,
            title: f.title.clone(),
            icon_url: f.icon_url.clone(),
            unread_count: unread_counts.get(&f.id).copied().unwrap_or(0),
            position: f.position,
        })
        .collect();

    // If there are orphan feeds, create a virtual folder for them
    if !orphan_feeds.is_empty() {
        let mut uncategorized = FolderNode::new(Folder::new("Uncategorized"));
        uncategorized.feeds = orphan_feeds;
        // Put uncategorized at the end
        uncategorized.folder.position = i32::MAX;
        root_nodes.push(uncategorized);
    }

    // Return nodes sorted by position
    root_nodes.sort_by(|a, b| a.folder.position.cmp(&b.folder.position));

    Ok(root_nodes)
}

/// Delete a folder
#[tauri::command]
pub async fn delete_folder(
    state: State<'_, AppState>,
    folder_id: String,
) -> Result<(), CommandError> {
    let id =
        Uuid::parse_str(&folder_id).map_err(|_| CommandError::validation("Invalid folder ID"))?;

    state.db.delete_folder(id)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[allow(unused_imports)]
    use super::*;
}
