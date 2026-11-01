//! Feed refresh: conditional fetch, parse, ingest, background hydration,
//! and the per-feed refresh serialization.

use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use futures_util::future::{Either, select};
use tokio::sync::Mutex as AsyncMutex;

use curio_types::Timestamp;

use crate::content;
use crate::feeds::ParsedEntry;
use crate::fetch::{FetchConfig, FetchError, FetchRequest, FetchResponse};
use crate::lock::{FileLock, FileLockGuard, LockError};
use crate::model::{
    ArticleContent, ArticleId, Feed, FeedId, FeedStatus, FetchErrorKind, FetchRecord, FetchStatus,
    NewArticle,
};
use crate::storage::Storage;

use super::schedule::{self, Attempt, RefreshPolicy};
use super::sweep::RefreshCancel;
use super::{CoreError, CoreHandle, LOCKS_DIR, RefreshOutcome};

impl CoreHandle {
    /// Refreshes one feed: policed conditional GET → parse → sanitize →
    /// upsert → fetch-log row. Fetch/parse failures are *outcomes*
    /// (recorded, validators preserved), not errors; only storage-level
    /// failures surface as `Err`.
    ///
    /// Everything but the network I/O — the storage reads and writes, the
    /// feed parse and the CPU-heavy sanitize/extract pipeline — runs on
    /// the blocking pool, so a large feed never stalls the async runtime
    /// it is awaited on (a current-thread runtime keeps its timers
    /// ticking).
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
    /// Every settled attempt also schedules the feed's next one (see
    /// [`RefreshPolicy`]). This one-feed refresh ignores a failure
    /// backoff — it is how a user retries by hand — but not a host's
    /// running `Retry-After`: until that deadline it returns a
    /// [`FetchStatus::Error`] outcome without fetching or recording
    /// anything.
    ///
    /// Should another process hold the feed's lock longer than one
    /// refresh can take (it is stuck), the call gives up without fetching
    /// and returns a [`FetchStatus::Error`] outcome saying so; nothing is
    /// recorded.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors;
    /// [`CoreError::Io`] when the lock file cannot be created;
    /// [`CoreError::Panicked`] when the ingest pipeline panics.
    pub async fn refresh_feed(&self, id: FeedId) -> Result<RefreshOutcome, CoreError> {
        self.refresh_feed_until(id, None).await
    }

