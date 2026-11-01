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
//! A flush that fails (full disk, unwritable events dir) is **not** an
//! operation failure: the change is already committed and its intent
//! stays staged, so the operation still reports success, the handle
//! raises [`CoreHandle::has_pending_events`], and the next mutation, a
//! [`CoreHandle::flush_pending_events`] retry (the desktop head runs one
//! on a timer) or the next open drains the backlog.
//!
//! The operation surface is one `impl CoreHandle` block per concern:
//! `feeds`, `refresh` (one feed), `sweep` (many feeds, with `work_queue`
//! handing out its jobs fairly across hosts), `hydrate` (the
//! full-text page queue), `articles`,
//! `state`, `destinations`, `imports`, `settings` and `maintenance`. This
//! file holds the shared types, `open` /
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
mod hydrate;
mod imports;
mod maintenance;
mod reddit_auth;
mod refresh;
mod schedule;
mod settings;
mod settle;
mod sink;
mod state;
mod sweep;
mod work_queue;

pub use hydrate::DEFAULT_HYDRATE_CONCURRENCY;
pub use reddit_auth::RedditAuthReason;
pub use schedule::RefreshPolicy;
pub use sweep::{DEFAULT_REFRESH_CONCURRENCY, RefreshOptions, SweepProgress, SweepReport};

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::Mutex as AsyncMutex;

use curio_types::DestinationName;

use crate::content::ContentError;
use crate::events::EventsError;
use crate::export::{ExportDisposition, ExportError};
use crate::feeds::{FeedParseError, OpmlError};
use crate::fetch::{FetchConfig, FetchError, PolicedClient};
use crate::import::{ImportError, ImportKind, Rejected};
use crate::model::{Article, FeedId, FetchErrorKind, FetchStatus};
use crate::storage::{OpenError, Storage, StorageError};
use reddit_auth::reddit_auth_message;

/// Facade-level errors: every engine subsystem, plus facade concerns.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The profile database could not be opened — typed so heads can act
    /// on the class (corrupt, too new, locked, IO, failed upgrade).
    #[error(transparent)]
    Open(#[from] OpenError),
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
    /// registry, `"destinations.exported"`, `"destinations.seeded"`) and
    /// cannot be read or written as a raw value.
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
    /// Reddit refused the request for auth reasons — actionable by the
    /// user, and `reason` says which action: add credentials, fix them,
    /// or re-authorize the app.
    #[error("{}", reddit_auth_message(*reason, *status))]
    RedditAuth {
        /// Why reddit refused.
        reason: RedditAuthReason,
        /// The HTTP status of the refusal (of the token grant for
        /// [`RedditAuthReason::CredentialsRejected`], of the post request
        /// otherwise — for [`RedditAuthReason::TokenUnavailable`], the
        /// anonymous fallback's).
        status: u16,
    },
    /// A blocking engine task (a refresh's parse/ingest/storage half)
    /// panicked — a bug. The panic is contained: only this operation
    /// failed, and a sweep reports it as that feed's outcome.
    #[error("an engine task panicked: {message}")]
    Panicked {
        /// The panic message.
        message: String,
    },
}

