//! Feed refresh: conditional fetch, parse, ingest, background hydration,
//! and the per-feed refresh serialization.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Mutex as AsyncMutex;

use curio_types::Timestamp;

use crate::content;
use crate::feeds::ParsedEntry;
use crate::fetch::{FetchConfig, FetchError, FetchRequest, FetchResponse};
use crate::lock::{FileLock, FileLockGuard, LockError};
use crate::model::{
    ArticleContent, Feed, FeedId, FeedStatus, FetchErrorKind, FetchRecord, FetchStatus, NewArticle,
};
use crate::storage::Storage;

use super::hydrate::{DEFAULT_HYDRATE_CONCURRENCY, Hydration};
use super::sweep::{RefreshCancel, until_cancelled};
use super::{CoreError, CoreHandle, LOCKS_DIR, RefreshOutcome};

impl CoreHandle {
    /// Refreshes one feed: policed conditional GET → parse → sanitize →
    /// upsert → fetch-log row. Fetch/parse failures are *outcomes*
    /// (recorded, validators preserved), not errors; only storage-level
    /// failures surface as `Err`.
    ///
    /// The refresh keeps off the async runtime it is awaited on: its
    /// storage reads and writes, the feed parse and the CPU-heavy sanitize
    /// pipeline run on the blocking pool, so a large feed never stalls
    /// that runtime (a current-thread runtime keeps its timers ticking).
    /// With full-text mode on, the articles the refresh inserted are then
    /// hydrated from their pages, [`DEFAULT_HYDRATE_CONCURRENCY`] at a
    /// time (fairly across their hosts), before the call returns — their
    /// readability extraction and writes run on the blocking pool too.
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
    /// [`CoreError::Io`] when the lock file cannot be created;
    /// [`CoreError::Panicked`] when the ingest pipeline panics.
    pub async fn refresh_feed(&self, id: FeedId) -> Result<RefreshOutcome, CoreError> {
        // Only a cancel signal abandons a refresh, and none was given.
        let Some(refreshed) = self.refresh_feed_until(id, None).await? else {
            return Ok(RefreshOutcome {
                feed_id: id,
                status: FetchStatus::Error,
                http_status: None,
                new_articles: 0,
                updated_articles: 0,
                error: Some("refresh cancelled".to_owned()),
                error_kind: None,
            });
        };
        self.hydrate_all(refreshed.hydrate, DEFAULT_HYDRATE_CONCURRENCY)
            .await;
        Ok(refreshed.outcome)
    }

