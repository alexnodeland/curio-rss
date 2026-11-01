//! Feed autodiscovery IPC types — thin DTOs over the engine's
//! [`curio_core::feeds::discover()`], which owns the whole policy: input
//! normalization, fetch-free URL rewrites, the `<link>` type allowlist,
//! capped same-origin probing and parse validation, all through the core's
//! policed client.
//!
//! The Google favicon fallback stays the frontend's opt-in concern: nothing
//! leaves the machine toward a third party unless the user turns it on
//! (PRIVACY.md).

use curio_core::CoreHandle;
use curio_core::feeds::{DiscoverError, DiscoverOptions, DiscoveredFeed, Discovery};
use serde::{Deserialize, Serialize};

use crate::error::{CommandError, ErrorCode, host_of};

/// One discovered — and validated — feed candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiscoveredFeedDto {
    /// Absolute feed URL, fetched and parsed before it was offered.
    pub url: String,
    /// The feed's own title, else the title the page gave its `<link>`.
    pub title: Option<String>,
}

/// What one autodiscovery turned up: zero or more validated feed candidates
/// plus a best-guess favicon URL (same-origin only — the Google fallback
/// is the frontend's opt-in concern).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DiscoveryDto {
    /// Feed candidates, best first (the URL itself if it *is* a feed).
    pub feeds: Vec<DiscoveredFeedDto>,
    /// A favicon URL to preview (declared icon or `/favicon.ico`), if the
    /// URL was absolute enough to resolve one.
    pub favicon: Option<String>,
}

impl From<DiscoveredFeed> for DiscoveredFeedDto {
    fn from(feed: DiscoveredFeed) -> Self {
        Self {
            url: feed.url,
            title: feed.title,
        }
    }
}

impl From<Discovery> for DiscoveryDto {
    fn from(discovery: Discovery) -> Self {
        Self {
            feeds: discovery.feeds.into_iter().map(Into::into).collect(),
            favicon: discovery.favicon,
        }
    }
}

impl From<DiscoverError> for CommandError {
    fn from(error: DiscoverError) -> Self {
        let message = error.to_string();
        match error {
            DiscoverError::Empty => Self::invalid_input(message),
            DiscoverError::InvalidUrl { input } => {
                Self::user(ErrorCode::InvalidUrl, message, true).with_param("input", input)
            }
            DiscoverError::Fetch { source, .. } => Self::fetch(&source, message),
            DiscoverError::TimedOut { url } => Self::user(ErrorCode::NetworkTimeout, message, true)
                .with_param("host", host_of(&url)),
            DiscoverError::Status { url, status } => {
                Self::user(ErrorCode::HttpStatus, message, true)
                    .with_param("host", host_of(&url))
                    .with_param("status", status.to_string())
            }
        }
    }
}

/// Discovers the feeds behind what the user typed, through the core's
/// policed client (default deny-private policy: the W1 exemption is a
/// per-feed config edit, never set from the add-feed modal).
///
/// # Errors
///
/// Blank / non-URL input is a user input error; an unreachable page is a
/// recoverable network error.
pub async fn discover(core: &CoreHandle, url: &str) -> Result<DiscoveryDto, CommandError> {
    Ok(core
        .discover_feeds(url, DiscoverOptions::default())
        .await?
        .into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::commands::testutil::temp_core_trusting_localhost;
    use crate::error::ErrorKind;

    const RSS: &str =
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Blog</title></channel></rss>"#;

    #[tokio::test]
    async fn discover_fetches_through_the_core_and_validates() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"<html><head>
                   <link rel="alternate" type="application/rss+xml" href="/feed.xml">
                   <link rel="icon" href="/icon.png">
                   </head></html>"#,
            ))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/feed.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .mount(&server)
            .await;

        let (_dir, core) = temp_core_trusting_localhost();
        let out = discover(&core, &format!("{}/", server.uri()))
            .await
            .unwrap();
        assert_eq!(
            out,
            DiscoveryDto {
                feeds: vec![DiscoveredFeedDto {
                    url: format!("{}/feed.xml", server.uri()),
                    title: Some("Blog".into()),
                }],
                favicon: Some(format!("{}/icon.png", server.uri())),
            }
        );
    }

    #[tokio::test]
    async fn an_empty_url_is_a_user_input_error() {
        let (_dir, core) = temp_core_trusting_localhost();
        let error = discover(&core, "   ").await.unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[tokio::test]
    async fn a_non_success_status_surfaces_as_a_network_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;
        let (_dir, core) = temp_core_trusting_localhost();
        let error = discover(&core, &format!("{}/", server.uri()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::HttpStatus);
        assert!(error.recoverable);
        assert_eq!(error.params.get("status").map(String::as_str), Some("500"));
        assert_eq!(
            error.params.get("host").map(String::as_str),
            Some("127.0.0.1")
        );
    }
}