/// Options for [`CoreHandle::open_with`].
#[derive(Debug, Clone, Default)]
pub struct CoreOptions {
    /// Policy configuration of the outbound client.
    pub fetch: FetchConfig,
    /// Open a profile on network or sync-managed storage anyway (NFS,
    /// SMB, iCloud Drive, Dropbox, Syncthing…), which is otherwise
    /// refused with [`OpenError::NetworkFs`]. Heads set it from an
    /// explicit user choice — the CLI's `--allow-network-fs`, or the
    /// [`crate::storage::ALLOW_NETWORK_FS_ENV`] variable
    /// ([`crate::storage::network_fs_override_from_env`]).
    pub allow_network_fs: bool,
    /// The per-feed refresh schedule: backoff, `Retry-After`, refresh
    /// hints and auto-pause.
    pub refresh: RefreshPolicy,
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
    /// Error detail for [`FetchStatus::Error`]: a short message naming the
    /// cause.
    pub error: Option<String>,
    /// Failure class for [`FetchStatus::Error`] — what a head branches on
    /// instead of parsing [`Self::error`].
    pub error_kind: Option<FetchErrorKind>,
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
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpmlImportOutcome {
    /// Feeds newly subscribed.
    pub added: usize,
    /// Feeds skipped because the URL was already subscribed.
    pub skipped: usize,
    /// Outlines dropped as unreadable or with an unusable URL.
    pub invalid: usize,
    /// Why outlines were dropped — the first [`MAX_REJECTED_DETAILS`] of
    /// them, in document order (`invalid` stays exact past the cap).
    pub rejected: Vec<Rejected>,
}

/// How many [`Rejected`] rows an [`ImportOutcome`] keeps in detail. The
/// counters stay exact past it; the list only has to explain the failures,
/// not mirror a garbage file row for row.
pub const MAX_REJECTED_DETAILS: usize = 100;

/// Outcome of a general [`CoreHandle::import_file`] / [`CoreHandle::import_items`]
/// run, split by kind so the UI can report "N feeds, M articles" honestly.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ImportOutcome {
    /// Feeds newly subscribed.
    pub feeds_added: usize,
    /// Feeds skipped because the URL was already subscribed.
    pub feeds_skipped: usize,
    /// Feed rows dropped: unreadable, or with an unusable URL.
    pub feeds_invalid: usize,
    /// Articles saved as feedless read-later items.
    pub articles_added: usize,
    /// Articles skipped because the URL was already imported.
    pub articles_skipped: usize,
    /// Article rows dropped: unreadable, or with an unusable URL.
    pub articles_invalid: usize,
    /// Why rows were dropped — the first [`MAX_REJECTED_DETAILS`] of them,
    /// in document order.
    pub rejected: Vec<Rejected>,
}

impl ImportOutcome {
    /// Counts one dropped row against its kind and keeps its reason while
    /// the detail list has room.
    fn reject(&mut self, rejected: Rejected) {
        match rejected.kind {
            ImportKind::Feed => self.feeds_invalid += 1,
            ImportKind::Article => self.articles_invalid += 1,
        }
        tracing::info!(
            kind = ?rejected.kind,
            reason = ?rejected.reason,
            detail = %rejected.detail,
            "import row dropped"
        );
        if self.rejected.len() < MAX_REJECTED_DETAILS {
            self.rejected.push(rejected);
        }
    }
}

