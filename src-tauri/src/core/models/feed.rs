//! Feed data model and related types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// View mode for displaying feed content
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ViewMode {
    #[default]
    Article,
    Reddit,
    YouTube,
    Podcast,
    Gallery,
}

impl ViewMode {
    /// Parse view mode from string (case-insensitive)
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "article" => Some(Self::Article),
            "reddit" => Some(Self::Reddit),
            "youtube" => Some(Self::YouTube),
            "podcast" => Some(Self::Podcast),
            "gallery" => Some(Self::Gallery),
            _ => None,
        }
    }
}

/// RSS/Atom feed source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Feed {
    pub id: Uuid,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    pub site_url: Option<String>,
    pub icon_url: Option<String>,

    /// View mode for this feed
    pub view_mode: ViewMode,

    /// Tags for filtering
    pub tags: Vec<String>,

    /// Parent folder ID
    pub folder_id: Option<Uuid>,

    /// ETag for conditional GET
    pub etag: Option<String>,

    /// Last-Modified header value
    pub last_modified: Option<String>,

    /// Last successful fetch timestamp
    pub last_fetched: Option<DateTime<Utc>>,

    /// Last error message if fetch failed
    pub last_error: Option<String>,

    /// Custom refresh interval in seconds (None = use global)
    pub refresh_interval: Option<i32>,

    /// Whether to notify on new items
    pub notify_new: bool,

    /// Sort position within folder
    pub position: i32,

    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Feed {
    /// Create a new feed with default values
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            url: url.into(),
            title: title.into(),
            description: None,
            site_url: None,
            icon_url: None,
            view_mode: ViewMode::Article,
            tags: Vec::new(),
            folder_id: None,
            etag: None,
            last_modified: None,
            last_fetched: None,
            last_error: None,
            refresh_interval: None,
            notify_new: false,
            position: 0,
            created_at: now,
            updated_at: now,
        }
    }

    /// Check if this feed appears to be a YouTube feed
    pub fn is_youtube(&self) -> bool {
        self.url.contains("youtube.com/feeds")
            || self.view_mode == ViewMode::YouTube
    }

    /// Check if this feed appears to be a Reddit feed
    pub fn is_reddit(&self) -> bool {
        self.url.contains("reddit.com") || self.view_mode == ViewMode::Reddit
    }

    /// Check if this feed appears to be a podcast feed
    pub fn is_podcast(&self) -> bool {
        self.view_mode == ViewMode::Podcast
    }

    /// Extract YouTube channel ID from feed URL if present
    pub fn youtube_channel_id(&self) -> Option<&str> {
        if !self.url.contains("youtube.com") {
            return None;
        }
        self.url
            .split("channel_id=")
            .nth(1)
            .and_then(|s| s.split('&').next())
    }

    /// Extract subreddit name from feed URL if present
    pub fn subreddit(&self) -> Option<&str> {
        if !self.url.contains("reddit.com") {
            return None;
        }
        // URL pattern: https://www.reddit.com/r/rust/.rss
        self.url.split("/r/").nth(1).and_then(|s| s.split('/').next())
    }
}

/// Updates to apply to an existing feed
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeedUpdate {
    pub title: Option<String>,
    pub description: Option<String>,
    pub site_url: Option<String>,
    pub icon_url: Option<String>,
    pub view_mode: Option<ViewMode>,
    pub tags: Option<Vec<String>>,
    pub folder_id: Option<Option<Uuid>>,
    pub refresh_interval: Option<Option<i32>>,
    pub notify_new: Option<bool>,
    pub position: Option<i32>,
}

