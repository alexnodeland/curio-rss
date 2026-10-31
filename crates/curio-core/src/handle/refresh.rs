//! Feed refresh: conditional fetch, parse, ingest, background hydration,
//! and the per-feed refresh serialization.

use std::sync::{Arc, PoisonError};
use std::time::Instant;

use tokio::sync::Mutex as AsyncMutex;

use curio_types::Timestamp;

use crate::content;
use crate::feeds::ParsedEntry;
use crate::fetch::FetchRequest;
use crate::model::{
    ArticleContent, ArticleId, Feed, FeedId, FeedStatus, FetchRecord, FetchStatus, NewArticle,
};

use super::{CoreError, CoreHandle, RefreshOutcome};

impl CoreHandle {
    /// Refreshes one feed: policed conditional GET → parse → sanitize →
    /// upsert → fetch-log row. Fetch/parse failures are *outcomes*
    /// (recorded, validators preserved), not errors; only storage-level
    /// failures surface as `Err`.
    ///
    /// Refreshes of the **same feed are serialized**: concurrent callers
    /// queue on a per-feed lock, and the feed snapshot below is read
    /// inside the critical section — so an error-path validator preserve
    /// can never write back a stale etag/last-modified over the fresher
    /// pair a parallel successful refresh just stored (the
    /// conditional-GET validator race, closed for the first concurrent
    /// head). Different feeds still refresh in parallel.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub async fn refresh_feed(&self, id: FeedId) -> Result<RefreshOutcome, CoreError> {
        let lock = self.refresh_lock(id);
        let serialized = lock.lock().await;
        let feed = self
            .storage
            .get_feed(id)?
            .ok_or(CoreError::NotFound { entity: "feed" })?;
        if feed.status == FeedStatus::Dead {
            return Ok(RefreshOutcome {
                feed_id: id,
                status: FetchStatus::Error,
                http_status: None,
                new_articles: 0,
                updated_articles: 0,
                error: Some("feed is dead (HTTP 410) — refusing to fetch".to_owned()),
            });
        }

        let started = Instant::now();
        let request = FetchRequest {
            url: feed.url.clone(),
            allow_private_network: feed.allow_private_network,
            etag: feed.etag.clone(),
            last_modified: feed.last_modified.clone(),
            bearer: None,
        };
        let fetched = self.client.fetch(&request).await;
        let now = Timestamp::now();
        let duration_ms = u32::try_from(started.elapsed().as_millis()).ok();

        let failed = |http_status: Option<u16>, error: String| RefreshOutcome {
            feed_id: id,
            status: FetchStatus::Error,
            http_status,
            new_articles: 0,
            updated_articles: 0,
            error: Some(error),
        };

        // Conditional-GET validators are preserved on every non-ingest
        // path — error, 304, 410, unparseable body. Only a successfully
        // *parsed* 2xx adopts the response's validators (the sketch
        // clobbered them on any failure).
        let mut hydrate_pending: Vec<ArticleId> = Vec::new();
        let outcome = match fetched {
            Err(err) => {
                self.preserve_validators(id, &feed, now)?;
                failed(None, err.to_string())
            }
            Ok(response) if response.is_not_modified() => {
                self.preserve_validators(id, &feed, now)?;
                RefreshOutcome {
                    feed_id: id,
                    status: FetchStatus::NotModified,
                    http_status: Some(response.status),
                    new_articles: 0,
                    updated_articles: 0,
                    error: None,
                }
            }
            Ok(response) if response.is_gone() => {
                // 410: the feed is gone for good — auto-pause forever.
                self.storage.set_feed_status(id, FeedStatus::Dead)?;
                self.preserve_validators(id, &feed, now)?;
                failed(
                    Some(response.status),
                    "HTTP 410 Gone — feed auto-paused as dead".to_owned(),
                )
            }
            Ok(response) if response.is_success() => {
                match crate::feeds::parse_feed(&response.body, Some(&response.final_url)) {
                    Err(err) => {
                        // A body we could not parse must not update the
                        // validators, or a broken feed 304s forever.
                        self.preserve_validators(id, &feed, now)?;
                        failed(Some(response.status), err.to_string())
                    }
                    Ok(parsed) => {
                        let (outcome, pending) =
                            self.ingest_parsed(id, &feed, &response, parsed, now)?;
                        hydrate_pending = pending;
                        outcome
                    }
                }
            }
            Ok(response) => {
                self.preserve_validators(id, &feed, now)?;
                failed(Some(response.status), format!("HTTP {}", response.status))
            }
        };

