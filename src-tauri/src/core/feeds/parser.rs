//! RSS/Atom feed parsing using feed-rs.

use chrono::{DateTime, Utc};
use feed_rs::model::Entry;
use feed_rs::parser;
use uuid::Uuid;

use crate::core::models::{Article, MediaAttachment, ViewMode};
use crate::error::{CoreError, InfraError};

/// Result of parsing a feed
#[derive(Debug)]
pub struct ParsedFeed {
    pub title: String,
    pub description: Option<String>,
    pub site_url: Option<String>,
    pub icon_url: Option<String>,
    pub articles: Vec<Article>,
}

/// Parse a feed from raw XML/JSON content
pub fn parse_feed(content: &str, feed_id: Uuid) -> Result<ParsedFeed, InfraError> {
    let parsed =
        parser::parse(content.as_bytes()).map_err(|e| InfraError::FeedParse(e.to_string()))?;

    let articles = parsed
        .entries
        .into_iter()
        .map(|entry| entry_to_article(entry, feed_id))
        .collect();

    Ok(ParsedFeed {
        title: parsed.title.map(|t| t.content).unwrap_or_default(),
        description: parsed.description.map(|d| d.content),
        site_url: parsed.links.first().map(|l| l.href.clone()),
        icon_url: parsed.icon.map(|i| i.uri),
        articles,
    })
}

/// Convert a feed-rs Entry to our Article model
fn entry_to_article(entry: Entry, feed_id: Uuid) -> Article {
    let guid = entry.id;
    let title = entry
        .title
        .map(|t| t.content)
        .unwrap_or_else(|| "Untitled".to_string());

    let mut article = Article::new(feed_id, guid, title);

    // URL
    article.url = entry.links.first().map(|l| l.href.clone());

    // Content
    if let Some(content) = entry.content {
        article.content_html = content.body;
    }

    // Summary
    article.summary = entry.summary.map(|s| s.content);

    // Author
    article.author = entry.authors.first().map(|a| a.name.clone());

    // Thumbnail
    article.thumbnail_url = entry
        .media
        .first()
        .and_then(|m| m.thumbnails.first())
        .map(|t| t.image.uri.clone());

    // Media attachments
    article.media = entry
        .media
        .into_iter()
        .flat_map(|m| m.content)
        .filter_map(|c| {
            c.url.map(|url| {
                let mut attachment = MediaAttachment::new(
                    url.to_string(),
                    c.content_type
                        .map(|m| m.to_string())
                        .unwrap_or_else(|| "application/octet-stream".to_string()),
                );
                attachment.size_bytes = c.size.map(|s| s as i64);
                attachment
            })
        })
        .collect();

    // Timestamps
    article.published_at = entry.published;
    article.updated_at = entry.updated;

    // Platform-specific: Reddit score from extensions
    // Reddit includes score in the content, we'll parse it later if needed

    article
}

/// Detect the appropriate view mode for a feed URL
pub fn detect_view_mode(url: &str) -> ViewMode {
    let url_lower = url.to_lowercase();

    if url_lower.contains("youtube.com/feeds") {
        ViewMode::YouTube
    } else if url_lower.contains("reddit.com") {
        ViewMode::Reddit
    } else if is_likely_podcast_url(&url_lower) {
        ViewMode::Podcast
    } else {
        ViewMode::Article
    }
}

/// Check if a URL is likely a podcast feed
fn is_likely_podcast_url(url: &str) -> bool {
    url.contains("podcast")
        || url.contains("feed.xml")
        || url.contains("rss.xml")
        || url.contains("anchor.fm")
        || url.contains("libsyn.com")
        || url.contains("podbean.com")
        || url.contains("buzzsprout.com")
        || url.contains("transistor.fm")
}