/// Result of a feed refresh operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    pub feed_id: Uuid,
    pub new_count: i32,
    pub updated_count: i32,
    pub unchanged: bool,
    pub error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_feed_new() {
        let feed = Feed::new("https://example.com/feed.xml", "Example Feed");

        assert_eq!(feed.url, "https://example.com/feed.xml");
        assert_eq!(feed.title, "Example Feed");
        assert_eq!(feed.view_mode, ViewMode::Article);
        assert!(feed.tags.is_empty());
        assert!(feed.folder_id.is_none());
        assert!(!feed.notify_new);
    }

    #[test]
    fn test_feed_is_youtube() {
        let feed = Feed::new(
            "https://www.youtube.com/feeds/videos.xml?channel_id=UC_iD0xppBwwsrM9DegC5cQQ",
            "YouTube Channel",
        );
        assert!(feed.is_youtube());

        let mut normal_feed = Feed::new("https://example.com/feed.xml", "Normal");
        assert!(!normal_feed.is_youtube());

        normal_feed.view_mode = ViewMode::YouTube;
        assert!(normal_feed.is_youtube());
    }

    #[test]
    fn test_feed_is_reddit() {
        let feed = Feed::new("https://www.reddit.com/r/rust/.rss", "r/rust");
        assert!(feed.is_reddit());

        let mut normal_feed = Feed::new("https://example.com/feed.xml", "Normal");
        assert!(!normal_feed.is_reddit());

        normal_feed.view_mode = ViewMode::Reddit;
        assert!(normal_feed.is_reddit());
    }

    #[test]
    fn test_youtube_channel_id_extraction() {
        let feed = Feed::new(
            "https://www.youtube.com/feeds/videos.xml?channel_id=UC_iD0xppBwwsrM9DegC5cQQ",
            "YouTube Channel",
        );
        assert_eq!(feed.youtube_channel_id(), Some("UC_iD0xppBwwsrM9DegC5cQQ"));

        let non_youtube = Feed::new("https://example.com/feed.xml", "Normal");
        assert!(non_youtube.youtube_channel_id().is_none());
    }

    #[test]
    fn test_subreddit_extraction() {
        let feed = Feed::new("https://www.reddit.com/r/rust/.rss", "r/rust");
        assert_eq!(feed.subreddit(), Some("rust"));

        let feed2 = Feed::new("https://www.reddit.com/r/programming/.rss", "r/programming");
        assert_eq!(feed2.subreddit(), Some("programming"));

        let non_reddit = Feed::new("https://example.com/feed.xml", "Normal");
        assert!(non_reddit.subreddit().is_none());
    }

    #[test]
    fn test_view_mode_from_str_loose() {
        assert_eq!(ViewMode::from_str_loose("article"), Some(ViewMode::Article));
        assert_eq!(ViewMode::from_str_loose("ARTICLE"), Some(ViewMode::Article));
        assert_eq!(ViewMode::from_str_loose("Reddit"), Some(ViewMode::Reddit));
        assert_eq!(ViewMode::from_str_loose("youtube"), Some(ViewMode::YouTube));
        assert_eq!(ViewMode::from_str_loose("podcast"), Some(ViewMode::Podcast));
        assert_eq!(ViewMode::from_str_loose("gallery"), Some(ViewMode::Gallery));
        assert_eq!(ViewMode::from_str_loose("unknown"), None);
    }

    #[test]
    fn test_view_mode_serialization() {
        let mode = ViewMode::YouTube;
        let json = serde_json::to_string(&mode).unwrap();
        assert_eq!(json, "\"youtube\"");

        let deserialized: ViewMode = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, ViewMode::YouTube);
    }

    #[test]
    fn test_feed_serialization() {
        let feed = Feed::new("https://example.com/feed.xml", "Test Feed");
        let json = serde_json::to_string(&feed).unwrap();

        assert!(json.contains("https://example.com/feed.xml"));
        assert!(json.contains("Test Feed"));
        assert!(json.contains("\"view_mode\":\"article\""));

        let deserialized: Feed = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.url, feed.url);
        assert_eq!(deserialized.title, feed.title);
    }

    #[test]
    fn test_feed_update_partial() {
        let update = FeedUpdate {
            title: Some("New Title".to_string()),
            ..Default::default()
        };

        assert_eq!(update.title, Some("New Title".to_string()));
        assert!(update.description.is_none());
        assert!(update.view_mode.is_none());
    }

    #[test]
    fn test_refresh_result() {
        let result = RefreshResult {
            feed_id: Uuid::new_v4(),
            new_count: 5,
            updated_count: 2,
            unchanged: false,
            error: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"new_count\":5"));
        assert!(json.contains("\"updated_count\":2"));
    }
}