        self.storage.record_fetch(FetchRecord {
            feed_id: id,
            fetched_at: now,
            status: outcome.status,
            http_status: outcome.http_status,
            error: outcome.error.clone(),
            articles_new: u32::try_from(outcome.new_articles).unwrap_or(u32::MAX),
            duration_ms,
        })?;
        // Full-text mode's refresh hook: hydrate the articles this refresh
        // *inserted* (never the whole backlog) from their source pages,
        // outside the per-feed lock. Best-effort per article — a dead page
        // keeps its feed excerpt, and hydrate_article's never-clobber
        // guard already protects feeds that ship full content.
        drop(serialized);
        self.hydrate_batch(hydrate_pending).await;
        Ok(outcome)
    }

    /// Hydrates the articles a refresh just inserted (full-text mode),
    /// best-effort per article. A rate-limit error stops the whole batch
    /// — every remaining hydrate would fail the same way; the articles
    /// keep their feed excerpts and a later manual load-full picks them
    /// up.
    async fn hydrate_batch(&self, pending: Vec<ArticleId>) {
        for article_id in pending {
            match self.hydrate_article(article_id).await {
                Ok(_) => {}
                Err(CoreError::RateLimited { .. } | CoreError::RedditAuth { .. }) => {
                    // Deterministic for every remaining article in the
                    // batch — stop now rather than grind through it.
                    tracing::warn!("full-text hydrate blocked upstream; stopping the batch");
                    break;
                }
                Err(err) => {
                    tracing::warn!(
                        article = %article_id,
                        %err,
                        "full-text hydrate failed; keeping the feed excerpt"
                    );
                }
            }
        }
    }

    /// Stamps `last_fetched_at` while keeping the stored conditional-GET
    /// validators exactly as they were.
    fn preserve_validators(
        &self,
        id: FeedId,
        feed: &Feed,
        now: Timestamp,
    ) -> Result<(), CoreError> {
        self.storage.update_feed_fetch_state(
            id,
            feed.etag.clone(),
            feed.last_modified.clone(),
            now,
        )?;
        Ok(())
    }

    /// The happy path of a refresh: pipeline every entry, batch-upsert,
    /// fill in feed metadata, adopt permanent redirects and the response
    /// validators. The second return value is the row ids the upsert
    /// *inserted* — the full-text hydration queue (empty unless the feed
    /// has full-text mode on).
    fn ingest_parsed(
        &self,
        id: FeedId,
        feed: &Feed,
        response: &crate::fetch::FetchResponse,
        parsed: crate::feeds::ParsedFeed,
        now: Timestamp,
    ) -> Result<(RefreshOutcome, Vec<ArticleId>), CoreError> {
        let articles: Vec<NewArticle> = parsed
            .entries
            .into_iter()
            .map(|entry| ingest_entry(id, &feed.url, entry))
            .collect();
        // Which entries are new, decided BEFORE the upsert (afterwards
        // everything exists). Same-feed refreshes are serialized by the
        // caller's per-feed lock, so this read-then-write cannot race. A
        // link-less entry has no page of its own (its source URL is the
        // feed's — see `ingest_entry`), so it is never queued: hydrating it
        // would fetch the feed XML as if it were an article.
        let fresh_keys: Vec<String> = if feed.fetch_full_text {
            articles
                .iter()
                .filter(|article| !is_feed_own_url(&article.source_url, feed))
                .filter(|article| {
                    self.storage
                        .article_id_by_dedupe_key(Some(id), &article.dedupe_key)
                        .ok()
                        .flatten()
                        .is_none()
                })
                .map(|article| article.dedupe_key.clone())
                .collect()
        } else {
            Vec::new()
        };
        let upserted = self.storage.upsert_articles(articles)?;
        let mut hydrate_pending = Vec::with_capacity(fresh_keys.len());
        for key in &fresh_keys {
            if let Some(article_id) = self.storage.article_id_by_dedupe_key(Some(id), key)? {
                hydrate_pending.push(article_id);
            }
        }
        self.storage.update_feed_metadata(
            id,
            parsed.meta.title,
            parsed.meta.site_url,
            parsed.meta.description,
        )?;
        if response.permanent_redirect && response.final_url != feed.url {
            // The URL is the identity key of feed.added/feed.removed;
            // adoption stages a removed/added pair — flush it now.
            self.storage
                .update_feed_url(id, response.final_url.clone())?;
            self.emit();
        }
        self.storage.update_feed_fetch_state(
            id,
            response.etag.clone(),
            response.last_modified.clone(),
            now,
        )?;
        Ok((
            RefreshOutcome {
                feed_id: id,
                status: FetchStatus::Ok,
                http_status: Some(response.status),
                new_articles: upserted.inserted,
                updated_articles: upserted.updated,
                error: None,
            },
            hydrate_pending,
        ))
    }

    /// Refreshes every active feed, sequentially (the policed client's
    /// per-host politeness paces same-host feeds anyway). Per-feed
    /// failures land in their outcome rows.
    ///
    /// # Errors
    ///
    /// Storage errors only.
    pub async fn refresh_all(&self) -> Result<Vec<RefreshOutcome>, CoreError> {
        let feeds = self.storage.list_feeds()?;
        let mut outcomes = Vec::new();
        for feed in feeds {
            if feed.status != FeedStatus::Active {
                continue;
            }
            outcomes.push(self.refresh_feed(feed.id).await?);
        }
        Ok(outcomes)
    }

    /// The feed's refresh lock, minted on first use. The registry mutex
    /// guards only the map lookup — the async lock itself is awaited
    /// outside it, so a slow refresh never blocks other feeds.
    fn refresh_lock(&self, id: FeedId) -> Arc<AsyncMutex<()>> {
        let mut locks = self
            .refresh_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(locks.entry(id).or_default())
    }
}