/// Validate and normalize a feed URL
pub fn validate_feed_url(url: &str) -> Result<String, CoreError> {
    let url = url.trim();

    if url.is_empty() {
        return Err(CoreError::InvalidFeedUrl("URL cannot be empty".to_string()));
    }

    // Parse the URL
    let parsed = url::Url::parse(url)
        .map_err(|_| CoreError::InvalidFeedUrl(format!("Invalid URL format: {}", url)))?;

    // Check scheme
    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(CoreError::InvalidFeedUrl(format!(
                "URL must use HTTP or HTTPS, got: {}",
                scheme
            )))
        }
    }

    // Check for host
    if parsed.host().is_none() {
        return Err(CoreError::InvalidFeedUrl(
            "URL must have a valid host".to_string(),
        ));
    }

    Ok(parsed.to_string())
}

/// Extract YouTube channel ID from various YouTube URL formats
pub fn extract_youtube_channel_id(url: &str) -> Option<String> {
    // Direct channel ID from feed URL
    if url.contains("channel_id=") {
        return url
            .split("channel_id=")
            .nth(1)
            .and_then(|s| s.split('&').next())
            .map(String::from);
    }

    // Parse URL to extract from path
    if let Ok(parsed) = url::Url::parse(url) {
        let path = parsed.path();

        // /channel/UC... format
        if path.contains("/channel/") {
            return path
                .split("/channel/")
                .nth(1)
                .and_then(|s| s.split('/').next())
                .map(String::from);
        }
    }

    None
}

