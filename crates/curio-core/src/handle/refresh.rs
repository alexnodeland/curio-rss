//! Feed refresh: conditional fetch, parse, ingest, background hydration,
//! and the per-feed refresh serialization.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use curio_types::Timestamp;

use crate::content;
use crate::feeds::ParsedEntry;
use crate::fetch::{FetchConfig, FetchRequest};
use crate::lock::{FileLock, FileLockGuard, LockError};
use crate::model::{
    ArticleContent, ArticleId, Feed, FeedId, FeedStatus, FetchRecord, FetchStatus, NewArticle,
};

use super::{CoreError, CoreHandle, LOCKS_DIR, RefreshOutcome};

impl CoreHandle {
    /// Refreshes one feed: policed conditional GET → parse → sanitize →
    /// upsert → fetch-log row. Fetch/parse failures are *outcomes*
    /// (recorded, validators preserved), not errors; only storage-level
    /// failures surface as `Err`.
    ///
    /// Refreshes of the **same feed are serialized**, across processes
    /// too: concurrent callers in this process queue on a per-feed mutex,
    /// and the one at the front then takes the feed's lock file under
    /// `.curio/locks/`, which another process on the profile (the desktop
    /// app and a CLI `curio fetch`) takes as well. The feed snapshot below
    /// is read inside that critical section, so an error-path validator
    /// preserve can never write back a stale etag/last-modified over the
    /// fresher pair a parallel successful refresh just stored (the
    /// conditional-GET validator race). Different feeds still refresh in
    /// parallel.
    ///
    /// Should another process hold the feed's lock longer than one
    /// refresh can take (it is stuck), the call gives up without fetching
    /// and returns a [`FetchStatus::Error`] outcome saying so; nothing is
    /// recorded.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors;
    /// [`CoreError::Io`] when the lock file cannot be created.
    pub async fn refresh_feed(&self, id: FeedId) -> Result<RefreshOutcome, CoreError> {
        // Unknown feeds fail before a lock file is minted for them.
        if self.storage.get_feed(id)?.is_none() {
            return Err(CoreError::NotFound { entity: "feed" });
        }
        let lock = self.refresh_lock(id);
        let serialized = lock.lock().await;
        let Some(across_processes) = self.refresh_file_lock(id).await? else {
            return Ok(not_fetched(
                id,
                "another Curio process is still refreshing this feed — skipped",
            ));
        };
        let feed = self
            .storage
            .get_feed(id)?
            .ok_or(CoreError::NotFound { entity: "feed" })?;
        if feed.status == FeedStatus::Dead {
            return Ok(not_fetched(
                id,
                "feed is dead (HTTP 410) — refusing to fetch",
            ));
        }

        let started = Instant::now();
        let request = FetchRequest {
            url: feed.url.clone(),
            allow_private_network: feed.allow_private_network,
            etag: feed.etag.clone(),
            last_modified: feed.last_modified.clone(),
            bearer: None,
            ..FetchRequest::default()
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
        // outside the per-feed locks. Best-effort per article — a dead page
        // keeps its feed excerpt, and hydrate_article's never-clobber
        // guard already protects feeds that ship full content.
        drop(across_processes);
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
        mut parsed: crate::feeds::ParsedFeed,
        now: Timestamp,
    ) -> Result<(RefreshOutcome, Vec<ArticleId>), CoreError> {
        // A reused guid's keys are chosen per fetch; settle them against
        // what earlier fetches stored so no item changes key (and splits
        // into a duplicate article) when the guid's sharing changes.
        let stored = self
            .storage
            .source_urls_by_dedupe_key(Some(id), parsed.settle_lookups())?;
        parsed.settle_keys(&feed.url, |key| stored.get(key).map(String::as_str));
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

    /// Takes the feed's cross-process refresh lock, waiting up to
    /// [`lock_wait`] for another process's refresh of the feed to end.
    /// `None` when it does not end in time. The lock file is opened per
    /// refresh (one open file per subscription would not scale), and
    /// [`FileLock`] copes with a lock file deleted in between.
    async fn refresh_file_lock(&self, id: FeedId) -> Result<Option<FileLockGuard>, CoreError> {
        let path = feed_lock_path(&self.profile_dir, id);
        let io = |source| CoreError::Io {
            path: path.clone(),
            source,
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let lock = FileLock::open(&path).map_err(io)?;
        match lock.acquire_async(self.refresh_lock_wait).await {
            Ok(guard) => Ok(Some(guard)),
            Err(LockError::TimedOut { waited }) => {
                tracing::warn!(
                    feed = %id,
                    waited_ms = waited.as_millis(),
                    "another Curio process kept refreshing this feed; skipped"
                );
                Ok(None)
            }
            Err(LockError::Io(source)) => Err(io(source)),
        }
    }
}

/// The outcome of a refresh that refused to fetch (nothing is recorded).
fn not_fetched(feed_id: FeedId, why: &str) -> RefreshOutcome {
    RefreshOutcome {
        feed_id,
        status: FetchStatus::Error,
        http_status: None,
        new_articles: 0,
        updated_articles: 0,
        error: Some(why.to_owned()),
    }
}

/// The feed's cross-process refresh lock file.
pub(super) fn feed_lock_path(profile_dir: &Path, id: FeedId) -> PathBuf {
    profile_dir
        .join(".curio")
        .join(LOCKS_DIR)
        .join(format!("feed-{id}.lock"))
}

/// How long a refresh waits for another process's refresh of the same
/// feed. The holder's critical section is one policed fetch — at most
/// the longest politeness delay, then every redirect hop bounded by the
/// connect and request timeouts — plus the ingest. Twice the fetch bound
/// leaves the ingest ample room; a holder still busy after that is stuck.
pub(super) fn lock_wait(config: &FetchConfig) -> Duration {
    let per_hop = config
        .connect_timeout
        .saturating_add(config.request_timeout);
    let fetch = per_hop.saturating_mul(config.max_redirects.saturating_add(1));
    let politeness = config
        .host_overrides
        .iter()
        .filter_map(|host| host.politeness_delay)
        .fold(config.politeness_delay, Duration::max);
    fetch.saturating_add(politeness).saturating_mul(2)
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::CoreOptions;
    use crate::model::NewFeed;

    /// Fetch timeouts this short make the refresh-lock wait 200 ms.
    fn quick_fetch() -> FetchConfig {
        FetchConfig {
            connect_timeout: Duration::from_millis(50),
            request_timeout: Duration::from_millis(50),
            max_redirects: 0,
            politeness_delay: Duration::ZERO,
            host_overrides: Vec::new(),
            ..FetchConfig::default()
        }
    }

    #[test]
    fn the_lock_wait_covers_a_whole_fetch_twice() {
        assert_eq!(lock_wait(&quick_fetch()), Duration::from_millis(200));
        // The defaults: 6 hops × (10 s + 30 s) plus reddit's 6.5 s
        // politeness, twice: 2 × 246.5 s.
        assert_eq!(lock_wait(&FetchConfig::default()), Duration::from_secs(493));
    }

    /// Another process holding a feed's refresh lock past the wait (it is
    /// stuck): the refresh gives up without fetching or recording
    /// anything, and the next one, once the lock is free, goes ahead.
    #[tokio::test]
    async fn a_refresh_skips_a_feed_another_process_keeps_locked() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500))
            .expect(1)
            .mount(&server)
            .await;
        let profile = tempfile::tempdir().unwrap();
        let core = CoreHandle::open_with(
            profile.path(),
            CoreOptions {
                fetch: quick_fetch(),
                ..CoreOptions::default()
            },
        )
        .unwrap();
        let feed = core
            .add_feed(NewFeed {
                url: format!("{}/feed.xml", server.uri()),
                title: None,
                tags: Vec::new(),
            })
            .unwrap();
        core.set_feed_allow_private_network(feed.id, true).unwrap();

        let path = feed_lock_path(profile.path(), feed.id);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let other_process = FileLock::open(&path).unwrap();
        let held = other_process.acquire(Duration::from_secs(1)).unwrap();

        let skipped = core.refresh_feed(feed.id).await.unwrap();
        assert_eq!(skipped.status, FetchStatus::Error);
        assert!(
            skipped
                .error
                .as_deref()
                .unwrap()
                .contains("another Curio process"),
            "{skipped:?}"
        );
        assert!(
            core.recent_fetches(feed.id, 10).unwrap().is_empty(),
            "nothing recorded"
        );

        drop(held);
        let refreshed = core.refresh_feed(feed.id).await.unwrap();
        assert_eq!(refreshed.http_status, Some(500));
        assert_eq!(core.recent_fetches(feed.id, 10).unwrap().len(), 1);

        // Unsubscribing deletes the lock file; an unknown feed never
        // mints one.
        core.remove_feed(feed.id).unwrap();
        assert!(!path.exists());
        assert!(matches!(
            core.refresh_feed(feed.id).await,
            Err(CoreError::NotFound { .. })
        ));
        assert!(!path.exists());
    }
}