/// Whether `url` is `feed`'s own URL — what a link-less entry's
/// `source_url` holds (see [`ingest_entry`]). Such an article has no page
/// of its own to hydrate.
pub(super) fn is_feed_own_url(url: &str, feed: &Feed) -> bool {
    url == feed.url
}

/// Turns a parsed entry into a storage row via the content pipeline
/// (sanitize → extract whole pages only → text — the D6 invariant lives
/// here).
///
/// An entry without a link of its own gets the feed URL as `source_url`:
/// the published contracts require every article to carry a source URI,
/// and the feed is the only one it has. That value marks the article as
/// link-less — full-text hydration recognizes it and never fetches it.
fn ingest_entry(feed_id: FeedId, feed_url: &str, entry: ParsedEntry) -> NewArticle {
    let base = entry.link.as_deref().unwrap_or(feed_url).to_owned();
    let processed = content::process(&entry.content_html, Some(&base));
    // Prefer the feed's own declared image (media:thumbnail / media:content /
    // enclosure); fall back to the first inline <img> in the sanitized body
    // (already base-resolved). Either way it's a URL loaded through the
    // policed image cache, never fetched directly.
    let lead_image = entry
        .lead_image
        .or_else(|| content::first_image(&processed.html));
    NewArticle {
        feed_id: Some(feed_id),
        dedupe_key: entry.dedupe_key,
        title: entry.title,
        source_url: entry.link.unwrap_or(base),
        author: entry.author,
        published_at: entry.published,
        content: ArticleContent {
            html: processed.html,
            text: processed.text,
        },
        lang: entry.lang,
        word_count: Some(processed.word_count),
        source_updated_at: entry.updated,
        lead_image,
    }
}