    /// The fetch-and-settle half of [`Self::refresh_feed`], abandoned
    /// early once `cancel` fires: a fetch still in flight is dropped —
    /// nothing is recorded, the stored validators stay as they were, and
    /// the result is `None` (an abandoned refresh has no outcome). Once
    /// the fetch is in, the refresh settles as usual. The full-text
    /// hydrations it queues are the caller's to run (a sweep runs them in
    /// its own bounded queue, so they never hold a feed's slot).
    pub(super) async fn refresh_feed_until(
        &self,
        id: FeedId,
        cancel: Option<&RefreshCancel>,
    ) -> Result<Option<Refreshed>, CoreError> {
        // Unknown feeds fail before a lock file is minted for them.
        if self
            .on_blocking_pool(move |storage| Ok(storage.get_feed(id)?))
            .await?
            .is_none()
        {
            return Err(CoreError::NotFound { entity: "feed" });
        }
        // Owned guards: they move into the blocking job below, so the
        // feed stays locked until its storage writes are done even when
        // this future is dropped (a sweep given up on) while they run.
        let serialized = self.refresh_lock(id).lock_owned().await;
        let Some(across_processes) = self.refresh_file_lock(id).await? else {
            return Ok(Some(Refreshed::alone(not_fetched(
                id,
                FetchErrorKind::Busy,
                "another Curio process is still refreshing this feed — skipped",
            ))));
        };
        let feed = self
            .on_blocking_pool(move |storage| Ok(storage.get_feed(id)?))
            .await?
            .ok_or(CoreError::NotFound { entity: "feed" })?;
        if feed.status == FeedStatus::Dead {
            return Ok(Some(Refreshed::alone(not_fetched(
                id,
                FetchErrorKind::Gone,
                "feed is dead (HTTP 410) — refusing to fetch",
            ))));
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
        let Some(fetched) = until_cancelled(cancel, self.client.fetch(&request)).await else {
            return Ok(None);
        };
        let now = Timestamp::now();
        let duration_ms = u32::try_from(started.elapsed().as_millis()).ok();

        let settled = self
            .on_blocking_pool(move |storage| {
                // Held for the whole job, released when it ends: a dropped
                // caller cannot let the next refresh of this feed in while
                // these writes still run (the validator race above).
                let _locked = (serialized, across_processes);
                settle(storage, id, &feed, fetched, now, duration_ms)
            })
            .await?;
        if settled.url_adopted {
            // The URL is the identity key of feed.added/feed.removed;
            // adoption staged a removed/added pair — flush it now.
            self.emit();
        }
        // Full-text mode's refresh hook: the articles this refresh
        // *inserted* (never the whole backlog) are queued for hydration
        // from their source pages, outside the per-feed locks (the
        // blocking job released them).
        Ok(Some(Refreshed {
            outcome: settled.outcome,
            hydrate: settled.hydrate_pending,
        }))
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

/// A settled feed refresh: its outcome plus the full-text hydrations it
/// queued.
#[derive(Debug)]
pub(super) struct Refreshed {
    /// The outcome, already recorded in the fetch log.
    pub(super) outcome: RefreshOutcome,
    /// The articles the refresh inserted, to hydrate from their pages
    /// (empty unless the feed has full-text mode on).
    pub(super) hydrate: Vec<Hydration>,
}

impl Refreshed {
    /// An outcome with nothing to hydrate.
    fn alone(outcome: RefreshOutcome) -> Self {
        Self {
            outcome,
            hydrate: Vec::new(),
        }
    }
}

/// The outcome of a refresh that refused to fetch (nothing is recorded).
fn not_fetched(feed_id: FeedId, kind: FetchErrorKind, why: &str) -> RefreshOutcome {
    RefreshOutcome {
        feed_id,
        status: FetchStatus::Error,
        http_status: None,
        new_articles: 0,
        updated_articles: 0,
        error: Some(why.to_owned()),
        error_kind: Some(kind),
    }
}

/// Classifies a 2xx body that did not parse as a feed. A body that sniffs
/// as an HTML page is [`FetchErrorKind::NotAFeed`] — the subscription
/// points at a web page — and the message counts the feeds that page
/// declares (fetch-free discovery over the body already in hand), so the
/// user knows whether there is a real feed to switch to. Anything else is
/// a genuine [`FetchErrorKind::Parse`] failure.
fn unparseable(
    response: &crate::fetch::FetchResponse,
    err: &crate::feeds::FeedParseError,
) -> (FetchErrorKind, String) {
    let Some(feeds) = crate::feeds::discover::web_page_feeds(response) else {
        return (FetchErrorKind::Parse, err.to_string());
    };
    let message = match feeds.as_slice() {
        [] => "this is a web page, not a feed — found no feeds on it".to_owned(),
        [only] => format!(
            "this is a web page, not a feed — found 1 feed on it: {}",
            only.url
        ),
        [first, ..] => format!(
            "this is a web page, not a feed — found {} feeds on it (first: {})",
            feeds.len(),
            first.url
        ),
    };
    (FetchErrorKind::NotAFeed, message)
}

/// What the blocking half of a refresh settled.
struct Settled {
    /// The outcome, already recorded in the fetch log.
    outcome: RefreshOutcome,
    /// The rows the upsert *inserted* — the full-text hydration queue
    /// (empty unless the feed has full-text mode on).
    hydrate_pending: Vec<Hydration>,
    /// A permanent redirect was adopted: feed events are staged.
    url_adopted: bool,
}

/// The blocking half of a refresh: maps the fetch result onto feed state
/// (parse → ingest on success, validators preserved on every other path)
/// and records the fetch-log row. Runs on the blocking pool, inside the
/// caller's per-feed locks.
fn settle(
    storage: &Storage,
    id: FeedId,
    feed: &Feed,
    fetched: Result<FetchResponse, FetchError>,
    now: Timestamp,
    duration_ms: Option<u32>,
) -> Result<Settled, CoreError> {
    let failed = |http_status: Option<u16>, kind: FetchErrorKind, error: String| RefreshOutcome {
        feed_id: id,
        status: FetchStatus::Error,
        http_status,
        new_articles: 0,
        updated_articles: 0,
        error: Some(error),
        error_kind: Some(kind),
    };

    // Conditional-GET validators are preserved on every non-ingest
    // path — error, 304, 410, unparseable body. Only a successfully
    // *parsed* 2xx adopts the response's validators (the sketch
    // clobbered them on any failure).
    let mut hydrate_pending: Vec<Hydration> = Vec::new();
    let mut url_adopted = false;
    let outcome = match fetched {
        Err(err) => {
            preserve_validators(storage, id, feed, now)?;
            failed(None, err.kind(), err.to_string())
        }
        Ok(response) if response.is_not_modified() => {
            preserve_validators(storage, id, feed, now)?;
            RefreshOutcome {
                feed_id: id,
                status: FetchStatus::NotModified,
                http_status: Some(response.status),
                new_articles: 0,
                updated_articles: 0,
                error: None,
                error_kind: None,
            }
        }
        Ok(response) if response.is_gone() => {
            // 410: the feed is gone for good — auto-pause forever.
            storage.set_feed_status(id, FeedStatus::Dead)?;
            preserve_validators(storage, id, feed, now)?;
            failed(
                Some(response.status),
                FetchErrorKind::Gone,
                "HTTP 410 Gone — feed auto-paused as dead".to_owned(),
            )
        }
        Ok(response) if response.is_success() => {
            match crate::feeds::parse_fetched_feed(&response) {
                Err(err) => {
                    // A body we could not parse must not update the
                    // validators, or a broken feed 304s forever.
                    preserve_validators(storage, id, feed, now)?;
                    let (kind, message) = unparseable(&response, &err);
                    failed(Some(response.status), kind, message)
                }
                Ok(parsed) => {
                    let ingested = ingest_parsed(storage, id, feed, &response, parsed, now)?;
                    hydrate_pending = ingested.hydrate_pending;
                    url_adopted = ingested.url_adopted;
                    ingested.outcome
                }
            }
        }
        Ok(response) => {
            preserve_validators(storage, id, feed, now)?;
            failed(
                Some(response.status),
                FetchErrorKind::Http,
                format!("HTTP {}", response.status),
            )
        }
    };

    storage.record_fetch(FetchRecord {
        feed_id: id,
        fetched_at: now,
        status: outcome.status,
        http_status: outcome.http_status,
        error: outcome.error.clone(),
        error_kind: outcome.error_kind,
        articles_new: u32::try_from(outcome.new_articles).unwrap_or(u32::MAX),
        duration_ms,
    })?;
    Ok(Settled {
        outcome,
        hydrate_pending,
        url_adopted,
    })
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
    let fresh: Vec<(String, String)> = if feed.fetch_full_text {
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
            .map(|article| (article.dedupe_key.clone(), article.source_url.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let upserted = storage.upsert_articles(articles)?;
    let mut hydrate_pending = Vec::with_capacity(fresh.len());
    for (key, url) in fresh {
        if let Some(article_id) = storage.article_id_by_dedupe_key(Some(id), &key)? {
            hydrate_pending.push(Hydration {
                feed_id: id,
                article_id,
                url,
            });
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
            error_kind: None,
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
        assert_eq!(skipped.error_kind, Some(FetchErrorKind::Busy));
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
        assert_eq!(refreshed.error_kind, Some(FetchErrorKind::Http));
        assert_eq!(refreshed.error.as_deref(), Some("HTTP 500"));
        let recorded = core.recent_fetches(feed.id, 10).unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].error_kind, Some(FetchErrorKind::Http));

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