    /// [`Self::refresh_feed`], abandoned early once `cancel` fires: a
    /// fetch still in flight is dropped (nothing is recorded, the stored
    /// validators stay as they were) and the outcome says the refresh was
    /// cancelled; the full-text hydration of a finished refresh stops at
    /// the next article.
    pub(super) async fn refresh_feed_until(
        &self,
        id: FeedId,
        cancel: Option<&RefreshCancel>,
    ) -> Result<RefreshOutcome, CoreError> {
        // Unknown feeds fail before a lock file is minted for them.
        if self
            .on_blocking_pool(move |storage| Ok(storage.get_feed(id)?))
            .await?
            .is_none()
        {
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
            .on_blocking_pool(move |storage| Ok(storage.get_feed(id)?))
            .await?
            .ok_or(CoreError::NotFound { entity: "feed" })?;
        if feed.status == FeedStatus::Dead {
            return Ok(not_fetched(
                id,
                "feed is dead (HTTP 410) — refusing to fetch",
            ));
        }
        // A host's Retry-After binds every refresh, manual ones included;
        // a plain failure backoff only binds sweeps.
        if let Some(until) = schedule::rate_limited_until(&feed, Timestamp::now()) {
            return Ok(not_fetched(
                id,
                &format!("the host asked Curio to wait — next attempt after {until}"),
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
        let fetched = match cancel {
            None => self.client.fetch(&request).await,
            Some(cancel) => {
                match select(pin!(self.client.fetch(&request)), pin!(cancel.cancelled())).await {
                    Either::Left((fetched, _)) => fetched,
                    Either::Right(((), _)) => return Ok(not_fetched(id, CANCELLED)),
                }
            }
        };
        let now = Timestamp::now();
        let duration_ms = u32::try_from(started.elapsed().as_millis()).ok();

        let policy = self.refresh_policy;
        let settled = self
            .on_blocking_pool(move |storage| {
                settle(storage, &policy, &feed, fetched, now, duration_ms)
            })
            .await?;
        if settled.url_adopted {
            // The URL is the identity key of feed.added/feed.removed;
            // adoption staged a removed/added pair — flush it now.
            self.emit();
        }
        // Full-text mode's refresh hook: hydrate the articles this refresh
        // *inserted* (never the whole backlog) from their source pages,
        // outside the per-feed locks. Best-effort per article — a dead page
        // keeps its feed excerpt, and hydrate_article's never-clobber
        // guard already protects feeds that ship full content.
        drop(across_processes);
        drop(serialized);
        self.hydrate_batch(settled.hydrate_pending, cancel).await;
        Ok(settled.outcome)
    }

    /// Runs a storage job on the blocking pool, off the async runtime.
    /// A panicking job comes back as [`CoreError::Panicked`].
    pub(super) async fn on_blocking_pool<T, F>(&self, job: F) -> Result<T, CoreError>
    where
        T: Send + 'static,
        F: FnOnce(&Storage) -> Result<T, CoreError> + Send + 'static,
    {
        let storage = Arc::clone(&self.storage);
        match tokio::task::spawn_blocking(move || job(&storage)).await {
            Ok(result) => result,
            Err(join) => {
                let message = if join.is_panic() {
                    crate::storage::panic_message(join.into_panic().as_ref())
                } else {
                    "the blocking task was cancelled (runtime shutting down)".to_owned()
                };
                tracing::error!(%message, "a blocking engine task failed");
                Err(CoreError::Panicked { message })
            }
        }
    }

    /// Hydrates the articles a refresh just inserted (full-text mode),
    /// best-effort per article. A rate-limit error stops the whole batch
    /// — every remaining hydrate would fail the same way; the articles
    /// keep their feed excerpts and a later manual load-full picks them
    /// up. So does a cancelled sweep, at the next article.
    async fn hydrate_batch(&self, pending: Vec<ArticleId>, cancel: Option<&RefreshCancel>) {
        for article_id in pending {
            if cancel.is_some_and(RefreshCancel::is_cancelled) {
                break;
            }
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

/// The error of a refresh whose fetch a cancelled sweep abandoned.
pub(super) const CANCELLED: &str = "refresh cancelled";

/// What the blocking half of a refresh settled.
struct Settled {
    /// The outcome, already recorded in the fetch log.
    outcome: RefreshOutcome,
    /// The rows the upsert *inserted* — the full-text hydration queue
    /// (empty unless the feed has full-text mode on).
    hydrate_pending: Vec<ArticleId>,
    /// A permanent redirect was adopted: feed events are staged.
    url_adopted: bool,
}

/// The blocking half of a refresh: maps the fetch result onto feed state
/// (parse → ingest on success, validators preserved on every other path),
/// works out the feed's next attempt (see [`RefreshPolicy`]) and settles
/// the fetch-log row together with that schedule. Runs on the blocking
/// pool, inside the caller's per-feed locks.
fn settle(
    storage: &Storage,
    policy: &RefreshPolicy,
    feed: &Feed,
    fetched: Result<FetchResponse, FetchError>,
    now: Timestamp,
    duration_ms: Option<u32>,
) -> Result<Settled, CoreError> {
    let id = feed.id;
    // Conditional-GET validators are preserved on every non-ingest
    // path — error, 304, 410, unparseable body. Only a successfully
    // *parsed* 2xx adopts the response's validators (the sketch
    // clobbered them on any failure).
    let mut hydrate_pending: Vec<ArticleId> = Vec::new();
    let mut url_adopted = false;
    let parsed = match fetched {
        Ok(response) if response.is_success() => {
            match crate::feeds::parse_feed(&response.body, Some(&response.final_url)) {
                Ok(parsed) => Ok((response, parsed)),
                // A body we could not parse must not update the
                // validators, or a broken feed 304s forever.
                Err(err) => Err((
                    failed(id, Some(response.status), err.to_string()),
                    Attempt::Failed {
                        kind: FetchErrorKind::Parse,
                        retry_after: None,
                    },
                )),
            }
        }
        other => Err(unsuccessful(feed, other)),
    };
    let (outcome, attempt) = match parsed {
        Ok((response, parsed)) => {
            let hint = parsed.meta.refresh_hint.max(response.max_age);
            let ingested = ingest_parsed(storage, id, feed, &response, parsed, now)?;
            hydrate_pending = ingested.hydrate_pending;
            url_adopted = ingested.url_adopted;
            (ingested.outcome, Attempt::Succeeded { hint })
        }
        Err(unsuccessful) => {
            preserve_validators(storage, id, feed, now)?;
            unsuccessful
        }
    };

    storage.settle_fetch(
        FetchRecord {
            feed_id: id,
            fetched_at: now,
            status: outcome.status,
            http_status: outcome.http_status,
            error: outcome.error.clone(),
            articles_new: u32::try_from(outcome.new_articles).unwrap_or(u32::MAX),
            duration_ms,
        },
        policy.after(feed, attempt, now),
    )?;
    Ok(Settled {
        outcome,
        hydrate_pending,
        url_adopted,
    })
}

/// An errored outcome.
fn failed(feed_id: FeedId, http_status: Option<u16>, error: String) -> RefreshOutcome {
    RefreshOutcome {
        feed_id,
        status: FetchStatus::Error,
        http_status,
        new_articles: 0,
        updated_articles: 0,
        error: Some(error),
    }
}

/// The outcome and schedule input of every fetch result but a 2xx: a
/// transport/policy error, 304, 410, a rate limit or another status.
fn unsuccessful(
    feed: &Feed,
    fetched: Result<FetchResponse, FetchError>,
) -> (RefreshOutcome, Attempt) {
    let id = feed.id;
    let failure = |kind, retry_after| Attempt::Failed { kind, retry_after };
    match fetched {
        Err(err) => {
            let kind = match err {
                FetchError::Dns { .. }
                | FetchError::DnsNoAddresses { .. }
                | FetchError::Timeout { .. }
                | FetchError::Transport { .. } => FetchErrorKind::Network,
                _ => FetchErrorKind::Refused,
            };
            (failed(id, None, err.to_string()), failure(kind, None))
        }
        Ok(response) if response.is_not_modified() => (
            RefreshOutcome {
                feed_id: id,
                status: FetchStatus::NotModified,
                http_status: Some(response.status),
                new_articles: 0,
                updated_articles: 0,
                error: None,
            },
            Attempt::Succeeded {
                hint: response.max_age,
            },
        ),
        Ok(response) if response.is_gone() => {
            // 410: gone for good — once a second 410 in a row confirms
            // it (the schedule marks the feed dead then).
            let error = if feed.last_error_kind == Some(FetchErrorKind::Gone) {
                "HTTP 410 Gone again — feed marked dead"
            } else {
                "HTTP 410 Gone — the feed is marked dead if the next attempt says so too"
            };
            (
                failed(id, Some(response.status), error.to_owned()),
                failure(FetchErrorKind::Gone, None),
            )
        }
        Ok(response) if response.is_rate_limited() => {
            let error = match response.retry_after {
                Some(wait) => format!(
                    "HTTP {} — the host is rate-limiting Curio (Retry-After: {} s)",
                    response.status,
                    wait.as_secs()
                ),
                None => format!("HTTP {} — the host is rate-limiting Curio", response.status),
            };
            (
                failed(id, Some(response.status), error),
                failure(FetchErrorKind::RateLimited, response.retry_after),
            )
        }
        Ok(response) => {
            let kind = if response.status >= 500 {
                FetchErrorKind::Server
            } else {
                FetchErrorKind::Client
            };
            (
                failed(
                    id,
                    Some(response.status),
                    format!("HTTP {}", response.status),
                ),
                failure(kind, response.retry_after),
            )
        }
    }
}

/// Stamps `last_fetched_at` while keeping the stored conditional-GET
/// validators exactly as they were.
fn preserve_validators(
    storage: &Storage,
    id: FeedId,
    feed: &Feed,
    now: Timestamp,
) -> Result<(), CoreError> {
    storage.update_feed_fetch_state(id, feed.etag.clone(), feed.last_modified.clone(), now)?;
    Ok(())
}

/// The happy path of a refresh: pipeline every entry, batch-upsert,
/// fill in feed metadata, adopt permanent redirects and the response
/// validators. The settled outcome is not recorded yet — [`settle`]
/// does that.
fn ingest_parsed(
    storage: &Storage,
    id: FeedId,
    feed: &Feed,
    response: &FetchResponse,
    mut parsed: crate::feeds::ParsedFeed,
    now: Timestamp,
) -> Result<Settled, CoreError> {
    // A reused guid's keys are chosen per fetch; settle them against
    // what earlier fetches stored so no item changes key (and splits
    // into a duplicate article) when the guid's sharing changes.
    let stored = storage.source_urls_by_dedupe_key(Some(id), parsed.settle_lookups())?;
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
                storage
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
    let upserted = storage.upsert_articles(articles)?;
    let mut hydrate_pending = Vec::with_capacity(fresh_keys.len());
    for key in &fresh_keys {
        if let Some(article_id) = storage.article_id_by_dedupe_key(Some(id), key)? {
            hydrate_pending.push(article_id);
        }
    }
    storage.update_feed_metadata(
        id,
        parsed.meta.title,
        parsed.meta.site_url,
        parsed.meta.description,
    )?;
    let url_adopted = response.permanent_redirect && response.final_url != feed.url;
    if url_adopted {
        // The URL is the identity key of feed.added/feed.removed;
        // adoption stages a removed/added pair (the caller flushes it).
        storage.update_feed_url(id, response.final_url.clone())?;
    }
    storage.update_feed_fetch_state(
        id,
        response.etag.clone(),
        response.last_modified.clone(),
        now,
    )?;
    Ok(Settled {
        outcome: RefreshOutcome {
            feed_id: id,
            status: FetchStatus::Ok,
            http_status: Some(response.status),
            new_articles: upserted.inserted,
            updated_articles: upserted.updated,
            error: None,
        },
        hydrate_pending,
        url_adopted,
    })
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
    #![allow(clippy::unwrap_used, clippy::panic)]

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

    /// A panic in a refresh's blocking half (a parser bug on a nasty
    /// feed) comes back as an error; the handle keeps working.
    #[tokio::test]
    async fn a_panicking_blocking_job_is_an_error_not_a_crash() {
        let profile = tempfile::tempdir().unwrap();
        let core = CoreHandle::open(profile.path()).unwrap();
        let err = core
            .on_blocking_pool(|_| -> Result<(), CoreError> { panic!("ingest bug") })
            .await
            .unwrap_err();
        assert!(
            matches!(&err, CoreError::Panicked { message } if message == "ingest bug"),
            "{err:?}"
        );
        assert_eq!(core.list_feeds().unwrap(), [] as [Feed; 0]);
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
