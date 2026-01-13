//! YouTube-specific commands.

use tauri::State;

use crate::commands::AppState;
use crate::error::CommandError;
use crate::platform::youtube::{YouTubeComment, YouTubeMetadata};

/// Fetch YouTube video metadata
#[tauri::command]
pub async fn fetch_youtube_metadata(
    video_url: &str,
    _state: State<'_, AppState>,
) -> Result<YouTubeMetadata, CommandError> {
    let metadata = crate::platform::youtube::fetch_metadata(video_url).await?;
    Ok(metadata)
}

/// Fetch YouTube video comments
#[tauri::command]
pub async fn fetch_youtube_comments(
    video_url: &str,
    _state: State<'_, AppState>,
) -> Result<Vec<YouTubeComment>, CommandError> {
    let comments = crate::platform::youtube::fetch_comments(video_url).await?;
    Ok(comments)
}

/// Check if yt-dlp is available
#[tauri::command]
pub fn check_ytdlp_version() -> Result<String, CommandError> {
    let version = crate::platform::youtube::get_version()?;
    Ok(version)
}

/// Extract YouTube video ID from various URL formats
#[tauri::command]
pub fn parse_youtube_url(url: &str) -> Result<YouTubeUrlInfo, CommandError> {
    let info = extract_youtube_info(url)
        .ok_or_else(|| CommandError::Validation("Invalid YouTube URL".to_string()))?;
    Ok(info)
}

/// Parsed YouTube URL information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct YouTubeUrlInfo {
    pub video_id: Option<String>,
    pub channel_id: Option<String>,
    pub playlist_id: Option<String>,
    pub url_type: YouTubeUrlType,
}

/// Type of YouTube URL
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum YouTubeUrlType {
    Video,
    Channel,
    Playlist,
    Unknown,
}

/// Extract video ID and other info from various YouTube URL formats
fn extract_youtube_info(url: &str) -> Option<YouTubeUrlInfo> {
    let url = url.trim();

    // Standard video URL: youtube.com/watch?v=VIDEO_ID
    if url.contains("youtube.com/watch") {
        if let Some(v_param) = extract_query_param(url, "v") {
            return Some(YouTubeUrlInfo {
                video_id: Some(v_param),
                channel_id: None,
                playlist_id: extract_query_param(url, "list"),
                url_type: YouTubeUrlType::Video,
            });
        }
    }

    // Short URL: youtu.be/VIDEO_ID
    if url.contains("youtu.be/") {
        if let Some(id) = url.split("youtu.be/").nth(1) {
            let video_id = id.split(['?', '&', '#']).next()?.to_string();
            return Some(YouTubeUrlInfo {
                video_id: Some(video_id),
                channel_id: None,
                playlist_id: extract_query_param(url, "list"),
                url_type: YouTubeUrlType::Video,
            });
        }
    }

    // Embed URL: youtube.com/embed/VIDEO_ID
    if url.contains("youtube.com/embed/") {
        if let Some(id) = url.split("/embed/").nth(1) {
            let video_id = id.split(['?', '&', '#']).next()?.to_string();
            return Some(YouTubeUrlInfo {
                video_id: Some(video_id),
                channel_id: None,
                playlist_id: None,
                url_type: YouTubeUrlType::Video,
            });
        }
    }

    // Channel URL: youtube.com/channel/CHANNEL_ID
    if url.contains("youtube.com/channel/") {
        if let Some(id) = url.split("/channel/").nth(1) {
            let channel_id = id.split(['/', '?', '#']).next()?.to_string();
            return Some(YouTubeUrlInfo {
                video_id: None,
                channel_id: Some(channel_id),
                playlist_id: None,
                url_type: YouTubeUrlType::Channel,
            });
        }
    }

    // Handle URL: youtube.com/@handle
    if url.contains("youtube.com/@") {
        if let Some(handle) = url.split("/@").nth(1) {
            let channel_id = format!("@{}", handle.split(['/', '?', '#']).next()?);
            return Some(YouTubeUrlInfo {
                video_id: None,
                channel_id: Some(channel_id),
                playlist_id: None,
                url_type: YouTubeUrlType::Channel,
            });
        }
    }

    // Playlist URL: youtube.com/playlist?list=PLAYLIST_ID
    if url.contains("youtube.com/playlist") {
        if let Some(list_id) = extract_query_param(url, "list") {
            return Some(YouTubeUrlInfo {
                video_id: None,
                channel_id: None,
                playlist_id: Some(list_id),
                url_type: YouTubeUrlType::Playlist,
            });
        }
    }

    None
}

/// Extract a query parameter from a URL
fn extract_query_param(url: &str, param: &str) -> Option<String> {
    let query_start = url.find('?')?;
    let query = &url[query_start + 1..];

    for part in query.split('&') {
        if let Some((key, value)) = part.split_once('=') {
            if key == param {
                return Some(value.split('#').next()?.to_string());
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_standard_youtube_url() {
        let info = extract_youtube_info("https://www.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        assert_eq!(info.video_id, Some("dQw4w9WgXcQ".to_string()));
        assert!(matches!(info.url_type, YouTubeUrlType::Video));
    }

    #[test]
    fn test_parse_short_youtube_url() {
        let info = extract_youtube_info("https://youtu.be/dQw4w9WgXcQ").unwrap();
        assert_eq!(info.video_id, Some("dQw4w9WgXcQ".to_string()));
        assert!(matches!(info.url_type, YouTubeUrlType::Video));
    }

    #[test]
    fn test_parse_embed_youtube_url() {
        let info = extract_youtube_info("https://www.youtube.com/embed/dQw4w9WgXcQ").unwrap();
        assert_eq!(info.video_id, Some("dQw4w9WgXcQ".to_string()));
    }

    #[test]
    fn test_parse_youtube_url_with_playlist() {
        let info =
            extract_youtube_info("https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL123").unwrap();
        assert_eq!(info.video_id, Some("dQw4w9WgXcQ".to_string()));
        assert_eq!(info.playlist_id, Some("PL123".to_string()));
    }

    #[test]
    fn test_parse_channel_url() {
        let info = extract_youtube_info("https://www.youtube.com/channel/UC123").unwrap();
        assert_eq!(info.channel_id, Some("UC123".to_string()));
        assert!(matches!(info.url_type, YouTubeUrlType::Channel));
    }

    #[test]
    fn test_parse_handle_url() {
        let info = extract_youtube_info("https://www.youtube.com/@LinusTechTips").unwrap();
        assert_eq!(info.channel_id, Some("@LinusTechTips".to_string()));
        assert!(matches!(info.url_type, YouTubeUrlType::Channel));
    }

    #[test]
    fn test_parse_playlist_url() {
        let info = extract_youtube_info(
            "https://www.youtube.com/playlist?list=PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf",
        )
        .unwrap();
        assert_eq!(
            info.playlist_id,
            Some("PLrAXtmErZgOeiKm4sgNOknGvNjby9efdf".to_string())
        );
        assert!(matches!(info.url_type, YouTubeUrlType::Playlist));
    }

    #[test]
    fn test_parse_invalid_url() {
        assert!(extract_youtube_info("https://example.com").is_none());
        assert!(extract_youtube_info("not a url").is_none());
    }
}
