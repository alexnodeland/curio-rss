//! Article data model and related types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Individual content item (article, post, episode, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Article {
    pub id: Uuid,
    pub feed_id: Uuid,

    /// Original GUID from feed (for deduplication)
    pub guid: String,

    /// Content fields
    pub title: String,
    pub url: Option<String>,
    pub content_html: Option<String>,
    pub content_text: Option<String>,
    pub summary: Option<String>,
    pub author: Option<String>,

    /// Media
    pub thumbnail_url: Option<String>,
    pub media: Vec<MediaAttachment>,

    /// Timestamps
    pub published_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
    pub fetched_at: DateTime<Utc>,

    /// User state
    pub is_read: bool,
    pub is_starred: bool,
    pub is_read_later: bool,
    pub is_archived: bool,
    pub read_at: Option<DateTime<Utc>>,
    pub read_later_at: Option<DateTime<Utc>>,
    pub read_later_position: Option<i32>,

    /// Platform-specific metadata
    pub reddit_score: Option<i32>,
    pub reddit_num_comments: Option<i32>,
    pub youtube_duration: Option<i32>,
    pub podcast_duration: Option<i32>,
    pub podcast_progress: Option<i32>,
}

impl Article {
    /// Create a new article with minimal required fields
    pub fn new(feed_id: Uuid, guid: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            feed_id,
            guid: guid.into(),
            title: title.into(),
            url: None,
            content_html: None,
            content_text: None,
            summary: None,
            author: None,
            thumbnail_url: None,
            media: Vec::new(),
            published_at: None,
            updated_at: None,
            fetched_at: Utc::now(),
            is_read: false,
            is_starred: false,
            is_read_later: false,
            is_archived: false,
            read_at: None,
            read_later_at: None,
            read_later_position: None,
            reddit_score: None,
            reddit_num_comments: None,
            youtube_duration: None,
            podcast_duration: None,
            podcast_progress: None,
        }
    }

    /// Check if this article has any media attachments
    pub fn has_media(&self) -> bool {
        !self.media.is_empty()
    }

    /// Check if this article is a podcast episode
    pub fn is_podcast_episode(&self) -> bool {
        self.podcast_duration.is_some()
            || self.media.iter().any(|m| m.mime_type.starts_with("audio/"))
    }

    /// Check if this article is a video
    pub fn is_video(&self) -> bool {
        self.youtube_duration.is_some()
            || self.media.iter().any(|m| m.mime_type.starts_with("video/"))
    }

    /// Get the primary URL (article URL or first media URL)
    pub fn primary_url(&self) -> Option<&str> {
        self.url
            .as_deref()
            .or_else(|| self.media.first().map(|m| m.url.as_str()))
    }

    /// Mark as read
    pub fn mark_read(&mut self) {
        if !self.is_read {
            self.is_read = true;
            self.read_at = Some(Utc::now());
        }
    }

    /// Mark as unread
    pub fn mark_unread(&mut self) {
        self.is_read = false;
        self.read_at = None;
    }

    /// Toggle starred status
    pub fn toggle_starred(&mut self) -> bool {
        self.is_starred = !self.is_starred;
        self.is_starred
    }

    /// Add to read later queue
    pub fn add_to_read_later(&mut self, position: Option<i32>) {
        if !self.is_read_later {
            self.is_read_later = true;
            self.read_later_at = Some(Utc::now());
            self.read_later_position = position;
        }
    }

    /// Remove from read later queue
    pub fn remove_from_read_later(&mut self) {
        self.is_read_later = false;
        self.read_later_at = None;
        self.read_later_position = None;
    }
}

/// Media attachment (audio, video, image)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub id: Uuid,
    pub url: String,
    pub mime_type: String,
    pub size_bytes: Option<i64>,
    pub title: Option<String>,
    pub is_downloaded: bool,
    pub local_path: Option<String>,
}

