//! Background feed refresh service.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::stream::{self, StreamExt};
use tokio::sync::{broadcast, RwLock};
use uuid::Uuid;

use crate::core::feeds::parse_feed;
use crate::core::models::{Feed, FeedUpdate, RefreshResult, Settings};
use crate::storage::Database;

/// Background feed refresh service
pub struct RefreshService {
    db: Arc<Database>,
    settings: Arc<RwLock<Settings>>,
    running: Arc<AtomicBool>,
    shutdown_tx: broadcast::Sender<()>,
}

impl RefreshService {
    /// Create a new refresh service
    pub fn new(db: Arc<Database>, settings: Arc<RwLock<Settings>>) -> Self {
        let (shutdown_tx, _) = broadcast::channel(1);

        Self {
            db,
            settings,
            running: Arc::new(AtomicBool::new(false)),
            shutdown_tx,
        }
    }

    /// Start the background refresh service
    pub async fn start(&self) -> Result<(), &'static str> {
        if self.running.load(Ordering::SeqCst) {
            return Err("Service already running");
        }

        self.running.store(true, Ordering::SeqCst);

        let db = self.db.clone();
        let settings = self.settings.clone();
        let running = self.running.clone();
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        tokio::spawn(async move {
            tracing::info!("Starting background refresh service");

            while running.load(Ordering::SeqCst) {
                // Get refresh interval from settings
                let interval = {
                    let s = settings.read().await;
                    Duration::from_secs(s.refresh_interval as u64)
                };

                tokio::select! {
                    _ = tokio::time::sleep(interval) => {
                        if let Err(e) = Self::refresh_all_feeds_internal(&db, &settings).await {
                            tracing::error!("Background refresh failed: {}", e);
                        }
                    }
                    _ = shutdown_rx.recv() => {
                        tracing::info!("Shutting down background refresh service");
                        break;
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
        });

        Ok(())
    }

    /// Stop the background refresh service
    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
        let _ = self.shutdown_tx.send(());
    }

    /// Check if the service is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Manually trigger a refresh of all feeds
    pub async fn refresh_now(&self) -> Result<Vec<RefreshResult>, String> {
        Self::refresh_all_feeds_internal(&self.db, &self.settings).await
    }

    /// Internal refresh implementation
    async fn refresh_all_feeds_internal(
        db: &Database,
        settings: &RwLock<Settings>,
    ) -> Result<Vec<RefreshResult>, String> {
        let feeds = db
            .get_all_feeds()
            .map_err(|e| format!("Failed to get feeds: {}", e))?;

        if feeds.is_empty() {
            return Ok(Vec::new());
        }

        // Get concurrency from settings
        let concurrency = {
            let s = settings.read().await;
            s.fetch_concurrency as usize
        };

        let client = reqwest::Client::builder()
            .user_agent("Curio Reader/0.1 (RSS Desktop Client)")
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

        // Refresh feeds concurrently with limited parallelism
        let results: Vec<RefreshResult> = stream::iter(feeds)
            .map(|feed| {
                let client = client.clone();
                async move { Self::refresh_single_feed(&client, db, feed).await }
            })
            .buffer_unordered(concurrency)
            .collect()
            .await;

        let new_total: i32 = results.iter().map(|r| r.new_count).sum();
        let updated_total: i32 = results.iter().map(|r| r.updated_count).sum();
        let error_count = results.iter().filter(|r| r.error.is_some()).count();

        tracing::info!(
            "Background refresh complete: {} new, {} updated, {} errors",
            new_total,
            updated_total,
            error_count
        );

        Ok(results)
    }

    /// Refresh a single feed
    async fn refresh_single_feed(
        client: &reqwest::Client,
        db: &Database,
        feed: Feed,
    ) -> RefreshResult {
        let feed_id = feed.id;

        // Build request with conditional headers
        let mut request = client.get(&feed.url);

        if let Some(ref etag) = feed.etag {
            request = request.header("If-None-Match", etag);
        }
        if let Some(ref last_modified) = feed.last_modified {
            request = request.header("If-Modified-Since", last_modified);
        }

        // Send request
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                let error_msg = format!("Network error: {}", e);
                let _ = db.update_feed_fetch(feed_id, None, None, Some(&error_msg));
                return RefreshResult {
                    feed_id,
                    new_count: 0,
                    updated_count: 0,
                    unchanged: false,
                    error: Some(error_msg),
                };
            }
        };

        // Handle 304 Not Modified
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let _ = db.update_feed_fetch(
                feed_id,
                feed.etag.as_deref(),
                feed.last_modified.as_deref(),
                None,
            );

            return RefreshResult {
                feed_id,
                new_count: 0,
                updated_count: 0,
                unchanged: true,
                error: None,
            };
        }

        // Handle non-success status
        if !response.status().is_success() {
            let error_msg = format!("HTTP {}", response.status());
            let _ = db.update_feed_fetch(feed_id, None, None, Some(&error_msg));

            return RefreshResult {
                feed_id,
                new_count: 0,
                updated_count: 0,
                unchanged: false,
                error: Some(error_msg),
            };
        }

        // Get new ETag and Last-Modified
        let new_etag = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let new_last_modified = response
            .headers()
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(String::from);

        // Get response body
        let content = match response.text().await {
            Ok(c) => c,
            Err(e) => {
                let error_msg = format!("Failed to read response: {}", e);
                let _ = db.update_feed_fetch(feed_id, None, None, Some(&error_msg));
                return RefreshResult {
                    feed_id,
                    new_count: 0,
                    updated_count: 0,
                    unchanged: false,
                    error: Some(error_msg),
                };
            }
        };

        // Parse feed content
        let parsed = match parse_feed(&content, feed_id) {
            Ok(p) => p,
            Err(e) => {
                let error_msg = format!("Failed to parse feed: {}", e);
                let _ = db.update_feed_fetch(feed_id, None, None, Some(&error_msg));
                return RefreshResult {
                    feed_id,
                    new_count: 0,
                    updated_count: 0,
                    unchanged: false,
                    error: Some(error_msg),
                };
            }
        };

        // Update feed metadata if changed
        if feed.title != parsed.title || feed.description != parsed.description {
            let update = FeedUpdate {
                title: Some(parsed.title.clone()),
                description: parsed.description.clone(),
                site_url: parsed.site_url.clone(),
                icon_url: parsed.icon_url.clone(),
                ..Default::default()
            };
            let _ = db.update_feed(feed_id, &update);
        }

        // Upsert articles
        let (new_count, updated_count) = match db.upsert_articles(&parsed.articles) {
            Ok((new, updated)) => (new, updated),
            Err(e) => {
                let error_msg = format!("Failed to save articles: {}", e);
                let _ = db.update_feed_fetch(feed_id, None, None, Some(&error_msg));
                return RefreshResult {
                    feed_id,
                    new_count: 0,
                    updated_count: 0,
                    unchanged: false,
                    error: Some(error_msg),
                };
            }
        };

        // Update fetch metadata
        let _ = db.update_feed_fetch(
            feed_id,
            new_etag.as_deref(),
            new_last_modified.as_deref(),
            None,
        );

        RefreshResult {
            feed_id,
            new_count,
            updated_count,
            unchanged: new_count == 0 && updated_count == 0,
            error: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Integration tests would go here with a mock database
}
