//! Reddit JSON API integration.

use serde::{Deserialize, Serialize};

const USER_AGENT: &str = "Curio Reader/0.1 (RSS Desktop Client)";

/// Reddit post data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedditPost {
    pub reddit_id: String,
    pub subreddit: String,
    pub author: String,
    pub author_flair: Option<String>,
    pub title: String,
    pub score: i32,
    pub upvote_ratio: f32,
    pub num_comments: i32,
    pub permalink: String,
    pub is_self: bool,
    pub selftext_html: Option<String>,
    pub link_url: Option<String>,
    pub thumbnail: Option<String>,
    pub created_utc: i64,
    pub flair_text: Option<String>,
}

/// Reddit comment
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedditComment {
    pub reddit_id: String,
    pub author: String,
    pub author_flair: Option<String>,
    pub body_html: String,
    pub score: i32,
    pub created_utc: i64,
    pub depth: i32,
    pub replies: Vec<RedditComment>,
    pub is_collapsed: bool,
}

/// Reddit thread with post and comments
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedditThread {
    pub post: RedditPost,
    pub comments: Vec<RedditComment>,
}

/// Comment sort options
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum CommentSort {
    #[default]
    Best,
    Top,
    New,
    Controversial,
    Old,
}

/// Fetch a Reddit thread with comments
pub async fn fetch_thread(
    subreddit: &str,
    post_id: &str,
    sort: CommentSort,
) -> Result<RedditThread, crate::error::InfraError> {
    let sort_str = match sort {
        CommentSort::Best => "confidence",
        CommentSort::Top => "top",
        CommentSort::New => "new",
        CommentSort::Controversial => "controversial",
        CommentSort::Old => "old",
    };

    let url = format!(
        "https://www.reddit.com/r/{}/comments/{}.json?sort={}&limit=500&raw_json=1",
        subreddit, post_id, sort_str
    );

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?;

    if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .map(std::time::Duration::from_secs);

        return Err(crate::error::InfraError::RateLimited { retry_after });
    }

    let json: serde_json::Value = response.json().await?;

    // Reddit returns an array: [post_listing, comments_listing]
    let post = parse_post(&json[0])?;
    let comments = parse_comments(&json[1])?;

    Ok(RedditThread { post, comments })
}

/// Parse post data from Reddit JSON
fn parse_post(listing: &serde_json::Value) -> Result<RedditPost, crate::error::InfraError> {
    let data = &listing["data"]["children"][0]["data"];

    Ok(RedditPost {
        reddit_id: data["id"].as_str().unwrap_or_default().to_string(),
        subreddit: data["subreddit"].as_str().unwrap_or_default().to_string(),
        author: data["author"].as_str().unwrap_or_default().to_string(),
        author_flair: data["author_flair_text"].as_str().map(String::from),
        title: data["title"].as_str().unwrap_or_default().to_string(),
        score: data["score"].as_i64().unwrap_or(0) as i32,
        upvote_ratio: data["upvote_ratio"].as_f64().unwrap_or(0.0) as f32,
        num_comments: data["num_comments"].as_i64().unwrap_or(0) as i32,
        permalink: data["permalink"].as_str().unwrap_or_default().to_string(),
        is_self: data["is_self"].as_bool().unwrap_or(false),
        selftext_html: data["selftext_html"].as_str().map(String::from),
        link_url: data["url"].as_str().map(String::from),
        thumbnail: data["thumbnail"]
            .as_str()
            .filter(|t| t.starts_with("http"))
            .map(String::from),
        created_utc: data["created_utc"].as_f64().unwrap_or(0.0) as i64,
        flair_text: data["link_flair_text"].as_str().map(String::from),
    })
}

/// Parse comments from Reddit JSON
fn parse_comments(listing: &serde_json::Value) -> Result<Vec<RedditComment>, crate::error::InfraError> {
    let children = &listing["data"]["children"];

    if !children.is_array() {
        return Ok(Vec::new());
    }

    let comments: Vec<RedditComment> = children
        .as_array()
        .unwrap_or(&Vec::new())
        .iter()
        .filter(|c| c["kind"].as_str() == Some("t1"))
        .filter_map(|c| parse_comment(&c["data"], 0).ok())
        .collect();

    Ok(comments)
}

/// Parse a single comment (recursive)
fn parse_comment(data: &serde_json::Value, depth: i32) -> Result<RedditComment, crate::error::InfraError> {
    let replies = if let Some(replies_data) = data["replies"]["data"]["children"].as_array() {
        replies_data
            .iter()
            .filter(|r| r["kind"].as_str() == Some("t1"))
            .filter_map(|r| parse_comment(&r["data"], depth + 1).ok())
            .collect()
    } else {
        Vec::new()
    };

    Ok(RedditComment {
        reddit_id: data["id"].as_str().unwrap_or_default().to_string(),
        author: data["author"].as_str().unwrap_or_default().to_string(),
        author_flair: data["author_flair_text"].as_str().map(String::from),
        body_html: data["body_html"].as_str().unwrap_or_default().to_string(),
        score: data["score"].as_i64().unwrap_or(0) as i32,
        created_utc: data["created_utc"].as_f64().unwrap_or(0.0) as i64,
        depth,
        replies,
        is_collapsed: data["collapsed"].as_bool().unwrap_or(false),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_comment_sort_serialization() {
        let sort = CommentSort::Top;
        let json = serde_json::to_string(&sort).unwrap();
        assert_eq!(json, "\"top\"");
    }

    #[test]
    fn test_reddit_post_serialization() {
        let post = RedditPost {
            reddit_id: "abc123".to_string(),
            subreddit: "rust".to_string(),
            author: "user".to_string(),
            author_flair: None,
            title: "Test Post".to_string(),
            score: 100,
            upvote_ratio: 0.95,
            num_comments: 50,
            permalink: "/r/rust/comments/abc123".to_string(),
            is_self: true,
            selftext_html: Some("<p>Content</p>".to_string()),
            link_url: None,
            thumbnail: None,
            created_utc: 1234567890,
            flair_text: None,
        };

        let json = serde_json::to_string(&post).unwrap();
        assert!(json.contains("abc123"));
        assert!(json.contains("rust"));
    }
}