impl MediaAttachment {
    /// Create a new media attachment
    pub fn new(url: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            url: url.into(),
            mime_type: mime_type.into(),
            size_bytes: None,
            title: None,
            is_downloaded: false,
            local_path: None,
        }
    }

    /// Check if this is an audio attachment
    pub fn is_audio(&self) -> bool {
        self.mime_type.starts_with("audio/")
    }

    /// Check if this is a video attachment
    pub fn is_video(&self) -> bool {
        self.mime_type.starts_with("video/")
    }

    /// Check if this is an image attachment
    pub fn is_image(&self) -> bool {
        self.mime_type.starts_with("image/")
    }
}

/// Filter criteria for querying articles
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArticleFilter {
    pub feed_id: Option<Uuid>,
    pub folder_id: Option<Uuid>,
    pub unread_only: bool,
    pub starred_only: bool,
    pub read_later_only: bool,
    pub archived: bool,
    pub tags: Option<Vec<String>>,
    pub search_query: Option<String>,
    pub published_after: Option<DateTime<Utc>>,
    pub published_before: Option<DateTime<Utc>>,
}

/// Pagination parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pagination {
    pub page: i32,
    pub per_page: i32,
}

impl Default for Pagination {
    fn default() -> Self {
        Self {
            page: 1,
            per_page: 50,
        }
    }
}

impl Pagination {
    /// Calculate offset for SQL queries
    pub fn offset(&self) -> i32 {
        (self.page - 1) * self.per_page
    }

    /// Calculate limit for SQL queries
    pub fn limit(&self) -> i32 {
        self.per_page
    }
}

/// Paginated list of articles
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArticlePage {
    pub items: Vec<Article>,
    pub total: i64,
    pub page: i32,
    pub per_page: i32,
    pub total_pages: i32,
}

impl ArticlePage {
    /// Create a new article page
    pub fn new(items: Vec<Article>, total: i64, pagination: &Pagination) -> Self {
        let total_pages = ((total as f64) / (pagination.per_page as f64)).ceil() as i32;
        Self {
            items,
            total,
            page: pagination.page,
            per_page: pagination.per_page,
            total_pages,
        }
    }

    /// Check if there are more pages
    pub fn has_next(&self) -> bool {
        self.page < self.total_pages
    }

    /// Check if this is the first page
    pub fn has_prev(&self) -> bool {
        self.page > 1
    }
}

