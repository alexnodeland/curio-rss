//! [`CoreHandle`] — the engine facade every head consumes.
//!
//! Opening a profile directory wires the whole engine together:
//!
//! - `curio.db` — WAL `SQLite` behind the [`Storage`] facade;
//! - `.curio/events/` — the `curio.events.v1` JSONL log, with the
//!   crash-recovery replay of staged intents at startup;
//! - the [`PolicedClient`] — the only outbound network path;
//! - the named-destination registry (persisted in settings; raw paths
//!   never cross the facade's operation surface).
//!
//! Every state change that the events contract cares about stages its
//! envelope inside the storage transaction and is flushed to JSONL
//! before the operation returns — heads never think about emission.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Instant;

use tokio::sync::Mutex as AsyncMutex;

use curio_types::{Destination, DestinationName, Timestamp};

use crate::content::{self, ContentError};
use crate::events::{EventEmitter, EventLog, EventsError};
use crate::export::{self, ExportDisposition, ExportError, ExportInput};
use crate::feeds::{FeedParseError, OpmlError, OpmlFeed, ParsedEntry};
use crate::fetch::{FetchConfig, FetchError, FetchRequest, PolicedClient};
use crate::import::{self, ImportError, ImportKind, ImportSource, ImportedItem};
use crate::model::{
    Article, ArticleContent, ArticleId, ArticleState, Feed, FeedId, FeedStatus, FetchRecord,
    FetchStatus, NewArticle, NewFeed,
};
use crate::storage::{ListArticles, Storage, StorageError};

/// Settings key holding the destination registry (JSON `name → root`).
const DESTINATIONS_KEY: &str = "destinations";

