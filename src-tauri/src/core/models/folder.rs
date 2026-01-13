//! Folder data model for feed organization.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::ViewMode;

/// Folder for organizing feeds
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub view_mode: Option<ViewMode>,
    pub position: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Folder {
    /// Create a new folder
    pub fn new(name: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            parent_id: None,
            icon: None,
            color: None,
            view_mode: None,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Create a child folder
    pub fn new_child(name: impl Into<String>, parent_id: Uuid) -> Self {
        let mut folder = Self::new(name);
        folder.parent_id = Some(parent_id);
        folder
    }

    /// Check if this is a root folder
    pub fn is_root(&self) -> bool {
        self.parent_id.is_none()
    }
}

/// Updates to apply to an existing folder
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FolderUpdate {
    pub name: Option<String>,
    pub parent_id: Option<Option<Uuid>>,
    pub icon: Option<Option<String>>,
    pub color: Option<Option<String>>,
    pub view_mode: Option<Option<ViewMode>>,
    pub position: Option<i32>,
}

/// Folder tree node for hierarchical display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderNode {
    pub folder: Folder,
    pub children: Vec<FolderNode>,
    pub feeds: Vec<FolderFeedItem>,
    pub unread_count: i32,
}

impl FolderNode {
    /// Create a new folder node
    pub fn new(folder: Folder) -> Self {
        Self {
            folder,
            children: Vec::new(),
            feeds: Vec::new(),
            unread_count: 0,
        }
    }

    /// Calculate total unread count including children
    pub fn total_unread(&self) -> i32 {
        let children_unread: i32 = self.children.iter().map(|c| c.total_unread()).sum();
        let feeds_unread: i32 = self.feeds.iter().map(|f| f.unread_count).sum();
        self.unread_count + children_unread + feeds_unread
    }

    /// Check if this folder or any children have unread items
    pub fn has_unread(&self) -> bool {
        self.total_unread() > 0
    }

    /// Get all folder IDs in this subtree
    pub fn all_folder_ids(&self) -> Vec<Uuid> {
        let mut ids = vec![self.folder.id];
        for child in &self.children {
            ids.extend(child.all_folder_ids());
        }
        ids
    }

    /// Get all feed IDs in this subtree
    pub fn all_feed_ids(&self) -> Vec<Uuid> {
        let mut ids: Vec<Uuid> = self.feeds.iter().map(|f| f.id).collect();
        for child in &self.children {
            ids.extend(child.all_feed_ids());
        }
        ids
    }
}

/// Simplified feed info for folder tree display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderFeedItem {
    pub id: Uuid,
    pub title: String,
    pub icon_url: Option<String>,
    pub unread_count: i32,
    pub position: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_folder_new() {
        let folder = Folder::new("Tech");

        assert_eq!(folder.name, "Tech");
        assert!(folder.parent_id.is_none());
        assert!(folder.is_root());
        assert_eq!(folder.position, 0);
    }

    #[test]
    fn test_folder_new_child() {
        let parent_id = Uuid::new_v4();
        let folder = Folder::new_child("Rust", parent_id);

        assert_eq!(folder.name, "Rust");
        assert_eq!(folder.parent_id, Some(parent_id));
        assert!(!folder.is_root());
    }

    #[test]
    fn test_folder_serialization() {
        let mut folder = Folder::new("Test Folder");
        folder.icon = Some("folder".to_string());
        folder.color = Some("#3b82f6".to_string());

        let json = serde_json::to_string(&folder).unwrap();
        assert!(json.contains("Test Folder"));
        assert!(json.contains("#3b82f6"));

        let deserialized: Folder = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, folder.name);
        assert_eq!(deserialized.color, folder.color);
    }

    #[test]
    fn test_folder_node_total_unread() {
        let mut root = FolderNode::new(Folder::new("Root"));
        root.unread_count = 5;

        let mut child = FolderNode::new(Folder::new("Child"));
        child.unread_count = 3;
        child.feeds.push(FolderFeedItem {
            id: Uuid::new_v4(),
            title: "Feed 1".to_string(),
            icon_url: None,
            unread_count: 10,
            position: 0,
        });

        root.children.push(child);
        root.feeds.push(FolderFeedItem {
            id: Uuid::new_v4(),
            title: "Feed 2".to_string(),
            icon_url: None,
            unread_count: 2,
            position: 0,
        });

        // 5 (root) + 3 (child) + 10 (child feed) + 2 (root feed) = 20
        assert_eq!(root.total_unread(), 20);
        assert!(root.has_unread());
    }

    #[test]
    fn test_folder_node_no_unread() {
        let root = FolderNode::new(Folder::new("Root"));
        assert_eq!(root.total_unread(), 0);
        assert!(!root.has_unread());
    }

    #[test]
    fn test_folder_node_all_folder_ids() {
        let mut root = FolderNode::new(Folder::new("Root"));
        let child1 = FolderNode::new(Folder::new("Child 1"));
        let child2 = FolderNode::new(Folder::new("Child 2"));

        let root_id = root.folder.id;
        let child1_id = child1.folder.id;
        let child2_id = child2.folder.id;

        root.children.push(child1);
        root.children.push(child2);

        let ids = root.all_folder_ids();
        assert_eq!(ids.len(), 3);
        assert!(ids.contains(&root_id));
        assert!(ids.contains(&child1_id));
        assert!(ids.contains(&child2_id));
    }

    #[test]
    fn test_folder_node_all_feed_ids() {
        let mut root = FolderNode::new(Folder::new("Root"));
        let feed1_id = Uuid::new_v4();
        let feed2_id = Uuid::new_v4();

        root.feeds.push(FolderFeedItem {
            id: feed1_id,
            title: "Feed 1".to_string(),
            icon_url: None,
            unread_count: 0,
            position: 0,
        });

        let mut child = FolderNode::new(Folder::new("Child"));
        child.feeds.push(FolderFeedItem {
            id: feed2_id,
            title: "Feed 2".to_string(),
            icon_url: None,
            unread_count: 0,
            position: 0,
        });

        root.children.push(child);

        let ids = root.all_feed_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&feed1_id));
        assert!(ids.contains(&feed2_id));
    }

    #[test]
    fn test_folder_update_partial() {
        let update = FolderUpdate {
            name: Some("New Name".to_string()),
            color: Some(Some("#ff0000".to_string())),
            ..Default::default()
        };

        assert_eq!(update.name, Some("New Name".to_string()));
        assert_eq!(update.color, Some(Some("#ff0000".to_string())));
        assert!(update.icon.is_none());
        assert!(update.parent_id.is_none());
    }
}