/// Sort order for articles
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    #[default]
    NewestFirst,
    OldestFirst,
    ByScore,
    ByFeed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_article_new() {
        let feed_id = Uuid::new_v4();
        let article = Article::new(feed_id, "guid-123", "Test Article");

        assert_eq!(article.feed_id, feed_id);
        assert_eq!(article.guid, "guid-123");
        assert_eq!(article.title, "Test Article");
        assert!(!article.is_read);
        assert!(!article.is_starred);
        assert!(!article.is_read_later);
    }

    #[test]
    fn test_article_mark_read() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");

        assert!(!article.is_read);
        assert!(article.read_at.is_none());

        article.mark_read();

        assert!(article.is_read);
        assert!(article.read_at.is_some());

        // Marking read again should not change read_at
        let read_at = article.read_at;
        article.mark_read();
        assert_eq!(article.read_at, read_at);
    }

    #[test]
    fn test_article_mark_unread() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");
        article.mark_read();

        assert!(article.is_read);

        article.mark_unread();

        assert!(!article.is_read);
        assert!(article.read_at.is_none());
    }

    #[test]
    fn test_article_toggle_starred() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");

        assert!(!article.is_starred);

        let result = article.toggle_starred();
        assert!(result);
        assert!(article.is_starred);

        let result = article.toggle_starred();
        assert!(!result);
        assert!(!article.is_starred);
    }

    #[test]
    fn test_article_read_later() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");

        assert!(!article.is_read_later);

        article.add_to_read_later(Some(1));

        assert!(article.is_read_later);
        assert!(article.read_later_at.is_some());
        assert_eq!(article.read_later_position, Some(1));

        article.remove_from_read_later();

        assert!(!article.is_read_later);
        assert!(article.read_later_at.is_none());
        assert!(article.read_later_position.is_none());
    }

    #[test]
    fn test_article_is_podcast_episode() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");

        assert!(!article.is_podcast_episode());

        article.podcast_duration = Some(3600);
        assert!(article.is_podcast_episode());

        article.podcast_duration = None;
        article.media.push(MediaAttachment::new(
            "https://example.com/episode.mp3",
            "audio/mpeg",
        ));
        assert!(article.is_podcast_episode());
    }

    #[test]
    fn test_article_is_video() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");

        assert!(!article.is_video());

        article.youtube_duration = Some(600);
        assert!(article.is_video());

        article.youtube_duration = None;
        article.media.push(MediaAttachment::new(
            "https://example.com/video.mp4",
            "video/mp4",
        ));
        assert!(article.is_video());
    }

    #[test]
    fn test_article_primary_url() {
        let mut article = Article::new(Uuid::new_v4(), "guid", "Title");

        assert!(article.primary_url().is_none());

        article.url = Some("https://example.com/article".to_string());
        assert_eq!(article.primary_url(), Some("https://example.com/article"));

        // URL takes precedence over media
        article.media.push(MediaAttachment::new(
            "https://example.com/media.mp3",
            "audio/mpeg",
        ));
        assert_eq!(article.primary_url(), Some("https://example.com/article"));

        // Falls back to media if no URL
        article.url = None;
        assert_eq!(article.primary_url(), Some("https://example.com/media.mp3"));
    }

    #[test]
    fn test_media_attachment() {
        let audio = MediaAttachment::new("https://example.com/audio.mp3", "audio/mpeg");
        assert!(audio.is_audio());
        assert!(!audio.is_video());
        assert!(!audio.is_image());

        let video = MediaAttachment::new("https://example.com/video.mp4", "video/mp4");
        assert!(!video.is_audio());
        assert!(video.is_video());
        assert!(!video.is_image());

        let image = MediaAttachment::new("https://example.com/image.jpg", "image/jpeg");
        assert!(!image.is_audio());
        assert!(!image.is_video());
        assert!(image.is_image());
    }

    #[test]
    fn test_pagination() {
        let page1 = Pagination {
            page: 1,
            per_page: 50,
        };
        assert_eq!(page1.offset(), 0);
        assert_eq!(page1.limit(), 50);

        let page3 = Pagination {
            page: 3,
            per_page: 20,
        };
        assert_eq!(page3.offset(), 40);
        assert_eq!(page3.limit(), 20);
    }

    #[test]
    fn test_article_page() {
        let articles = vec![
            Article::new(Uuid::new_v4(), "1", "Article 1"),
            Article::new(Uuid::new_v4(), "2", "Article 2"),
        ];
        let pagination = Pagination {
            page: 1,
            per_page: 10,
        };
        let page = ArticlePage::new(articles, 25, &pagination);

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 25);
        assert_eq!(page.page, 1);
        assert_eq!(page.per_page, 10);
        assert_eq!(page.total_pages, 3);
        assert!(page.has_next());
        assert!(!page.has_prev());
    }

    #[test]
    fn test_article_page_last_page() {
        let articles = vec![Article::new(Uuid::new_v4(), "1", "Article 1")];
        let pagination = Pagination {
            page: 3,
            per_page: 10,
        };
        let page = ArticlePage::new(articles, 25, &pagination);

        assert!(!page.has_next());
        assert!(page.has_prev());
    }

    #[test]
    fn test_article_filter_default() {
        let filter = ArticleFilter::default();

        assert!(filter.feed_id.is_none());
        assert!(!filter.unread_only);
        assert!(!filter.starred_only);
        assert!(!filter.read_later_only);
    }

    #[test]
    fn test_sort_order_serialization() {
        let order = SortOrder::NewestFirst;
        let json = serde_json::to_string(&order).unwrap();
        assert_eq!(json, "\"newest_first\"");

        let deserialized: SortOrder = serde_json::from_str("\"by_score\"").unwrap();
        assert!(matches!(deserialized, SortOrder::ByScore));
    }
}