/// Facade-level errors: every engine subsystem, plus facade concerns.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// Storage failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Event emission failed.
    #[error(transparent)]
    Events(#[from] EventsError),
    /// The policed fetch failed.
    #[error(transparent)]
    Fetch(#[from] FetchError),
    /// Feed bytes did not parse.
    #[error(transparent)]
    FeedParse(#[from] FeedParseError),
    /// Export failed.
    #[error(transparent)]
    Export(#[from] ExportError),
    /// Content processing failed.
    #[error(transparent)]
    Content(#[from] ContentError),
    /// OPML import/export failed.
    #[error(transparent)]
    Opml(#[from] OpmlError),
    /// A refugee import file could not be parsed.
    #[error(transparent)]
    Import(#[from] ImportError),
    /// Settings (de)serialization failed.
    #[error("settings json: {0}")]
    Settings(#[from] serde_json::Error),
    /// Profile-directory setup failed.
    #[error("profile io at {path}: {source}")]
    Io {
        /// The path being touched.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// No destination is registered under that name.
    #[error("unknown destination {name:?} — register it with add_destination first")]
    UnknownDestination {
        /// The name that was looked up.
        name: String,
    },
    /// A row referenced by the caller does not exist.
    #[error("{entity} not found")]
    NotFound {
        /// What was looked up.
        entity: &'static str,
    },
    /// The URL handed to a save operation is not a fetchable http(s) URL.
    #[error("not a fetchable http(s) URL: {url}")]
    InvalidUrl {
        /// The rejected input.
        url: String,
    },
    /// The upstream service is rate-limiting us; the enrichment breaker
    /// is open and no request was (or will be) made until it cools down.
    #[error("{host} is rate-limiting requests — try again in about {} min", retry_after_secs.div_ceil(60))]
    RateLimited {
        /// The throttling host.
        host: String,
        /// Seconds until the breaker re-tries.
        retry_after_secs: u64,
    },
}

/// Options for [`CoreHandle::open_with`].
#[derive(Debug, Clone, Default)]
pub struct CoreOptions {
    /// Policy configuration of the outbound client.
    pub fetch: FetchConfig,
}

/// What one feed refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshOutcome {
    /// The refreshed feed.
    pub feed_id: FeedId,
    /// Outcome class, as recorded in `fetch_log`.
    pub status: FetchStatus,
    /// HTTP status, if a response arrived.
    pub http_status: Option<u16>,
    /// Articles inserted by this refresh.
    pub new_articles: u64,
    /// Existing articles refreshed by this refresh.
    pub updated_articles: u64,
    /// Error detail for [`FetchStatus::Error`].
    pub error: Option<String>,
}

/// What one save-to-destination did.
#[derive(Debug, Clone, PartialEq)]
pub struct SaveOutcome {
    /// The destination written to.
    pub destination: DestinationName,
    /// Note path relative to the destination root.
    pub path: String,
    /// Managed-region checksum (change token only).
    pub checksum: curio_types::Checksum,
    /// Created (`article.saved`), Updated (`article.updated`) or
    /// Unchanged (idempotency hit — no event).
    pub disposition: ExportDisposition,
}

/// Outcome of a single-URL save (the read-later clip path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedUrl {
    /// The stored (or already-present) article.
    pub article: Article,
    /// `false` when the URL was already in the library (it was
    /// re-flagged read-later; nothing was refetched).
    pub created: bool,
    /// Whether full-text content was fetched and stored by this call —
    /// `false` for an unreachable page (the link is saved bare) and for
    /// an existing article.
    pub hydrated: bool,
}

/// Outcome of a bulk note export: how many notes each disposition got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BulkSaveOutcome {
    /// Notes written for the first time (`article.saved` each).
    pub created: u64,
    /// Existing notes whose managed region changed (`article.updated` each).
    pub updated: u64,
    /// Idempotency hits — already exported, byte-identical, no event.
    pub unchanged: u64,
}

impl BulkSaveOutcome {
    /// Articles the export walked in total.
    #[must_use]
    pub fn total(&self) -> u64 {
        self.created + self.updated + self.unchanged
    }
}

/// Outcome of an OPML import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpmlImportOutcome {
    /// Feeds newly subscribed.
    pub added: usize,
    /// Feeds skipped because the URL was already subscribed.
    pub skipped: usize,
}

/// Outcome of a general [`CoreHandle::import_file`] / [`CoreHandle::import_items`]
/// run, split by kind so the UI can report "N feeds, M articles" honestly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    /// Feeds newly subscribed.
    pub feeds_added: usize,
    /// Feeds skipped because the URL was already subscribed.
    pub feeds_skipped: usize,
    /// Articles saved as feedless read-later items.
    pub articles_added: usize,
    /// Articles skipped because the URL was already imported.
    pub articles_skipped: usize,
}

/// The engine service object. Heads hold an `Arc<CoreHandle>` and
/// delegate; they contain no business logic of their own.
#[derive(Debug)]
pub struct CoreHandle {
    profile_dir: PathBuf,
    storage: Arc<Storage>,
    emitter: Mutex<EventEmitter>,
    client: PolicedClient,
    destinations: RwLock<BTreeMap<DestinationName, PathBuf>>,
    /// Per-feed refresh serialization (see [`CoreHandle::refresh_feed`]).
    /// Bounded by the subscription count; `remove_feed` drops entries.
    refresh_locks: Mutex<HashMap<FeedId, Arc<AsyncMutex<()>>>>,
    /// Circuit breaker over enrichment calls (D8/D14): repeated provider
    /// failures skip enrichment for a cool-down instead of stalling
    /// every hydrate.
    #[cfg(feature = "enrich-reddit")]
    enrich_breaker: crate::enrich::Breaker,
    /// BYO Reddit OAuth (D15): runtime credentials + cached bearer.
    /// Loaded by the heads (keychain) after open; absent = the public
    /// unauthenticated endpoints.
    #[cfg(feature = "enrich-reddit")]
    reddit_auth: crate::enrich::reddit_auth::TokenManager,
}

impl CoreHandle {
    /// Opens a profile directory with default options.
    ///
    /// # Errors
    ///
    /// See [`CoreHandle::open_with`].
    pub fn open(profile_dir: impl AsRef<Path>) -> Result<Self, CoreError> {
        Self::open_with(profile_dir, CoreOptions::default())
    }

    /// Opens (creating if absent) a profile directory: database, events
    /// log (with crash-recovery replay of staged intents), policed
    /// client, and the persisted destination registry.
    ///
    /// # Errors
    ///
    /// [`CoreError`] on filesystem, storage or event-log failures.
    pub fn open_with(
        profile_dir: impl AsRef<Path>,
        options: CoreOptions,
    ) -> Result<Self, CoreError> {
        let profile_dir = profile_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&profile_dir).map_err(|source| CoreError::Io {
            path: profile_dir.clone(),
            source,
        })?;
        let storage = Arc::new(Storage::open(profile_dir.join("curio.db"))?);
        let events_dir = profile_dir.join(".curio").join("events");
        let log = EventLog::open(&events_dir).map_err(EventsError::Log)?;
        // Contract: the event log is never committed to git.
        let gitignore = profile_dir.join(".curio").join(".gitignore");
        if !gitignore.exists() {
            std::fs::write(&gitignore, "events/\n").map_err(|source| CoreError::Io {
                path: gitignore.clone(),
                source,
            })?;
        }
        let emitter = EventEmitter::new(Arc::clone(&storage), log)?;
        let destinations = load_destinations(&storage)?;
        Ok(Self {
            profile_dir,
            storage,
            emitter: Mutex::new(emitter),
            client: PolicedClient::new(options.fetch),
            destinations: RwLock::new(destinations),
            refresh_locks: Mutex::new(HashMap::new()),
            #[cfg(feature = "enrich-reddit")]
            enrich_breaker: crate::enrich::Breaker::default(),
            #[cfg(feature = "enrich-reddit")]
            reddit_auth: crate::enrich::reddit_auth::TokenManager::default(),
        })
    }

    /// The engine crate version.
    #[must_use]
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// The profile directory this handle was opened on.
    #[must_use]
    pub fn profile_dir(&self) -> &Path {
        &self.profile_dir
    }

    /// The storage facade, for read paths the facade does not wrap.
    #[must_use]
    pub fn storage(&self) -> &Storage {
        &self.storage
    }

    /// Installs (or clears) the user's own Reddit API credentials (D15).
    /// Runtime-only — persistence is the heads' concern (the keychain);
    /// this never writes anything to disk. Clearing also drops any
    /// cached token and resets the enrichment breaker (a rate limit hit
    /// unauthenticated says nothing about the authenticated tier).
    #[cfg(feature = "enrich-reddit")]
    pub fn set_reddit_api(&self, config: Option<crate::enrich::reddit_auth::RedditApiConfig>) {
        self.reddit_auth.set_config(config);
        self.enrich_breaker.record_success();
    }

    /// Whether Reddit API credentials are installed, and under which
    /// client id (never the secret).
    #[cfg(feature = "enrich-reddit")]
    #[must_use]
    pub fn reddit_api_client_id(&self) -> Option<String> {
        self.reddit_auth.client_id()
    }

    // ------------------------------------------------------------ feeds

    /// Subscribes to a feed; emits `feed.added` (with tags).
    ///
    /// # Errors
    ///
    /// Storage (including duplicate URL) or emission errors.
    pub fn add_feed(&self, new: NewFeed) -> Result<Feed, CoreError> {
        let (feed, _envelope) = self.storage.add_feed(new)?;
        self.emit()?;
        Ok(feed)
    }

    /// Unsubscribes a feed; emits the `feed.removed` negation. Articles
    /// keep their rows.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn remove_feed(&self, id: FeedId) -> Result<(), CoreError> {
        self.storage.remove_feed(id)?;
        self.refresh_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        self.emit()?;
        Ok(())
    }

    /// Every subscription.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_feeds(&self) -> Result<Vec<Feed>, CoreError> {
        Ok(self.storage.list_feeds()?)
    }

    /// One feed by id.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_feed(&self, id: FeedId) -> Result<Option<Feed>, CoreError> {
        Ok(self.storage.get_feed(id)?)
    }

    /// Contract W1: flips the per-feed private-network exemption. This
    /// is the *only* path to the flag — explicit configuration, never
    /// feed content.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn set_feed_allow_private_network(&self, id: FeedId, allow: bool) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_allow_private_network(id, allow)?)
    }

    /// Flips a feed's full-text mode: when on, every refresh hydrates the
    /// feed's *new* articles from their source pages (readability-extract
    /// through the policed client) — the upgrade for feeds that ship only
    /// excerpts. DB-local, no event: how a feed's content is fetched is a
    /// local reading preference, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn set_feed_full_text(&self, id: FeedId, enabled: bool) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_full_text(id, enabled)?)
    }

    /// Replaces a feed's tags — the move-to-folder / re-tag path (folders
    /// are a `/`-path-tag rendering in the head). A wholesale overwrite that
    /// is DB-local, like the feed-metadata edit: it stages no event, since
    /// consumers key feeds by URL and folder membership is a local
    /// reading-organization concern, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_tags(&self, id: FeedId, tags: Vec<String>) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_tags(id, tags)?)
    }

    /// Renames a feed — an unconditional title overwrite (an empty title
    /// clears it, falling back to the URL). DB-local, no event: the display
    /// title is not part of the published contract, and consumers key feeds
    /// by URL. This is the user-rename counterpart to the fetch-fill
    /// `update_feed_metadata`, which can only fill a NULL title.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_title(&self, id: FeedId, title: Option<String>) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_title(id, title)?)
    }

    /// Overwrites a feed's site URL and description — the edit-feed modal's
    /// user-edit path. DB-local, no contract event (the head's command layer
    /// emits the UI `FeedsChanged`): site URL / description are not part of
    /// the published contract, and consumers key feeds by URL. This is the
    /// user-overwrite counterpart to the fetch-fill `update_feed_metadata`,
    /// which only fills a NULL field so a refresh never clobbers a human edit.
    /// An empty value clears the field. The display title stays on
    /// [`Self::set_feed_title`].
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_metadata(
        &self,
        id: FeedId,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_metadata(id, site_url, description)?)
    }

    /// Rewrites the sidebar feed order to `ordered` (drag-to-reorder). The
    /// caller passes the complete new sequence. DB-local, no event: feed
    /// ordering is a local reading-organization preference, not part of the
    /// published contract.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn reorder_feeds(&self, ordered: &[FeedId]) -> Result<(), CoreError> {
        Ok(self.storage.reorder_feeds(ordered)?)
    }

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
                Err(CoreError::RateLimited { host, .. }) => {
                    tracing::warn!(
                        %host,
                        "rate-limited during full-text hydrate; stopping the batch"
                    );
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
        // caller's per-feed lock, so this read-then-write cannot race.
        let fresh_keys: Vec<String> = if feed.fetch_full_text {
            articles
                .iter()
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
            self.emit()?;
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

    // --------------------------------------------------------- articles

    /// Keyset-paginated article listing.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_articles(&self, params: ListArticles) -> Result<Vec<Article>, CoreError> {
        Ok(self.storage.list_articles(params)?)
    }

    /// One article by row id.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_article(&self, id: ArticleId) -> Result<Option<Article>, CoreError> {
        Ok(self.storage.get_article(id)?)
    }

    /// Fetches the article's source page and replaces its stored content with
    /// the readability-extracted main content — the on-demand "load full
    /// article" enrichment. Uses the SAME policed client as refresh (the SSRF
    /// guard applies; the private-network exemption is taken only from the
    /// owning feed's flag, never widened for a manual fetch). Never clobbers
    /// good content: a non-2xx response, or an empty/shorter extract, leaves
    /// the article unchanged. DB-local like [`Self::mark_read`] — the events
    /// contract has no article-content event, so nothing is staged.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article; fetch, content or
    /// storage errors.
    pub async fn hydrate_article(&self, id: ArticleId) -> Result<Article, CoreError> {
        let article = self
            .storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })?;
        let allow_private_network = match article.feed_id {
            Some(feed_id) => self
                .storage
                .get_feed(feed_id)?
                .is_some_and(|feed| feed.allow_private_network),
            None => false,
        };
        // Enrichment first (feature-gated): a source-specific provider
        // beats generic readability when it recognizes the URL. A rate
        // limit is a hard stop (no fallback fetch to the throttled
        // host); any other enrichment failure falls through to the
        // generic page fetch below.
        #[cfg(feature = "enrich-reddit")]
        if let Some(enriched) = self
            .try_reddit_enrichment(id, &article, allow_private_network)
            .await?
        {
            return Ok(enriched);
        }
        let response = self
            .client
            .fetch(&FetchRequest {
                url: article.source_url.clone(),
                allow_private_network,
                ..FetchRequest::default()
            })
            .await?;
        if !response.is_success() {
            return Ok(article);
        }
        let raw = String::from_utf8_lossy(&response.body);
        let processed = content::process_full_page(&raw, &response.final_url)?;
        // A mis-scored thin page must not clobber real feed content.
        if processed.text.trim().is_empty() || processed.text.len() < article.content.text.len() {
            return Ok(article);
        }
        self.storage.update_article_content(
            id,
            &ArticleContent {
                html: processed.html,
                text: processed.text,
            },
            Some(processed.word_count),
        )?;
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
    }

    /// Saves a single URL as a feedless read-later article — the
    /// GoodLinks-style clip path. The page is fetched through the policed
    /// client (no private-network exemption — that flag belongs to feeds)
    /// and readability-extracted, adopting the page's own title, byline,
    /// language, publish time and lead image where declared. An
    /// unreachable or unextractable page still saves the bare link
    /// (`hydrated: false`) — a save must never lose the URL.
    ///
    /// Dedupe is by URL within the manual-save scope: re-saving a URL
    /// (or importing it first) re-flags it read-later and applies any new
    /// tags, without refetching. Emits `article.read_later.added` and
    /// `article.tagged` per the state changes it actually makes.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidUrl`] for a non-http(s) input; storage or
    /// emission errors. Fetch/extract failures are NOT errors (the bare
    /// link is saved instead).
    pub async fn save_url(&self, url: &str, tags: Vec<String>) -> Result<SavedUrl, CoreError> {
        let canonical = url::Url::parse(url.trim())
            .ok()
            .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
            .map(String::from)
            .ok_or_else(|| CoreError::InvalidUrl {
                url: url.to_owned(),
            })?;
        let key = crate::feeds::dedupe_key(None, Some(&canonical), &canonical, None);
        if let Some(id) = self.storage.article_id_by_dedupe_key(None, &key)? {
            let article = self.flag_saved_url(id, &tags)?;
            return Ok(SavedUrl {
                article,
                created: false,
                hydrated: false,
            });
        }

        // Best-effort fetch + extract: the row below is written either way.
        let page = match self
            .client
            .fetch(&FetchRequest {
                url: canonical.clone(),
                ..FetchRequest::default()
            })
            .await
        {
            Ok(response) if response.is_success() => {
                let raw = String::from_utf8_lossy(&response.body);
                content::process_page(&raw, &response.final_url).ok()
            }
            _ => None,
        };
        let hydrated = page.is_some();
        let page = page.unwrap_or_default();
        let (body, meta) = (page.content, page.meta);
        // The page's declared image wins when it's a fetchable URL; else
        // fall back to the first inline <img> of the sanitized body.
        let lead_image = meta
            .image
            .as_deref()
            .map(str::trim)
            .filter(|src| src.starts_with("http://") || src.starts_with("https://"))
            .map(ToOwned::to_owned)
            .or_else(|| content::first_image(&body.html));
        self.storage.upsert_articles(vec![NewArticle {
            feed_id: None,
            dedupe_key: key.clone(),
            title: meta.title.unwrap_or_else(|| canonical.clone()),
            source_url: canonical,
            author: meta.byline,
            published_at: meta
                .published_time
                .and_then(|raw| raw.parse::<Timestamp>().ok()),
            lang: meta.lang,
            word_count: hydrated.then_some(body.word_count),
            content: ArticleContent {
                html: body.html,
                text: body.text,
            },
            source_updated_at: None,
            lead_image,
        }])?;
        let id = self
            .storage
            .article_id_by_dedupe_key(None, &key)?
            .ok_or(CoreError::NotFound {
                entity: "saved article",
            })?;
        let article = self.flag_saved_url(id, &tags)?;
        Ok(SavedUrl {
            article,
            created: true,
            hydrated,
        })
    }

    /// Flags a saved-URL row read-later and applies its tags (each staging
    /// its event), flushing one emission; returns the fresh row.
    fn flag_saved_url(&self, id: ArticleId, tags: &[String]) -> Result<Article, CoreError> {
        self.storage.add_read_later(id)?;
        for tag in tags {
            self.storage.tag_article(id, tag)?;
        }
        self.emit()?;
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
    }

    /// The reddit enrichment attempt (D14/D15), split out of
    /// [`Self::hydrate_article`]: `Ok(Some(_))` = enriched and stored;
    /// `Ok(None)` = not a reddit post / soft failure, use the generic
    /// path; `Err(RateLimited)` = hard stop, make no further request to
    /// the throttled host. Provider output passes the SAME sanitize gate
    /// as everything else. A 429 — from the token grant or the API call
    /// — trips the breaker immediately, honoring `Retry-After`.
    #[cfg(feature = "enrich-reddit")]
    async fn try_reddit_enrichment(
        &self,
        id: ArticleId,
        article: &Article,
        allow_private_network: bool,
    ) -> Result<Option<Article>, CoreError> {
        use crate::enrich::{EnrichError, reddit};

        if !reddit::is_reddit_post(&article.source_url) {
            return Ok(None);
        }
        let rate_limited = |remaining: Option<std::time::Duration>| CoreError::RateLimited {
            host: "reddit.com".to_owned(),
            retry_after_secs: remaining.map_or(60, |d| d.as_secs().max(1)),
        };
        if !self.enrich_breaker.closed() {
            return Err(rate_limited(self.enrich_breaker.open_remaining()));
        }
        // BYO OAuth (D15): trade the installed credentials for a bearer.
        // A 429 from the token endpoint is a rate limit like any other;
        // a failed grant (bad credentials) degrades to unauthenticated.
        let bearer = match self.reddit_auth.bearer(&self.client).await {
            Ok(bearer) => bearer,
            Err(EnrichError::Http {
                status: 429,
                retry_after,
            }) => {
                self.enrich_breaker.trip(retry_after);
                tracing::warn!("reddit token endpoint rate limit; enrichment breaker opened");
                return Err(rate_limited(self.enrich_breaker.open_remaining()));
            }
            Err(err) => {
                tracing::warn!(%err, "reddit token grant failed; using the public endpoint");
                None
            }
        };
        let api_origin = self.reddit_auth.api_origin();
        let auth = match (bearer.as_deref(), api_origin.as_deref()) {
            (Some(bearer), Some(api_origin)) => Some(reddit::PostAuth { bearer, api_origin }),
            _ => None,
        };
        let authenticated = auth.is_some();
        match reddit::fetch_post(
            &self.client,
            &article.source_url,
            allow_private_network,
            auth,
        )
        .await
        {
            Ok(enriched) => {
                self.enrich_breaker.record_success();
                // Provider output IS the main content already — it only
                // passes the sanitize gate (readability's boilerplate
                // extraction would drop the appended media figures).
                let html = content::sanitize(&enriched.html, Some(&article.source_url));
                let text = content::plain_text(&html);
                let word_count = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
                // The API is authoritative for its own posts, so it may
                // replace longer feed content — but never with nothing.
                if text.trim().is_empty() && content::first_image(&html).is_none() {
                    return Ok(None);
                }
                self.storage.update_article_content(
                    id,
                    &ArticleContent { html, text },
                    Some(word_count),
                )?;
                self.storage
                    .get_article(id)?
                    .ok_or(CoreError::NotFound { entity: "article" })
                    .map(Some)
            }
            Err(EnrichError::Http {
                status: 429,
                retry_after,
            }) => {
                self.enrich_breaker.trip(retry_after);
                tracing::warn!(
                    article = %id,
                    retry_after_secs = retry_after.map(|d| d.as_secs()),
                    "reddit rate limit hit; enrichment breaker opened"
                );
                Err(rate_limited(self.enrich_breaker.open_remaining()))
            }
            Err(EnrichError::Http {
                status: status @ (401 | 403),
                ..
            }) if authenticated => {
                // A stale or revoked token: drop it so the next call
                // re-grants, and fall back to the public endpoint.
                self.reddit_auth.invalidate();
                self.enrich_breaker.record_failure();
                tracing::warn!(article = %id, status, "reddit bearer rejected; token dropped");
                Ok(None)
            }
            Err(err) => {
                self.enrich_breaker.record_failure();
                tracing::warn!(
                    article = %id,
                    %err,
                    "reddit enrichment failed; falling back to the page fetch"
                );
                Ok(None)
            }
        }
    }

    /// Full-text search (escaped FTS5 — user input is never raw MATCH).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn search_articles(&self, query: &str, limit: u32) -> Result<Vec<Article>, CoreError> {
        Ok(self.storage.search_articles(query, limit)?)
    }

    /// The article's current flag projection.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn article_state(&self, id: ArticleId) -> Result<ArticleState, CoreError> {
        Ok(self.storage.article_state(id)?)
    }

    /// The article's tags, sorted.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn article_tags(&self, id: ArticleId) -> Result<Vec<String>, CoreError> {
        Ok(self.storage.article_tags(id)?)
    }

    /// Unread-article counts grouped by feed (`None` = articles without
    /// a feed; total = the sum of the values). Counts are backend-owned:
    /// heads render them, they never re-derive them client-side.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn unread_counts(&self) -> Result<BTreeMap<Option<FeedId>, u64>, CoreError> {
        Ok(self.storage.unread_counts()?)
    }

    // ------------------------------------------------------------ state

    /// Star / unstar; emits `article.starred` (with tags) or the
    /// `article.unstarred` negation. Idempotent — `false` means no
    /// change and no event.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_starred(&self, id: ArticleId, starred: bool) -> Result<bool, CoreError> {
        let staged = if starred {
            self.storage.star_article(id)?
        } else {
            self.storage.unstar_article(id)?
        };
        self.emit_if(staged.is_some())
    }

    /// Add to / remove from read-later; emits `article.read_later.added`
    /// or the `.removed` negation. Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_read_later(&self, id: ArticleId, read_later: bool) -> Result<bool, CoreError> {
        let staged = if read_later {
            self.storage.add_read_later(id)?
        } else {
            self.storage.remove_read_later(id)?
        };
        self.emit_if(staged.is_some())
    }

    /// Archive / unarchive; emits `article.archived` or the
    /// `article.unarchived` negation. Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn set_archived(&self, id: ArticleId, archived: bool) -> Result<bool, CoreError> {
        let staged = if archived {
            self.storage.archive_article(id)?
        } else {
            self.storage.unarchive_article(id)?
        };
        self.emit_if(staged.is_some())
    }

    /// Read/unread. DB-local: `curio.events.v1` defines no read event.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn mark_read(&self, id: ArticleId, read: bool) -> Result<bool, CoreError> {
        Ok(self.storage.mark_read(id, read)?)
    }

    /// Marks every unread article read — one feed (`Some`) or the whole
    /// library (`None`). Returns how many were changed. DB-local, no event.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn mark_all_read(&self, feed_id: Option<FeedId>) -> Result<u64, CoreError> {
        Ok(self.storage.mark_all_read(feed_id)?)
    }

    /// Records an open-for-reading; emits `article.opened`.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn record_opened(&self, id: ArticleId, dwell_ms: Option<u64>) -> Result<(), CoreError> {
        self.storage.record_opened(id, dwell_ms)?;
        self.emit()?;
        Ok(())
    }

    /// Tags an article; emits `article.tagged`. Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn tag_article(&self, id: ArticleId, tag: &str) -> Result<bool, CoreError> {
        let staged = self.storage.tag_article(id, tag)?;
        self.emit_if(staged.is_some())
    }

    /// Untags an article; emits the `article.untagged` negation.
    /// Idempotent.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article.
    pub fn untag_article(&self, id: ArticleId, tag: &str) -> Result<bool, CoreError> {
        let staged = self.storage.untag_article(id, tag)?;
        self.emit_if(staged.is_some())
    }

    // ----------------------------------------------------- destinations

    /// Registers (or re-roots) a named destination and persists the
    /// registry. The only place a destination path ever enters.
    ///
    /// The registry is persisted *inside* the write-lock critical
    /// section, and the in-memory map only adopts the entry after the
    /// persist succeeds — so concurrent registrations cannot clobber
    /// each other's settings value, and a failed persist never leaves a
    /// destination that works now but vanishes on the next open.
    ///
    /// # Errors
    ///
    /// Filesystem or settings errors.
    pub fn add_destination(&self, name: DestinationName, root: PathBuf) -> Result<(), CoreError> {
        std::fs::create_dir_all(&root).map_err(|source| CoreError::Io {
            path: root.clone(),
            source,
        })?;
        let mut map = self.lock_destinations_mut();
        let mut next = map.clone();
        next.insert(name, root);
        self.persist_destinations(&next)?;
        *map = next;
        Ok(())
    }

    /// Unregisters a named destination and persists the registry.
    /// Exported notes under the old root are untouched — removal only
    /// forgets the name (the event stream already told their story).
    /// Same persist-then-adopt discipline as [`CoreHandle::add_destination`].
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`] if no destination is registered
    /// under `name`; settings errors.
    pub fn remove_destination(&self, name: &DestinationName) -> Result<(), CoreError> {
        let mut map = self.lock_destinations_mut();
        if !map.contains_key(name) {
            return Err(CoreError::UnknownDestination {
                name: name.to_string(),
            });
        }
        let mut next = map.clone();
        next.remove(name);
        self.persist_destinations(&next)?;
        *map = next;
        Ok(())
    }

    /// The registered destinations.
    #[must_use]
    pub fn destinations(&self) -> Vec<Destination> {
        self.lock_destinations()
            .iter()
            .map(|(name, root)| Destination {
                name: name.clone(),
                root: root.clone(),
            })
            .collect()
    }

    /// Exports an article to a named destination and emits the matching
    /// contract event: `article.saved` on first export,
    /// `article.updated` on a changed re-export, nothing on the
    /// idempotency hit.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`], [`CoreError::NotFound`],
    /// export/content/storage/emission errors.
    pub fn save_to_destination(
        &self,
        id: ArticleId,
        destination: &DestinationName,
    ) -> Result<SaveOutcome, CoreError> {
        let root = self
            .lock_destinations()
            .get(destination)
            .cloned()
            .ok_or_else(|| CoreError::UnknownDestination {
                name: destination.to_string(),
            })?;
        let article = self
            .storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })?;
        let feed = match article.feed_id {
            Some(feed_id) => self.storage.get_feed(feed_id)?,
            None => None,
        };
        let tags = self.storage.article_tags(id)?;
        let markdown = content::to_markdown(&article.content.html)?;

        let input = ExportInput {
            curio_id: article.curio_id,
            title: article.title.clone(),
            source: article.source_url.clone(),
            feed: feed.as_ref().map(|f| f.url.clone()),
            feed_title: feed.as_ref().and_then(|f| f.title.clone()),
            author: article.author.clone(),
            published: article.published_at,
            saved: article.saved_at,
            tags: tags.clone(),
            lang: article.lang.clone(),
            word_count: article.word_count,
            markdown,
        };
        let dest = Destination {
            name: destination.clone(),
            root,
        };
        // Ordering: note write → event intent → manifest write. The
        // intent is staged BEFORE the manifest commit — were it the
        // other way around, a crash between the two would make the
        // manifest's (curio_id, checksum) idempotency hit suppress the
        // event on every retry, losing article.saved/updated forever.
        let staged = export::stage_export_note(&dest, &input)?;
        let snapshot = article.snapshot(
            input.feed.clone(),
            input.feed_title.clone(),
            tags,
            destination.clone(),
            staged.outcome().path.clone(),
            staged.outcome().checksum,
        );
        match staged.outcome().disposition {
            ExportDisposition::Created => {
                self.storage.record_article_saved(snapshot)?;
            }
            ExportDisposition::Updated => {
                self.storage.record_article_updated(snapshot)?;
            }
            ExportDisposition::Unchanged => {}
        }
        let outcome = staged.commit()?;
        if outcome.disposition != ExportDisposition::Unchanged {
            self.emit()?;
        }
        Ok(SaveOutcome {
            destination: destination.clone(),
            path: outcome.path,
            checksum: outcome.checksum,
            disposition: outcome.disposition,
        })
    }

    /// Exports every article matching `filter` to a named destination —
    /// the export-everything path ("my library as markdown"). Each note
    /// goes through the same per-article pipeline as
    /// [`Self::save_to_destination`] (frontmatter, managed region,
    /// manifest idempotency, `article.saved`/`article.updated` events), so
    /// re-running a bulk export only rewrites what actually changed.
    /// `filter.before`/`limit` are pagination internals and are ignored.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`]; the first per-article
    /// export/storage/emission error aborts the walk (already-written
    /// notes stay — each save is atomic).
    pub fn save_all_to_destination(
        &self,
        filter: &ListArticles,
        destination: &DestinationName,
    ) -> Result<BulkSaveOutcome, CoreError> {
        // Fail on an unknown destination before walking anything.
        if !self.lock_destinations().contains_key(destination) {
            return Err(CoreError::UnknownDestination {
                name: destination.to_string(),
            });
        }
        let mut page_filter = filter.clone();
        page_filter.before = None;
        page_filter.limit = 200;
        let mut outcome = BulkSaveOutcome::default();
        loop {
            let page = self.storage.list_articles(page_filter.clone())?;
            let page_len = page.len();
            for article in page {
                page_filter.before = Some(article.id);
                match self
                    .save_to_destination(article.id, destination)?
                    .disposition
                {
                    ExportDisposition::Created => outcome.created += 1,
                    ExportDisposition::Updated => outcome.updated += 1,
                    ExportDisposition::Unchanged => outcome.unchanged += 1,
                }
            }
            if page_len < page_filter.limit as usize {
                break;
            }
        }
        Ok(outcome)
    }

    // ---------------------------------------------------------- imports

    /// Imports an OPML document, subscribing every feed not already
    /// known (each emits `feed.added` with its folder/category tags).
    ///
    /// Thin wrapper over [`CoreHandle::import_file`]; OPML carries only
    /// feeds, so the article counters are always zero here.
    ///
    /// # Errors
    ///
    /// OPML parse, storage or emission errors.
    pub fn import_opml(&self, xml: &str) -> Result<OpmlImportOutcome, CoreError> {
        let outcome = self.import_file(ImportSource::Opml, xml)?;
        Ok(OpmlImportOutcome {
            added: outcome.feeds_added,
            skipped: outcome.feeds_skipped,
        })
    }

    /// Parses `content` as `source` and applies it — the one entry point
    /// for every refugee import (OPML, Pocket, Instapaper, Readwise).
    ///
    /// # Errors
    ///
    /// [`CoreError::Import`] if the file cannot be parsed, plus the storage
    /// and emission errors of [`CoreHandle::import_items`].
    pub fn import_file(
        &self,
        source: ImportSource,
        content: &str,
    ) -> Result<ImportOutcome, CoreError> {
        self.import_items(import::parse(source, content)?)
    }

    /// Applies a neutral [`ImportedItem`] stream: feed items become
    /// subscriptions (each emits `feed.added`), article items become
    /// feedless read-later saves — upserted, flagged read-later, and
    /// tagged, each flagging/tagging staging its own event. URLs already
    /// present (a subscribed feed, a previously imported article) are
    /// skipped, so re-importing the same export is idempotent. Every
    /// staged event flushes in a single emission at the end.
    ///
    /// # Errors
    ///
    /// Storage or emission errors.
    pub fn import_items(&self, items: Vec<ImportedItem>) -> Result<ImportOutcome, CoreError> {
        let mut outcome = ImportOutcome::default();
        for item in items {
            match item.kind {
                ImportKind::Feed => {
                    if self.storage.get_feed_by_url(&item.url)?.is_some() {
                        outcome.feeds_skipped += 1;
                        continue;
                    }
                    self.storage.add_feed(NewFeed {
                        url: item.url,
                        title: item.title,
                        tags: item.tags,
                    })?;
                    outcome.feeds_added += 1;
                }
                ImportKind::Article => {
                    let title = item.title.unwrap_or_else(|| item.url.clone());
                    let key =
                        crate::feeds::dedupe_key(None, Some(&item.url), &title, item.saved_at);
                    if self.storage.article_id_by_dedupe_key(None, &key)?.is_some() {
                        outcome.articles_skipped += 1;
                        continue;
                    }
                    self.storage.upsert_articles(vec![NewArticle {
                        feed_id: None,
                        dedupe_key: key.clone(),
                        title,
                        source_url: item.url,
                        author: None,
                        published_at: item.saved_at,
                        content: ArticleContent::default(),
                        lang: None,
                        word_count: None,
                        source_updated_at: item.saved_at,
                        lead_image: None,
                    }])?;
                    // The row exists now; look its id back up to flag it.
                    // upsert_articles does not return ids (feed refresh
                    // never needs them), so the importer reads it back.
                    let id = self.storage.article_id_by_dedupe_key(None, &key)?.ok_or(
                        CoreError::NotFound {
                            entity: "imported article",
                        },
                    )?;
                    self.storage.add_read_later(id)?;
                    for tag in item.tags {
                        self.storage.tag_article(id, &tag)?;
                    }
                    outcome.articles_added += 1;
                }
            }
        }
        self.emit()?;
        Ok(outcome)
    }

    /// Exports every subscription as OPML 2.0.
    ///
    /// # Errors
    ///
    /// Storage or OPML write errors.
    pub fn export_opml(&self) -> Result<String, CoreError> {
        let feeds = self
            .storage
            .list_feeds()?
            .into_iter()
            .map(|feed| OpmlFeed {
                xml_url: feed.url,
                title: feed.title,
                html_url: feed.site_url,
                tags: feed.tags,
            })
            .collect::<Vec<_>>();
        Ok(crate::feeds::export_opml("Curio subscriptions", &feeds)?)
    }

    // ------------------------------------------------------ maintenance

    /// Deletes event-log files older than the retention window (≥ 90
    /// days — the `curio.events.v1` floor), measured from today (UTC).
    /// Returns the removed paths.
    ///
    /// Facade-owned on purpose: the event log is single-writer and the
    /// handle owns that writer, so heads must never open their own
    /// [`crate::events::EventLog`] to sweep.
    ///
    /// # Errors
    ///
    /// Filesystem errors scanning or deleting log files.
    pub fn sweep_event_retention(&self) -> Result<Vec<PathBuf>, CoreError> {
        let today = Timestamp::now().as_datetime().date_naive();
        let removed = self
            .emitter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .log_mut()
            .sweep_retention(today)
            .map_err(EventsError::Log)?;
        Ok(removed)
    }

    // ---------------------------------------------------------- private

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

    /// Serializes the registry into the reserved settings key. Callers
    /// adopt the new map only after this succeeds — a failed persist must
    /// never leave an in-memory registry that vanishes on the next open.
    fn persist_destinations(
        &self,
        map: &BTreeMap<DestinationName, PathBuf>,
    ) -> Result<(), CoreError> {
        let serialized = serde_json::to_string(
            &map.iter()
                .map(|(n, p)| (n.to_string(), p.display().to_string()))
                .collect::<BTreeMap<_, _>>(),
        )?;
        self.storage.set_setting(DESTINATIONS_KEY, &serialized)?;
        Ok(())
    }

    /// Drains staged event intents to the JSONL log (fsync before the
    /// intents are deleted — the crash-safe ordering).
    fn emit(&self) -> Result<(), CoreError> {
        self.emitter
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .emit_pending()?;
        Ok(())
    }

    fn emit_if(&self, changed: bool) -> Result<bool, CoreError> {
        if changed {
            self.emit()?;
        }
        Ok(changed)
    }

    fn lock_destinations(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, BTreeMap<DestinationName, PathBuf>> {
        self.destinations
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_destinations_mut(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, BTreeMap<DestinationName, PathBuf>> {
        self.destinations
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Turns a parsed entry into a storage row via the content pipeline
/// (sanitize → extract → text — the D6 invariant lives here).
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

fn load_destinations(storage: &Storage) -> Result<BTreeMap<DestinationName, PathBuf>, CoreError> {
    let Some(raw) = storage.get_setting(DESTINATIONS_KEY)? else {
        return Ok(BTreeMap::new());
    };
    let parsed: BTreeMap<String, String> = serde_json::from_str(&raw)?;
    let mut map = BTreeMap::new();
    for (name, root) in parsed {
        match name.parse::<DestinationName>() {
            Ok(name) => {
                map.insert(name, PathBuf::from(root));
            }
            Err(err) => {
                tracing::warn!(%name, %err, "skipping invalid destination name in settings");
            }
        }
    }
    Ok(map)
}
