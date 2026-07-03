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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Instant;

use curio_types::{Destination, DestinationName, Timestamp};

use crate::content::{self, ContentError};
use crate::events::{EventEmitter, EventLog, EventsError};
use crate::export::{self, ExportDisposition, ExportError, ExportInput};
use crate::feeds::{FeedParseError, OpmlError, OpmlFeed, ParsedEntry};
use crate::fetch::{FetchConfig, FetchError, FetchRequest, PolicedClient};
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

/// Outcome of an OPML import.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OpmlImportOutcome {
    /// Feeds newly subscribed.
    pub added: usize,
    /// Feeds skipped because the URL was already subscribed.
    pub skipped: usize,
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

    /// Refreshes one feed: policed conditional GET → parse → sanitize →
    /// upsert → fetch-log row. Fetch/parse failures are *outcomes*
    /// (recorded, validators preserved), not errors; only storage-level
    /// failures surface as `Err`.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub async fn refresh_feed(&self, id: FeedId) -> Result<RefreshOutcome, CoreError> {
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
                    Ok(parsed) => self.ingest_parsed(id, &feed, &response, parsed, now)?,
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
        Ok(outcome)
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
    /// validators.
    fn ingest_parsed(
        &self,
        id: FeedId,
        feed: &Feed,
        response: &crate::fetch::FetchResponse,
        parsed: crate::feeds::ParsedFeed,
        now: Timestamp,
    ) -> Result<RefreshOutcome, CoreError> {
        let articles = parsed
            .entries
            .into_iter()
            .map(|entry| ingest_entry(id, &feed.url, entry))
            .collect();
        let upserted = self.storage.upsert_articles(articles)?;
        self.storage.update_feed_metadata(
            id,
            parsed.meta.title,
            parsed.meta.site_url,
            parsed.meta.description,
        )?;
        if response.permanent_redirect && response.final_url != feed.url {
            self.storage
                .update_feed_url(id, response.final_url.clone())?;
        }
        self.storage.update_feed_fetch_state(
            id,
            response.etag.clone(),
            response.last_modified.clone(),
            now,
        )?;
        Ok(RefreshOutcome {
            feed_id: id,
            status: FetchStatus::Ok,
            http_status: Some(response.status),
            new_articles: upserted.inserted,
            updated_articles: upserted.updated,
            error: None,
        })
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
    /// # Errors
    ///
    /// Filesystem or settings errors.
    pub fn add_destination(&self, name: DestinationName, root: PathBuf) -> Result<(), CoreError> {
        std::fs::create_dir_all(&root).map_err(|source| CoreError::Io {
            path: root.clone(),
            source,
        })?;
        let mut map = self.lock_destinations_mut();
        map.insert(name, root);
        let serialized = serde_json::to_string(
            &map.iter()
                .map(|(n, p)| (n.to_string(), p.display().to_string()))
                .collect::<BTreeMap<_, _>>(),
        )?;
        drop(map);
        self.storage.set_setting(DESTINATIONS_KEY, &serialized)?;
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
        let outcome = export::export_note(&dest, &input)?;

        let snapshot = article.snapshot(
            input.feed.clone(),
            input.feed_title.clone(),
            tags,
            destination.clone(),
            outcome.path.clone(),
            outcome.checksum,
        );
        match outcome.disposition {
            ExportDisposition::Created => {
                self.storage.record_article_saved(snapshot)?;
                self.emit()?;
            }
            ExportDisposition::Updated => {
                self.storage.record_article_updated(snapshot)?;
                self.emit()?;
            }
            ExportDisposition::Unchanged => {}
        }
        Ok(SaveOutcome {
            destination: destination.clone(),
            path: outcome.path,
            checksum: outcome.checksum,
            disposition: outcome.disposition,
        })
    }

    // ------------------------------------------------------------- opml

    /// Imports an OPML document, subscribing every feed not already
    /// known (each emits `feed.added` with its folder/category tags).
    ///
    /// # Errors
    ///
    /// OPML parse, storage or emission errors.
    pub fn import_opml(&self, xml: &str) -> Result<OpmlImportOutcome, CoreError> {
        let mut outcome = OpmlImportOutcome::default();
        for feed in crate::feeds::import_opml(xml)? {
            if self.storage.get_feed_by_url(&feed.xml_url)?.is_some() {
                outcome.skipped += 1;
                continue;
            }
            self.storage.add_feed(NewFeed {
                url: feed.xml_url,
                title: feed.title,
                tags: feed.tags,
            })?;
            outcome.added += 1;
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
                tags: Vec::new(),
            })
            .collect::<Vec<_>>();
        Ok(crate::feeds::export_opml("Curio subscriptions", &feeds)?)
    }

    // ---------------------------------------------------------- private

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