/// The engine service object. Heads hold an `Arc<CoreHandle>` and
/// delegate; they contain no business logic of their own.
#[derive(Debug)]
pub struct CoreHandle {
    profile_dir: PathBuf,
    storage: Arc<Storage>,
    events: Mutex<sink::EventSink>,
    /// The "pending events" health flag: set when a flush failed and
    /// intents are left staged (or the log could not be opened at all),
    /// cleared by the next successful drain.
    events_backlog: AtomicBool,
    client: PolicedClient,
    /// Per-feed refresh serialization inside this process (see
    /// [`CoreHandle::refresh_feed`]); the per-feed lock files under
    /// `.curio/locks/` serialize across processes. Bounded by the
    /// subscription count; `remove_feed` drops entries.
    refresh_locks: Mutex<HashMap<FeedId, Arc<AsyncMutex<()>>>>,
    /// How long a refresh waits for another process to finish refreshing
    /// the same feed (derived from the fetch timeouts at open).
    refresh_lock_wait: Duration,
    /// The per-feed refresh schedule (see [`RefreshPolicy`]).
    refresh_policy: RefreshPolicy,
    /// The one refresh sweep allowed in flight (see
    /// [`CoreHandle::refresh_many`]): a second request joins it.
    sweep: sweep::SweepSlot,
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
    /// A profile on network or sync-managed storage is refused before
    /// anything is created (see [`CoreOptions::allow_network_fs`]). An
    /// events directory that cannot be opened does not fail the open: the
    /// handle comes up with [`CoreHandle::has_pending_events`] raised,
    /// mutations keep their intents staged, and a later drain opens the
    /// log once it is writable.
    ///
    /// # Errors
    ///
    /// [`CoreError::Open`] when the database cannot be opened (match the
    /// inner [`OpenError`] for the failure class, [`OpenError::NetworkFs`]
    /// included); [`CoreError::is_profile_busy`] contention on the
    /// database or the events lock; other [`CoreError`]s on filesystem or
    /// storage failures.
    pub fn open_with(
        profile_dir: impl AsRef<Path>,
        options: CoreOptions,
    ) -> Result<Self, CoreError> {
        let profile_dir = profile_dir.as_ref().to_path_buf();
        let db_path = profile_dir.join(crate::paths::DB_FILE);
        if let Some(kind) = crate::storage::detect_network_fs(&db_path) {
            if !options.allow_network_fs {
                return Err(OpenError::NetworkFs {
                    path: db_path,
                    kind,
                }
                .into());
            }
            tracing::warn!(
                path = %db_path.display(),
                %kind,
                "opening a profile on network or sync-managed storage (override set)"
            );
        }
        std::fs::create_dir_all(&profile_dir).map_err(|source| CoreError::Io {
            path: profile_dir.clone(),
            source,
        })?;
        let storage = Arc::new(Storage::open(&db_path)?);
        // The events log is attached lazily (see `sink`): an events
        // directory that cannot be opened degrades to the backlog flag
        // below instead of refusing a profile whose database is fine.
        // Only contention is reported, as the retryable "busy".
        let mut events =
            sink::EventSink::detached(Arc::clone(&storage), profile_dir.join(".curio"));
        let attached = match events.emitter() {
            Ok(_) => true,
            Err(error) if error.is_profile_busy() => return Err(error),
            Err(error) => {
                tracing::warn!(
                    %error,
                    "the event log cannot be opened; events stay staged until it can"
                );
                false
            }
        };
        let refresh_lock_wait = refresh::lock_wait(&options.fetch);
        let core = Self {
            profile_dir,
            storage,
            events: Mutex::new(events),
            events_backlog: AtomicBool::new(!attached),
            client: PolicedClient::new(options.fetch),
            refresh_locks: Mutex::new(HashMap::new()),
            refresh_lock_wait,
            refresh_policy: options.refresh,
            sweep: sweep::SweepSlot::default(),
            #[cfg(feature = "enrich-reddit")]
            enrich_breaker: crate::enrich::Breaker::default(),
            #[cfg(feature = "enrich-reddit")]
            reddit_auth: crate::enrich::reddit_auth::TokenManager::default(),
        };
        // The startup replay: intents a previous process left staged are
        // drained here, non-fatally (a failure raises the backlog flag).
        if attached {
            core.emit();
        }
        Ok(core)
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
    /// intents are deleted — the crash-safe ordering). Non-fatal by
    /// design: every caller has already committed its state change, so a
    /// failure is logged and raises the backlog flag instead of turning a
    /// stored change into a reported failure.
    fn emit(&self) {
        if let Err(error) = self.flush_pending_events() {
            tracing::warn!(
                %error,
                "event emission failed; the intents stay staged and are retried"
            );
        }
    }

    fn emit_if(&self, changed: bool) -> bool {
        if changed {
            self.emit();
        }
        changed
    }
}

impl CoreError {
    /// Whether this is transient contention with another process sharing
    /// the profile — the database stayed busy
    /// ([`StorageError::Busy`], or [`OpenError::Locked`] while opening) or
    /// the events lock stayed held
    /// ([`crate::events::EventLogError::ProfileLocked`]). Retrying the
    /// same operation once the other process finishes can succeed; heads
    /// surface it as "busy, try again" (the CLI exits with a distinct
    /// code).
    #[must_use]
    pub fn is_profile_busy(&self) -> bool {
        match self {
            Self::Storage(err)
            | Self::Events(EventsError::Storage(err))
            | Self::Open(OpenError::Storage(err)) => err.is_busy(),
            Self::Open(OpenError::Locked { .. })
            | Self::Events(EventsError::Log(crate::events::EventLogError::ProfileLocked {
                ..
            })) => true,
            _ => false,
        }
    }
}

/// The cross-process events lock, inside `.curio/` beside (never inside)
/// the `events/` contract directory (see [`EventLog`]).
const EVENTS_LOCK_FILE: &str = "events.lock";

/// The directory of the cross-process per-feed refresh locks, inside
/// `.curio/` (see [`CoreHandle::refresh_feed`]).
const LOCKS_DIR: &str = "locks";

/// Lines `.curio/.gitignore` must carry: the events directory, its lock
/// file and the refresh locks.
const GITIGNORE_LINES: [&str; 3] = ["events/", EVENTS_LOCK_FILE, "locks/"];

/// Writes `.curio/.gitignore`, or appends whichever required lines an
/// existing one (from an older build, or hand-edited) lacks.
fn ensure_gitignore(path: &Path) -> Result<(), CoreError> {
    let io = |source| CoreError::Io {
        path: path.to_path_buf(),
        source,
    };
    let existing = match std::fs::read_to_string(path) {
        Ok(existing) => existing,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(io(err)),
    };
    let missing: Vec<&str> = GITIGNORE_LINES
        .into_iter()
        .filter(|line| !existing.lines().any(|have| have.trim() == *line))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let mut next = existing;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    for line in missing {
        next.push_str(line);
        next.push('\n');
    }
    std::fs::write(path, next).map_err(io)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::events::EventLogError;

    fn busy() -> StorageError {
        StorageError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        ))
    }

    #[test]
    fn contention_from_either_side_reads_as_profile_busy() {
        assert!(CoreError::Storage(busy()).is_profile_busy());
        assert!(CoreError::Events(EventsError::Storage(busy())).is_profile_busy());
        assert!(
            CoreError::Events(EventsError::Log(EventLogError::ProfileLocked {
                path: PathBuf::from("events.lock"),
                waited_ms: 15_000,
            }))
            .is_profile_busy()
        );
        assert!(
            CoreError::Open(OpenError::Locked {
                path: PathBuf::from("curio.db"),
                source: rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                    None,
                ),
            })
            .is_profile_busy()
        );
        assert!(CoreError::Open(OpenError::Storage(busy())).is_profile_busy());
        assert!(
            !CoreError::Open(OpenError::TooNew {
                found: 99,
                supported: 8
            })
            .is_profile_busy()
        );
        assert!(!CoreError::Storage(StorageError::WriterGone).is_profile_busy());
        assert!(
            !CoreError::Events(EventsError::Log(EventLogError::Io(std::io::Error::other(
                "disk"
            ))))
            .is_profile_busy()
        );
        assert!(!CoreError::NotFound { entity: "article" }.is_profile_busy());
    }

    #[test]
    fn gitignore_is_written_then_upgraded_without_duplicates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".gitignore");
        ensure_gitignore(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "events/\nevents.lock\nlocks/\n"
        );

        // An older build's file (no lock lines, no trailing newline) gains
        // only the missing lines; the user's own lines stay.
        std::fs::write(&path, "events/\n*.bak").unwrap();
        ensure_gitignore(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "events/\n*.bak\nevents.lock\nlocks/\n"
        );
        ensure_gitignore(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "events/\n*.bak\nevents.lock\nlocks/\n",
            "idempotent"
        );
    }

    #[test]
    fn an_unreadable_gitignore_is_a_profile_io_error() {
        let dir = tempfile::tempdir().unwrap();
        // A directory where the file should be: reading it fails.
        let path = dir.path().join(".gitignore");
        std::fs::create_dir(&path).unwrap();
        assert!(matches!(ensure_gitignore(&path), Err(CoreError::Io { .. })));
    }
}
