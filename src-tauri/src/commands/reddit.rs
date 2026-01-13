//! Reddit-specific commands.

use tauri::State;

use crate::commands::AppState;
use crate::error::CommandError;
use crate::platform::reddit::{CommentSort, RedditThread};

/// Fetch a Reddit thread with comments
#[tauri::command]
pub async fn fetch_reddit_thread(
    subreddit: &str,
    post_id: &str,
    sort: Option<CommentSort>,
    _state: State<'_, AppState>,
) -> Result<RedditThread, CommandError> {
    let sort = sort.unwrap_or_default();
    let thread = crate::platform::reddit::fetch_thread(subreddit, post_id, sort).await?;
    Ok(thread)
}

/// Extract Reddit post info from a URL
#[tauri::command]
pub fn parse_reddit_url(url: &str) -> Result<RedditUrlInfo, CommandError> {
    let info = extract_reddit_info(url)
        .ok_or_else(|| CommandError::Validation("Invalid Reddit URL".to_string()))?;
    Ok(info)
}

/// Parsed Reddit URL information
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RedditUrlInfo {
    pub subreddit: String,
    pub post_id: Option<String>,
    pub is_comment: bool,
}

/// Extract subreddit and post ID from various Reddit URL formats
fn extract_reddit_info(url: &str) -> Option<RedditUrlInfo> {
    let url = url.trim();

    // Handle r/subreddit format
    if let Some(sub) = url.strip_prefix("r/") {
        return Some(RedditUrlInfo {
            subreddit: sub.split('/').next()?.to_string(),
            post_id: None,
            is_comment: false,
        });
    }

    // Handle full URLs
    let patterns = [
        // https://reddit.com/r/subreddit/comments/post_id/title
        r"reddit\.com/r/([^/]+)/comments/([^/]+)",
        // https://old.reddit.com/r/subreddit/comments/post_id
        r"old\.reddit\.com/r/([^/]+)/comments/([^/]+)",
        // https://www.reddit.com/r/subreddit
        r"reddit\.com/r/([^/]+)/?$",
    ];

    for pattern in &patterns {
        if let Ok(re) = regex::Regex::new(pattern) {
            if let Some(caps) = re.captures(url) {
                let subreddit = caps.get(1)?.as_str().to_string();
                let post_id = caps.get(2).map(|m| m.as_str().to_string());
                return Some(RedditUrlInfo {
                    subreddit,
                    post_id,
                    is_comment: url.contains("/comment/"),
                });
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_reddit_url_subreddit_only() {
        let info = extract_reddit_info("https://reddit.com/r/rust").unwrap();
        assert_eq!(info.subreddit, "rust");
        assert!(info.post_id.is_none());
    }

    #[test]
    fn test_parse_reddit_url_with_post() {
        let info =
            extract_reddit_info("https://reddit.com/r/rust/comments/abc123/my_post").unwrap();
        assert_eq!(info.subreddit, "rust");
        assert_eq!(info.post_id, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_reddit_url_old_reddit() {
        let info =
            extract_reddit_info("https://old.reddit.com/r/programming/comments/xyz789").unwrap();
        assert_eq!(info.subreddit, "programming");
        assert_eq!(info.post_id, Some("xyz789".to_string()));
    }

    #[test]
    fn test_parse_reddit_url_shorthand() {
        let info = extract_reddit_info("r/sveltejs").unwrap();
        assert_eq!(info.subreddit, "sveltejs");
        assert!(info.post_id.is_none());
    }

    #[test]
    fn test_parse_reddit_url_invalid() {
        assert!(extract_reddit_info("https://example.com").is_none());
        assert!(extract_reddit_info("not a url").is_none());
    }
}
