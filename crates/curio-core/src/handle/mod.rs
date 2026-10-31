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
//!
//! The operation surface is one `impl CoreHandle` block per concern:
//! `feeds`, `refresh`, `articles`, `state`, `destinations`, `imports`,
//! `settings` and `maintenance`. This file holds the shared types, `open` /
//! `open_with`, the accessors (`version`, `profile_dir`), the
//! `enrich-reddit` API-credential setters (`set_reddit_api`,
//! `reddit_api_client_id`), and the emission helpers every mutation ends
//! with.
//!
//! Heads reach storage only through these methods. The raw `storage()`
//! accessor exists only under the `test-support` feature, which test code
//! enables through `[dev-dependencies]`, so a production head that
//! bypasses the facade does not compile. (Builds that include test
//! targets unify that feature into every head; `just check-prod` — a CI
//! step — type-checks libs + bins alone so the bypass is caught per
//! commit.)

mod articles;
mod destinations;
mod feeds;
mod imports;
mod maintenance;
mod refresh;
mod settings;
mod state;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use tokio::sync::Mutex as AsyncMutex;

use curio_types::DestinationName;

use crate::content::ContentError;
use crate::events::{EventEmitter, EventLog, EventsError};
use crate::export::{ExportDisposition, ExportError};
use crate::feeds::{FeedParseError, OpmlError};
use crate::fetch::{FetchConfig, FetchError, PolicedClient};
use crate::import::ImportError;
use crate::model::{Article, FeedId, FetchStatus};
use crate::storage::{Storage, StorageError};

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
    /// The settings key is owned by the facade (the `"destinations"`
    /// registry, `"destinations.exported"`) and cannot be read or
    /// written as a raw value.
    #[error("the {key:?} settings key is facade-owned — use the destination operations")]
    ReservedSettingKey {
        /// The refused key.
        key: String,
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
    /// Reddit refused the request for auth reasons (missing, rejected,
    /// or revoked credentials) — actionable by the user, so the detail
    /// is surfaced verbatim.
    #[error("{detail}")]
    RedditAuth {
        /// The user-facing guidance.
        detail: String,
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
    /// Existing articles this refresh actually changed (identical
    /// re-listed entries are not counted).
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
    /// Destinations that completed an export since they were
    /// (re-)registered: their root and `.curio/` marker must still exist
    /// (see [`CoreHandle::save_to_destination`]).
    exported_destinations: RwLock<BTreeSet<DestinationName>>,
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
        let destinations = destinations::load_destinations(&storage)?;
        let exported_destinations = destinations::load_exported(&storage)?;
        Ok(Self {
            profile_dir,
            storage,
            emitter: Mutex::new(emitter),
            client: PolicedClient::new(options.fetch),
            destinations: RwLock::new(destinations),
            exported_destinations: RwLock::new(exported_destinations),
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

    /// The raw storage layer, for tests that seed or inspect rows the
    /// facade deliberately does not expose (e.g. inserting an article
    /// without a fetch). Test-only: compiled in by the `test-support`
    /// feature, which no production build enables.
    #[cfg(feature = "test-support")]
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
}
