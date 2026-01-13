//! YouTube metadata extraction via yt-dlp.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::process::Command;

/// YouTube video metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeMetadata {
    pub id: String,
    pub title: String,
    pub description: String,
    pub channel: String,
    pub channel_id: String,
    pub channel_url: String,
    pub channel_follower_count: Option<i64>,
    pub upload_date: String,
    pub view_count: Option<i64>,
    pub like_count: Option<i64>,
    pub comment_count: Option<i64>,
    pub duration: i64,
    pub categories: Vec<String>,
    pub tags: Vec<String>,
    pub chapters: Option<Vec<Chapter>>,
    pub thumbnails: Vec<Thumbnail>,
}

/// Video chapter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub start_time: f64,
    pub end_time: f64,
    pub title: String,
}

/// Thumbnail information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thumbnail {
    pub url: String,
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub id: String,
}

/// YouTube comment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeComment {
    pub id: String,
    pub text: String,
    pub author: String,
    pub author_id: String,
    pub author_thumbnail: String,
    pub author_is_uploader: bool,
    pub parent: Option<String>,
    pub like_count: Option<i64>,
    pub is_pinned: bool,
    pub is_favorited: bool,
    pub timestamp: Option<i64>,
}

/// Get the path to the bundled yt-dlp binary
fn ytdlp_path() -> PathBuf {
    // In development, look for it in the bin directory
    // In production, it's bundled with the app
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.to_path_buf()))
        .unwrap_or_default();

    #[cfg(target_os = "windows")]
    let binary_name = "yt-dlp.exe";
    #[cfg(not(target_os = "windows"))]
    let binary_name = "yt-dlp";

    // Check bundled location first
    let bundled = exe_dir.join("bin").join(binary_name);
    if bundled.exists() {
        return bundled;
    }

    // Fall back to system PATH
    PathBuf::from(binary_name)
}

/// Fetch video metadata using yt-dlp
pub async fn fetch_metadata(video_url: &str) -> Result<YouTubeMetadata, crate::error::InfraError> {
    let output = tokio::task::spawn_blocking({
        let url = video_url.to_string();
        move || {
            Command::new(ytdlp_path())
                .args([
                    "--dump-json",
                    "--no-download",
                    "--no-warnings",
                    "--no-playlist",
                    &url,
                ])
                .output()
        }
    })
    .await
    .map_err(|e| crate::error::InfraError::YtDlp(e.to_string()))?
    .map_err(|e| crate::error::InfraError::YtDlp(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::InfraError::YtDlp(format!(
            "yt-dlp failed: {}",
            stderr
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    Ok(YouTubeMetadata {
        id: json["id"].as_str().unwrap_or_default().to_string(),
        title: json["title"].as_str().unwrap_or_default().to_string(),
        description: json["description"].as_str().unwrap_or_default().to_string(),
        channel: json["channel"].as_str().unwrap_or_default().to_string(),
        channel_id: json["channel_id"].as_str().unwrap_or_default().to_string(),
        channel_url: json["channel_url"].as_str().unwrap_or_default().to_string(),
        channel_follower_count: json["channel_follower_count"].as_i64(),
        upload_date: json["upload_date"].as_str().unwrap_or_default().to_string(),
        view_count: json["view_count"].as_i64(),
        like_count: json["like_count"].as_i64(),
        comment_count: json["comment_count"].as_i64(),
        duration: json["duration"].as_i64().unwrap_or(0),
        categories: json["categories"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        tags: json["tags"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
            .unwrap_or_default(),
        chapters: json["chapters"].as_array().map(|chapters| {
            chapters
                .iter()
                .filter_map(|c| {
                    Some(Chapter {
                        start_time: c["start_time"].as_f64()?,
                        end_time: c["end_time"].as_f64()?,
                        title: c["title"].as_str()?.to_string(),
                    })
                })
                .collect()
        }),
        thumbnails: json["thumbnails"]
            .as_array()
            .map(|thumbs| {
                thumbs
                    .iter()
                    .filter_map(|t| {
                        Some(Thumbnail {
                            url: t["url"].as_str()?.to_string(),
                            width: t["width"].as_i64().map(|w| w as i32),
                            height: t["height"].as_i64().map(|h| h as i32),
                            id: t["id"].as_str().unwrap_or_default().to_string(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// Fetch video comments using yt-dlp
pub async fn fetch_comments(video_url: &str) -> Result<Vec<YouTubeComment>, crate::error::InfraError> {
    let output = tokio::task::spawn_blocking({
        let url = video_url.to_string();
        move || {
            Command::new(ytdlp_path())
                .args([
                    "--dump-json",
                    "--write-comments",
                    "--no-download",
                    "--no-warnings",
                    "--no-playlist",
                    &url,
                ])
                .output()
        }
    })
    .await
    .map_err(|e| crate::error::InfraError::YtDlp(e.to_string()))?
    .map_err(|e| crate::error::InfraError::YtDlp(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(crate::error::InfraError::YtDlp(format!(
            "yt-dlp failed: {}",
            stderr
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)?;

    let comments = json["comments"]
        .as_array()
        .map(|comments| {
            comments
                .iter()
                .filter_map(|c| {
                    Some(YouTubeComment {
                        id: c["id"].as_str()?.to_string(),
                        text: c["text"].as_str().unwrap_or_default().to_string(),
                        author: c["author"].as_str().unwrap_or_default().to_string(),
                        author_id: c["author_id"].as_str().unwrap_or_default().to_string(),
                        author_thumbnail: c["author_thumbnail"]
                            .as_str()
                            .unwrap_or_default()
                            .to_string(),
                        author_is_uploader: c["author_is_uploader"].as_bool().unwrap_or(false),
                        parent: c["parent"].as_str().map(String::from),
                        like_count: c["like_count"].as_i64(),
                        is_pinned: c["is_pinned"].as_bool().unwrap_or(false),
                        is_favorited: c["is_favorited"].as_bool().unwrap_or(false),
                        timestamp: c["timestamp"].as_i64(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(comments)
}

/// Get yt-dlp version
pub fn get_version() -> Result<String, crate::error::InfraError> {
    let output = Command::new(ytdlp_path())
        .arg("--version")
        .output()
        .map_err(|e| crate::error::InfraError::YtDlp(e.to_string()))?;

    if !output.status.success() {
        return Err(crate::error::InfraError::YtDlp(
            "Failed to get yt-dlp version".to_string(),
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_youtube_metadata_serialization() {
        let metadata = YouTubeMetadata {
            id: "dQw4w9WgXcQ".to_string(),
            title: "Test Video".to_string(),
            description: "A test video".to_string(),
            channel: "Test Channel".to_string(),
            channel_id: "UC123".to_string(),
            channel_url: "https://youtube.com/channel/UC123".to_string(),
            channel_follower_count: Some(1000000),
            upload_date: "20240115".to_string(),
            view_count: Some(1000000),
            like_count: Some(50000),
            comment_count: Some(1000),
            duration: 212,
            categories: vec!["Music".to_string()],
            tags: vec!["test".to_string()],
            chapters: None,
            thumbnails: vec![],
        };

        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("dQw4w9WgXcQ"));
        assert!(json.contains("Test Video"));
    }

    #[test]
    fn test_chapter_serialization() {
        let chapter = Chapter {
            start_time: 0.0,
            end_time: 60.0,
            title: "Introduction".to_string(),
        };

        let json = serde_json::to_string(&chapter).unwrap();
        assert!(json.contains("Introduction"));
    }
}