/// Extract subreddit name from Reddit URL
pub fn extract_subreddit(url: &str) -> Option<String> {
    if !url.contains("reddit.com") {
        return None;
    }

    // Match /r/subreddit pattern
    url.split("/r/")
        .nth(1)
        .and_then(|s| s.split('/').next())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Build YouTube RSS feed URL from channel ID
pub fn youtube_feed_url(channel_id: &str) -> String {
    format!(
        "https://www.youtube.com/feeds/videos.xml?channel_id={}",
        channel_id
    )
}

/// Build Reddit RSS feed URL from subreddit
pub fn reddit_feed_url(subreddit: &str, sort: &str) -> String {
    format!("https://www.reddit.com/r/{}/.rss?sort={}", subreddit, sort)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_RSS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0">
  <channel>
    <title>Example Feed</title>
    <link>https://example.com</link>
    <description>An example RSS feed</description>
    <item>
      <title>Article One</title>
      <link>https://example.com/article-1</link>
      <description>First article description</description>
      <pubDate>Mon, 15 Jan 2024 10:00:00 GMT</pubDate>
      <guid>article-1</guid>
    </item>
    <item>
      <title>Article Two</title>
      <link>https://example.com/article-2</link>
      <description>Second article description</description>
      <pubDate>Tue, 16 Jan 2024 12:00:00 GMT</pubDate>
      <guid>article-2</guid>
    </item>
  </channel>
</rss>"#;

    const SAMPLE_ATOM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Example Atom Feed</title>
  <link href="https://example.com"/>
  <id>urn:uuid:example-feed</id>
  <updated>2024-01-15T10:00:00Z</updated>
  <entry>
    <title>Atom Entry</title>
    <link href="https://example.com/entry-1"/>
    <id>urn:uuid:entry-1</id>
    <updated>2024-01-15T10:00:00Z</updated>
    <summary>An atom entry summary</summary>
  </entry>
</feed>"#;

    #[test]
    fn test_parse_rss_feed() {
        let feed_id = Uuid::new_v4();
        let result = parse_feed(SAMPLE_RSS, feed_id).unwrap();

        assert_eq!(result.title, "Example Feed");
        assert_eq!(result.description, Some("An example RSS feed".to_string()));
        assert_eq!(result.articles.len(), 2);

        let article = &result.articles[0];
        assert_eq!(article.title, "Article One");
        assert_eq!(
            article.url,
            Some("https://example.com/article-1".to_string())
        );
        assert!(article.published_at.is_some());
    }

    #[test]
    fn test_parse_atom_feed() {
        let feed_id = Uuid::new_v4();
        let result = parse_feed(SAMPLE_ATOM, feed_id).unwrap();

        assert_eq!(result.title, "Example Atom Feed");
        assert_eq!(result.articles.len(), 1);

        let article = &result.articles[0];
        assert_eq!(article.title, "Atom Entry");
        assert_eq!(article.summary, Some("An atom entry summary".to_string()));
    }

    #[test]
    fn test_parse_invalid_feed() {
        let feed_id = Uuid::new_v4();
        let result = parse_feed("not valid xml", feed_id);

        assert!(result.is_err());
    }

    #[test]
    fn test_detect_view_mode_youtube() {
        let url = "https://www.youtube.com/feeds/videos.xml?channel_id=UC123";
        assert_eq!(detect_view_mode(url), ViewMode::YouTube);
    }

    #[test]
    fn test_detect_view_mode_reddit() {
        let url = "https://www.reddit.com/r/rust/.rss";
        assert_eq!(detect_view_mode(url), ViewMode::Reddit);
    }

    #[test]
    fn test_detect_view_mode_podcast() {
        let url = "https://anchor.fm/s/123/podcast/rss";
        assert_eq!(detect_view_mode(url), ViewMode::Podcast);
    }

    #[test]
    fn test_detect_view_mode_article() {
        let url = "https://example.com/feed.xml";
        assert_eq!(detect_view_mode(url), ViewMode::Article);
    }

    #[test]
    fn test_validate_feed_url_valid() {
        let result = validate_feed_url("https://example.com/feed.xml");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://example.com/feed.xml");
    }

    #[test]
    fn test_validate_feed_url_with_whitespace() {
        let result = validate_feed_url("  https://example.com/feed.xml  ");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://example.com/feed.xml");
    }

    #[test]
    fn test_validate_feed_url_invalid_scheme() {
        let result = validate_feed_url("ftp://example.com/feed.xml");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_feed_url_empty() {
        let result = validate_feed_url("");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_feed_url_invalid_format() {
        let result = validate_feed_url("not a url");
        assert!(result.is_err());
    }

    #[test]
    fn test_extract_youtube_channel_id_from_feed() {
        let url = "https://www.youtube.com/feeds/videos.xml?channel_id=UC_iD0xppBwwsrM9DegC5cQQ";
        let result = extract_youtube_channel_id(url);
        assert_eq!(result, Some("UC_iD0xppBwwsrM9DegC5cQQ".to_string()));
    }

    #[test]
    fn test_extract_youtube_channel_id_from_channel_url() {
        let url = "https://www.youtube.com/channel/UC_iD0xppBwwsrM9DegC5cQQ";
        let result = extract_youtube_channel_id(url);
        assert_eq!(result, Some("UC_iD0xppBwwsrM9DegC5cQQ".to_string()));
    }

    #[test]
    fn test_extract_youtube_channel_id_none() {
        let url = "https://example.com/feed.xml";
        let result = extract_youtube_channel_id(url);
        assert!(result.is_none());
    }

    #[test]
    fn test_extract_subreddit() {
        let url = "https://www.reddit.com/r/rust/.rss";
        let result = extract_subreddit(url);
        assert_eq!(result, Some("rust".to_string()));
    }

    #[test]
    fn test_extract_subreddit_none() {
        let url = "https://example.com/feed.xml";
        let result = extract_subreddit(url);
        assert!(result.is_none());
    }

    #[test]
    fn test_youtube_feed_url() {
        let url = youtube_feed_url("UC_iD0xppBwwsrM9DegC5cQQ");
        assert_eq!(
            url,
            "https://www.youtube.com/feeds/videos.xml?channel_id=UC_iD0xppBwwsrM9DegC5cQQ"
        );
    }

    #[test]
    fn test_reddit_feed_url() {
        let url = reddit_feed_url("rust", "hot");
        assert_eq!(url, "https://www.reddit.com/r/rust/.rss?sort=hot");
    }
}
