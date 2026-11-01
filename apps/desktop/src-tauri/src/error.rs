//! The one error type that crosses IPC.
//!
//! Every command returns `Result<T, CommandError>`; the wire shape is the
//! sketch's proven three-tier model ported onto the generated contract:
//! `{ kind, code, params, message, recoverable }`. `kind: "user"` means the
//! frontend should surface the failure to the human (bad input, missing
//! row, network trouble); `kind: "internal"` means something the user
//! cannot fix — show a generic failure and point at diagnostics. Every
//! internal-tier error a command returns is logged once, with the command's
//! name, by [`logged`].
//!
//! **Localisation.** The app ships in eight locales, so the frontend shows
//! its own copy for a `code` rather than `message` (English): it maps the
//! code through i18n and fills the copy's `{placeholders}` from `params` (a
//! host, an HTTP status, a folder). `message` stays the English fallback
//! and the diagnostic detail. The broad codes (`NOT_FOUND`,
//! `INVALID_INPUT`, `NETWORK`, `PARSE`, `STORAGE`, `IO`) get a generic
//! localized line; the finer ones carry the params their copy names.
//!
//! Mapping policy (from the build spec): `NotFound` / `UnknownDestination`
//! / `InvalidTag` / `SchemaTooNew` (`OpenError::TooNew`) are user-tier, as
//! is a missing destination folder (`ExportError::DestinationMissing`, code
//! `DESTINATION_MISSING`) and a profile on network or sync-managed storage
//! (`OpenError::NetworkFs`, code `IO`). A subscribe that collides with an
//! existing feed is `ALREADY_SUBSCRIBED` — checked before any insert, and
//! the same code for the insert's own `StorageError::Duplicate` should a
//! concurrent subscribe win the race; any other duplicate row is
//! `DUPLICATE`. A full disk (`StorageError::DiskFull`, or a full export
//! destination — `ExportError::DiskFull`) is `DISK_FULL`, recoverable, and a
//! read-only profile (`StorageError::ReadOnly`) is `READ_ONLY`. A fetch
//! failure is user-tier and recoverable, its code read off the core's typed
//! [`FetchErrorKind`]. `Sqlite` / `DatabaseCorrupt` / `WriterGone` /
//! `JobPanicked` / `PoolTimeout` / `IntegrityFailed` / an unwritable export
//! destination (`ExportError::PermissionDenied`, which has no code of its
//! own yet) / other IO / a failed upgrade are internal-tier.
//!
//! Contention with another Curio process on the profile — a busy database
//! (`StorageError::Busy`, `OpenError::Locked`) or a held events lock
//! (`EventLogError::ProfileLocked`) — is user-tier, recoverable, code
//! `PROFILE_BUSY`.

use std::collections::BTreeMap;

use curio_core::export::ExportError;
use curio_core::fetch::FetchError;
use curio_core::model::FetchErrorKind;
use curio_core::storage::{OpenError, StorageError};
use curio_core::{CoreError, RedditAuthReason};
use curio_types::ParseDestinationNameError;
use serde::{Deserialize, Serialize};

/// Which tier of the UI handles the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// Actionable by the user — surface the localized copy for `code`.
    User,
    /// Not actionable by the user — show a generic failure.
    Internal,
}

/// Stable machine-readable code the frontend branches on and localizes.
/// A code's doc names the [`CommandError::params`] its copy fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The referenced feed/article/row does not exist.
    NotFound,
    /// The input failed validation (bad tag, bad name, bad token…).
    InvalidInput,
    /// The input is not an http(s) URL Curio can fetch. Params: `input`.
    InvalidUrl,
    /// A picked-path token is unknown or already used — the user picks the
    /// file or folder again.
    PathTokenExpired,
    /// No destination is registered under that name.
    UnknownDestination,
    /// A registered destination's folder is gone (moved, renamed,
    /// unmounted). Params: `root`.
    DestinationMissing,
    /// Every candidate path for a note is taken by other files. Params:
    /// `path`.
    PathsTaken,
    /// The database was written by a newer build of Curio.
    SchemaTooNew,
    /// An outbound fetch failed in a way no finer code names (a refused
    /// redirect).
    Network,
    /// Connecting, or the answer, took too long. Params: `host`.
    NetworkTimeout,
    /// The host name did not resolve. Params: `host`.
    HostNotFound,
    /// The connection was refused, reset or failed on the wire. Params:
    /// `host`.
    ConnectionFailed,
    /// The TLS certificate or handshake was rejected. Params: `host`.
    Tls,
    /// The server answered with a non-success HTTP status. Params: `host`,
    /// `status`.
    HttpStatus,
    /// The SSRF guard refused a private / non-public address. Params:
    /// `host`.
    BlockedAddress,
    /// The response exceeded the size cap. Params: `host`.
    TooLarge,
    /// The host is rate-limiting Curio. Params: `host`, `minutes`.
    RateLimited,
    /// Reddit refused a full-post request for auth reasons. Params:
    /// `reason` — which action fixes it: `credentials_missing` (add Reddit
    /// API credentials), `credentials_rejected` (correct them),
    /// `token_revoked` (retry, else re-create the app),
    /// `token_unavailable` (retry later), or `unknown` for a reason added
    /// to the core since — and `status`, the refusal's HTTP status.
    RedditAuth,
    /// Feed/OPML bytes did not parse.
    Parse,
    /// Subscribe-time validation: the page answered but neither it nor
    /// anything it declares (or the probes found) is a feed. The UI offers
    /// an explicit "subscribe anyway".
    NoFeed,
    /// The feed URL (as typed, or as a homepage resolved to) is already a
    /// subscription.
    AlreadySubscribed,
    /// The storage layer failed.
    Storage,
    /// The row already exists. Params: `entity` (English, diagnostic).
    Duplicate,
    /// The disk holding the profile (or an export destination) is full.
    DiskFull,
    /// The profile database or its folder is read-only.
    ReadOnly,
    /// Another Curio process (a CLI run, a cron job) holds the profile —
    /// its database stayed busy or its events lock stayed held. Retrying
    /// once it finishes succeeds.
    ProfileBusy,
    /// Filesystem IO failed.
    Io,
    /// Anything else — a bug, not a state.
    Internal,
}

/// The serializable error every command returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type, thiserror::Error)]
#[error("{message}")]
pub struct CommandError {
    /// User-actionable vs internal.
    pub kind: ErrorKind,
    /// Machine-readable code — what the frontend localizes.
    pub code: ErrorCode,
    /// Values for the `{placeholders}` of the code's localized copy (a
    /// host, an HTTP status, a folder). Empty for most codes.
    pub params: BTreeMap<String, String>,
    /// English description: the fallback when the UI has no copy of its
    /// own, and the diagnostic detail (logged for internal errors).
    pub message: String,
    /// Whether retrying the same action can plausibly succeed.
    pub recoverable: bool,
}

impl CommandError {
    /// A user-tier error: surfaced to the user (localized by `code`).
    pub fn user(code: ErrorCode, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            kind: ErrorKind::User,
            code,
            params: BTreeMap::new(),
            message: message.into(),
            recoverable,
        }
    }

    /// An internal-tier error: shown generically, and logged once — with
    /// the command's name — by [`logged`] at the command boundary.
    pub fn internal(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            code,
            params: BTreeMap::new(),
            message: message.into(),
            recoverable: false,
        }
    }

    /// Adds a `{placeholder}` value for the code's localized copy.
    #[must_use]
    pub fn with_param(mut self, key: &str, value: impl Into<String>) -> Self {
        self.params.insert(key.to_owned(), value.into());
        self
    }

    /// Contention with another Curio process on the profile: user-tier,
    /// recoverable, [`ErrorCode::ProfileBusy`].
    fn profile_busy(error: &dyn std::fmt::Display) -> Self {
        Self::user(ErrorCode::ProfileBusy, error.to_string(), true)
    }

    /// Shorthand for invalid user input.
    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::user(ErrorCode::InvalidInput, message, true)
    }

    /// A picked-path token that is unknown or already redeemed: the user
    /// picks the file or folder again.
    #[must_use]
    pub fn path_token_expired() -> Self {
        Self::user(
            ErrorCode::PathTokenExpired,
            "unknown or already-used path token — pick the path again",
            true,
        )
    }

    /// A failed fetch: user-tier and recoverable, its code read off the
    /// core's typed [`FetchErrorKind`] and its `host` param off the URL.
    /// `message` is the English description (a caller may have added
    /// context to the fetch error's own, as discovery does).
    #[must_use]
    pub fn fetch(error: &FetchError, message: String) -> Self {
        let code = match error.kind() {
            FetchErrorKind::InvalidUrl => {
                return Self::user(ErrorCode::InvalidUrl, message, true)
                    .with_param("input", fetch_target(error));
            }
            FetchErrorKind::Dns => ErrorCode::HostNotFound,
            FetchErrorKind::Blocked => ErrorCode::BlockedAddress,
            FetchErrorKind::Refused | FetchErrorKind::Reset | FetchErrorKind::Transport => {
                ErrorCode::ConnectionFailed
            }
            FetchErrorKind::TlsCertificate | FetchErrorKind::TlsHandshake => ErrorCode::Tls,
            FetchErrorKind::ConnectTimeout | FetchErrorKind::ReadTimeout => {
                ErrorCode::NetworkTimeout
            }
            FetchErrorKind::TooLarge => ErrorCode::TooLarge,
            // Refused redirects, and the response-level kinds (HTTP status,
            // parse, busy) a bare `FetchError` never carries.
            _ => ErrorCode::Network,
        };
        Self::user(code, message, true).with_param("host", host_of(fetch_target(error)))
    }
}

/// The URL (or, for DNS and SSRF refusals, the host) a fetch error names.
fn fetch_target(error: &FetchError) -> &str {
    match error {
        FetchError::Dns { host, .. }
        | FetchError::DnsNoAddresses { host }
        | FetchError::PrivateAddress { host, .. } => host,
        FetchError::InvalidUrl { url, .. }
        | FetchError::UnsupportedScheme { url, .. }
        | FetchError::UserinfoForbidden { url }
        | FetchError::NoHost { url }
        | FetchError::RedirectNotAllowed { url }
        | FetchError::TooManyRedirects { url, .. }
        | FetchError::CrossOriginRedirect { url, .. }
        | FetchError::MissingRedirectLocation { url }
        | FetchError::BodyTooLarge { url, .. }
        | FetchError::Timeout { url, .. }
        | FetchError::Transport { url, .. } => url,
    }
}

/// The host of `target` when it parses as a URL, else `target` itself (a
/// bare host already).
pub(crate) fn host_of(target: &str) -> String {
    url::Url::parse(target)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| target.to_owned())
}

/// The command boundary's one log point: an internal-tier error is written
/// to the diagnostics log once here, at `error` level, naming the
/// `command` that returned it. (A panic also leaves its own record, from
/// the panic hook — and, in a storage writer job, from the writer — which
/// names where it happened; this one names the command it failed.)
/// User-tier errors are the user's to act on and stay out of the log. Every command routes its result through here —
/// `commands::run_blocking` does it for the blocking ones, and a
/// test pins that each command names itself exactly once.
///
/// # Errors
///
/// Returns `result` unchanged — the error, if any, is passed through.
pub fn logged<T>(
    command: &'static str,
    result: Result<T, CommandError>,
) -> Result<T, CommandError> {
    if let Err(error) = &result
        && error.kind == ErrorKind::Internal
    {
        tracing::error!(command, code = ?error.code, "command failed: {}", error.message);
    }
    result
}

impl From<StorageError> for CommandError {
    fn from(error: StorageError) -> Self {
        match &error {
            StorageError::NotFound { .. } => {
                Self::user(ErrorCode::NotFound, error.to_string(), false)
            }
            StorageError::InvalidTag => {
                Self::user(ErrorCode::InvalidInput, error.to_string(), true)
            }
            // Another Curio process (a CLI run) is writing to the
            // profile: tell the user why, and that a retry will work.
            StorageError::Busy(_) => Self::profile_busy(&error),
            // Subscribing to a URL that is already subscribed (a concurrent
            // subscribe won the race past the up-front check): the user's
            // input, and the same input fails the same way again.
            StorageError::Duplicate { entity: "feed", .. } => {
                Self::user(ErrorCode::AlreadySubscribed, error.to_string(), false)
            }
            StorageError::Duplicate { entity, .. } => {
                Self::user(ErrorCode::Duplicate, error.to_string(), false)
                    .with_param("entity", *entity)
            }
            // The user can act on both — free space and retry; fix the
            // permissions of the profile folder or volume.
            StorageError::DiskFull(_) => Self::user(ErrorCode::DiskFull, error.to_string(), true),
            StorageError::ReadOnly(_) => Self::user(ErrorCode::ReadOnly, error.to_string(), false),
            StorageError::Sqlite(_)
            | StorageError::DatabaseCorrupt(_)
            | StorageError::WriterGone
            | StorageError::JobPanicked { .. }
            | StorageError::PoolTimeout { .. }
            | StorageError::WalUnavailable { .. }
            | StorageError::Corrupt { .. }
            | StorageError::IntegrityFailed { .. }
            | StorageError::Envelope(_) => Self::internal(ErrorCode::Storage, error.to_string()),
        }
    }
}

impl From<OpenError> for CommandError {
    fn from(error: OpenError) -> Self {
        match error {
            OpenError::Storage(storage) => storage.into(),
            OpenError::TooNew { .. } => {
                Self::user(ErrorCode::SchemaTooNew, error.to_string(), false)
            }
            OpenError::Locked { .. } => Self::profile_busy(&error),
            OpenError::Io { .. } => Self::internal(ErrorCode::Io, error.to_string()),
            // The user's choice of location: they move the profile, or
            // set the override. Retrying as-is never helps.
            OpenError::NetworkFs { .. } => Self::user(ErrorCode::Io, error.to_string(), false),
            // Corrupt, Backup, MigrationFailed, and any class added later.
            _ => Self::internal(ErrorCode::Storage, error.to_string()),
        }
    }
}

impl From<CoreError> for CommandError {
    fn from(error: CoreError) -> Self {
        // Contention with another process on the profile (a busy
        // database, or the events lock held by a CLI run) is the same
        // story whichever layer hit it: retry once the other run ends.
        if error.is_profile_busy() {
            return Self::profile_busy(&error);
        }
        let message = error.to_string();
        match error {
            CoreError::Storage(storage) => storage.into(),
            CoreError::Open(open) => open.into(),
            CoreError::NotFound { .. } => Self::user(ErrorCode::NotFound, message, false),
            CoreError::UnknownDestination { .. } => {
                Self::user(ErrorCode::UnknownDestination, message, true)
            }
            CoreError::Fetch(fetch) => Self::fetch(&fetch, message),
            CoreError::InvalidUrl { url } => {
                Self::user(ErrorCode::InvalidUrl, message, true).with_param("input", url)
            }
            CoreError::ReservedSettingKey { .. } => {
                Self::user(ErrorCode::InvalidInput, message, true)
            }
            CoreError::RateLimited {
                host,
                retry_after_secs,
            } => Self::user(ErrorCode::RateLimited, message, true)
                .with_param("host", host)
                .with_param("minutes", retry_after_secs.div_ceil(60).to_string()),
            CoreError::RedditAuth { reason, status } => reddit_auth(reason, status, message),
            CoreError::FeedParse(_) | CoreError::Opml(_) | CoreError::Import(_) => {
                Self::user(ErrorCode::Parse, message, true)
            }
            // A vanished vault (moved, renamed, unmounted), or a note whose
            // every candidate path is taken by other files, is the user's to
            // fix — reconnect the folder or register the destination again;
            // rename or move the files on the named path — so it is shown,
            // not a generic failure.
            CoreError::Export(ExportError::DestinationMissing { root }) => {
                Self::user(ErrorCode::DestinationMissing, message, true)
                    .with_param("root", root.display().to_string())
            }
            CoreError::Export(ExportError::PathsTaken { path }) => {
                Self::user(ErrorCode::PathsTaken, message, true).with_param("path", path)
            }
            // The atomic writes left the previous note intact: free some
            // space and export again.
            CoreError::Export(ExportError::DiskFull { .. }) => {
                Self::user(ErrorCode::DiskFull, message, true)
            }
            // Includes the typed `ExportError::PermissionDenied`: its
            // message is the core's English text, so it stays internal until
            // it has a localized code of its own.
            CoreError::Io { .. } | CoreError::Export(_) | CoreError::Events(_) => {
                Self::internal(ErrorCode::Io, message)
            }
            CoreError::Content(_) | CoreError::Settings(_) | CoreError::Panicked { .. } => {
                Self::internal(ErrorCode::Internal, message)
            }
        }
    }
}

/// A reddit auth refusal: its own code, so every locale gets the guidance
/// the core's English names (add, correct or re-authorise the credentials)
/// rather than a generic network line. Only a token problem clears on a
/// retry; missing or refused credentials fail the same way until the user
/// changes them in Settings.
fn reddit_auth(reason: RedditAuthReason, status: u16, message: String) -> CommandError {
    let (param, recoverable) = match reason {
        RedditAuthReason::CredentialsMissing => ("credentials_missing", false),
        RedditAuthReason::CredentialsRejected => ("credentials_rejected", false),
        RedditAuthReason::TokenRevoked => ("token_revoked", true),
        RedditAuthReason::TokenUnavailable => ("token_unavailable", true),
        // A reason added to the core since: the generic Reddit line.
        _ => ("unknown", true),
    };
    CommandError::user(ErrorCode::RedditAuth, message, recoverable)
        .with_param("reason", param)
        .with_param("status", status.to_string())
}

impl From<ParseDestinationNameError> for CommandError {
    fn from(error: ParseDestinationNameError) -> Self {
        Self::invalid_input(format!("invalid destination name: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use curio_core::storage::NetworkFsKind;

    #[test]
    fn not_found_maps_to_the_user_tier() {
        let error = CommandError::from(CoreError::NotFound { entity: "article" });
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
        assert!(!error.recoverable);
    }

    #[test]
    fn invalid_tag_is_recoverable_user_input() {
        let error = CommandError::from(StorageError::InvalidTag);
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert!(error.recoverable);
    }

    #[test]
    fn unknown_destination_is_user_tier() {
        let error = CommandError::from(CoreError::UnknownDestination {
            name: "vault".into(),
        });
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::UnknownDestination);
    }

    #[test]
    fn user_fixable_export_failures_are_user_tier_and_recoverable() {
        let error = CommandError::from(CoreError::Export(ExportError::DestinationMissing {
            root: "/vault".into(),
        }));
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::DestinationMissing);
        assert!(error.recoverable);
        assert!(error.message.contains("missing"), "{}", error.message);
        assert_eq!(error.params.get("root").map(String::as_str), Some("/vault"));
        let taken = CommandError::from(CoreError::Export(ExportError::PathsTaken {
            path: "curio/busy-0197b2c4.md".into(),
        }));
        assert_eq!(taken.kind, ErrorKind::User);
        assert_eq!(taken.code, ErrorCode::PathsTaken);
        assert!(taken.recoverable);
        assert_eq!(
            taken.params.get("path").map(String::as_str),
            Some("curio/busy-0197b2c4.md")
        );
        assert!(
            taken.message.contains("curio/busy-0197b2c4.md"),
            "{}",
            taken.message
        );
        // Other export failures stay internal.
        let other = CommandError::from(CoreError::Export(ExportError::Containment {
            path: "x".into(),
        }));
        assert_eq!(other.kind, ErrorKind::Internal);
    }

    #[test]
    fn a_full_export_destination_is_disk_full_and_a_refused_one_stays_internal() {
        use std::io::{Error, ErrorKind as IoKind};
        let full = ExportError::io("/vault/curio/note.md", Error::from(IoKind::StorageFull));
        assert!(matches!(full, ExportError::DiskFull { .. }), "{full:?}");
        let error = CommandError::from(CoreError::Export(full));
        assert_eq!(
            (error.kind, error.code, error.recoverable),
            (ErrorKind::User, ErrorCode::DiskFull, true),
            "{error}"
        );
        // No localized code of its own yet: its English text stays out of
        // the UI.
        let denied = ExportError::io(
            "/vault/curio/note.md",
            Error::from(IoKind::PermissionDenied),
        );
        assert!(
            matches!(denied, ExportError::PermissionDenied { .. }),
            "{denied:?}"
        );
        let error = CommandError::from(CoreError::Export(denied));
        assert_eq!(
            (error.kind, error.code),
            (ErrorKind::Internal, ErrorCode::Io),
            "{error}"
        );
    }

    #[test]
    fn reddit_auth_refusals_are_user_facing_guidance() {
        // Each reason names the action that fixes it, and only a token
        // problem is worth a Retry.
        for (reason, param, recoverable) in [
            (
                RedditAuthReason::CredentialsMissing,
                "credentials_missing",
                false,
            ),
            (
                RedditAuthReason::CredentialsRejected,
                "credentials_rejected",
                false,
            ),
            (RedditAuthReason::TokenRevoked, "token_revoked", true),
            (
                RedditAuthReason::TokenUnavailable,
                "token_unavailable",
                true,
            ),
        ] {
            let error = CommandError::from(CoreError::RedditAuth {
                reason,
                status: 403,
            });
            assert_eq!(
                (error.kind, error.code, error.recoverable),
                (ErrorKind::User, ErrorCode::RedditAuth, recoverable),
                "{reason:?}"
            );
            assert_eq!(
                error.params.get("reason").map(String::as_str),
                Some(param),
                "{reason:?}"
            );
            assert_eq!(
                error.params.get("status").map(String::as_str),
                Some("403"),
                "{reason:?}"
            );
            assert!(error.message.contains("HTTP 403"), "{reason:?}: {error}");
        }
        assert_eq!(
            serde_json::to_string(&ErrorCode::RedditAuth).unwrap_or_default(),
            "\"REDDIT_AUTH\""
        );
    }

    #[test]
    fn a_busy_profile_is_a_recoverable_user_facing_error() {
        let error = CommandError::from(StorageError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
            None,
        )));
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::ProfileBusy);
        assert!(error.recoverable);
        assert!(error.message.contains("another Curio process"), "{error}");
    }

    #[test]
    fn typed_storage_classes_reach_the_user_with_their_own_localized_codes() {
        let sqlite = |code| {
            StorageError::from(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(code),
                Some("UNIQUE constraint failed: feeds.url".to_owned()),
            ))
        };
        let duplicate = CommandError::from(sqlite(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE));
        assert_eq!(
            (duplicate.kind, duplicate.code, duplicate.recoverable),
            (ErrorKind::User, ErrorCode::AlreadySubscribed, false)
        );
        assert_eq!(duplicate.message, "feed already exists");
        let other = CommandError::from(StorageError::from(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE),
            Some("UNIQUE constraint failed: tags.name".to_owned()),
        )));
        assert_eq!(other.code, ErrorCode::Duplicate);
        assert!(other.params.contains_key("entity"), "{other:?}");
        let full = CommandError::from(sqlite(rusqlite::ffi::SQLITE_FULL));
        assert_eq!(
            (full.kind, full.code, full.recoverable),
            (ErrorKind::User, ErrorCode::DiskFull, true)
        );
        let read_only = CommandError::from(sqlite(rusqlite::ffi::SQLITE_READONLY));
        assert_eq!(
            (read_only.kind, read_only.code, read_only.recoverable),
            (ErrorKind::User, ErrorCode::ReadOnly, false)
        );
        let corrupt = CommandError::from(sqlite(rusqlite::ffi::SQLITE_CORRUPT));
        assert_eq!(
            (corrupt.kind, corrupt.code),
            (ErrorKind::Internal, ErrorCode::Storage)
        );
    }

    #[test]
    fn a_database_locked_at_open_is_profile_busy_too() {
        let locked = OpenError::Locked {
            path: "/profile/curio.db".into(),
            source: rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY),
                None,
            ),
        };
        for error in [
            CommandError::from(CoreError::Open(locked)),
            CommandError::from(OpenError::Locked {
                path: "/profile/curio.db".into(),
                source: rusqlite::Error::SqliteFailure(
                    rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_LOCKED),
                    None,
                ),
            }),
        ] {
            assert_eq!(error.kind, ErrorKind::User);
            assert_eq!(error.code, ErrorCode::ProfileBusy);
            assert!(error.recoverable);
        }
    }

    #[test]
    fn the_profile_busy_code_is_screaming_snake_on_the_wire() {
        assert_eq!(
            serde_json::to_string(&ErrorCode::ProfileBusy).unwrap_or_default(),
            "\"PROFILE_BUSY\""
        );
    }

    #[test]
    fn a_held_events_lock_is_a_recoverable_user_facing_error() {
        use curio_core::events::{EventLogError, EventsError};
        let error = CommandError::from(CoreError::Events(EventsError::Log(
            EventLogError::ProfileLocked {
                path: "events.lock".into(),
                waited_ms: 15_000,
            },
        )));
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::ProfileBusy);
        assert!(error.recoverable);
        // Any other emission failure stays internal.
        let error = CommandError::from(CoreError::Events(EventsError::Log(EventLogError::Io(
            std::io::Error::other("disk"),
        ))));
        assert_eq!(error.kind, ErrorKind::Internal);
    }

    #[test]
    fn panicked_jobs_and_pool_timeouts_are_internal() {
        for storage in [
            StorageError::JobPanicked {
                message: "bug".into(),
            },
            StorageError::PoolTimeout { waited_ms: 30_000 },
        ] {
            let error = CommandError::from(storage);
            assert_eq!(error.kind, ErrorKind::Internal);
            assert_eq!(error.code, ErrorCode::Storage);
        }
        // A panic in a refresh's blocking half is a bug, not user input.
        let error = CommandError::from(CoreError::Panicked {
            message: "parser bug".into(),
        });
        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(error.code, ErrorCode::Internal);
    }

    #[test]
    fn writer_gone_is_internal_storage() {
        let error = CommandError::from(StorageError::WriterGone);
        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(!error.recoverable);
    }

    #[test]
    fn schema_too_new_is_a_user_facing_stop() {
        let error = CommandError::from(CoreError::Open(OpenError::TooNew {
            found: 9,
            supported: 1,
        }));
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::SchemaTooNew);
        assert!(!error.recoverable);
    }

    #[test]
    fn open_error_classes_keep_their_tier() {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("tempdir: {e}"));
        let garbage = dir.path().join("curio.db");
        std::fs::write(
            &garbage,
            b"definitely not sqlite, just long enough to have a header",
        )
        .unwrap_or_else(|e| panic!("write: {e}"));
        let corrupt = match curio_core::storage::Storage::open(&garbage) {
            Err(open) => CommandError::from(open),
            Ok(_) => panic!("a garbage file opened"),
        };
        assert_eq!(corrupt.kind, ErrorKind::Internal);
        assert_eq!(corrupt.code, ErrorCode::Storage);

        let wrapped = CommandError::from(OpenError::Storage(StorageError::InvalidTag));
        assert_eq!(wrapped.code, ErrorCode::InvalidInput);

        let network = CommandError::from(CoreError::Open(OpenError::NetworkFs {
            path: "/Volumes/share/curio/curio.db".into(),
            kind: NetworkFsKind::Smb,
        }));
        assert_eq!(network.kind, ErrorKind::User);
        assert_eq!(network.code, ErrorCode::Io);
        assert!(!network.recoverable);
        assert!(
            network.message.contains("CURIO_ALLOW_NETWORK_FS"),
            "{network}"
        );
    }

    #[test]
    fn wire_shape_matches_the_contract() {
        let error = CommandError::user(ErrorCode::NotFound, "feed not found", false);
        let json = serde_json::to_value(&error).unwrap_or_else(|e| panic!("serialize: {e}"));
        assert_eq!(json["kind"], "user");
        assert_eq!(json["code"], "NOT_FOUND");
        assert_eq!(json["message"], "feed not found");
        assert_eq!(json["recoverable"], false);
        assert_eq!(json["params"], serde_json::json!({}));

        let with_params = CommandError::user(ErrorCode::HttpStatus, "x answered 503", true)
            .with_param("host", "example.org")
            .with_param("status", "503");
        let json = serde_json::to_value(&with_params).unwrap_or_else(|e| panic!("serialize: {e}"));
        assert_eq!(json["code"], "HTTP_STATUS");
        assert_eq!(
            json["params"],
            serde_json::json!({ "host": "example.org", "status": "503" })
        );
    }

    #[test]
    fn fetch_failures_get_a_code_per_kind_and_the_host_as_a_param() {
        use curio_core::fetch::{TimeoutPhase, TransportKind};
        let url = "https://news.example.org/feed.xml".to_owned();
        let cases = [
            (
                FetchError::Timeout {
                    url: url.clone(),
                    phase: TimeoutPhase::Read,
                },
                ErrorCode::NetworkTimeout,
            ),
            (
                FetchError::Transport {
                    url: url.clone(),
                    kind: TransportKind::Refused,
                    cause: "Connection refused (os error 111)".into(),
                },
                ErrorCode::ConnectionFailed,
            ),
            (
                FetchError::Transport {
                    url: url.clone(),
                    kind: TransportKind::TlsCertificate,
                    cause: "invalid peer certificate: UnknownIssuer".into(),
                },
                ErrorCode::Tls,
            ),
            (
                FetchError::BodyTooLarge {
                    limit: 1,
                    url: url.clone(),
                },
                ErrorCode::TooLarge,
            ),
            (
                FetchError::Dns {
                    host: "news.example.org".into(),
                    message: "no such host".into(),
                },
                ErrorCode::HostNotFound,
            ),
            (
                FetchError::PrivateAddress {
                    host: "news.example.org".into(),
                    addr: std::net::IpAddr::from([10, 0, 0, 1]),
                },
                ErrorCode::BlockedAddress,
            ),
            (
                FetchError::TooManyRedirects {
                    limit: 5,
                    url: url.clone(),
                },
                ErrorCode::Network,
            ),
        ];
        for (fetch, code) in cases {
            let error = CommandError::from(CoreError::Fetch(fetch));
            assert_eq!(
                (error.kind, error.code),
                (ErrorKind::User, code),
                "{error:?}"
            );
            assert!(error.recoverable);
            assert_eq!(
                error.params.get("host").map(String::as_str),
                Some("news.example.org"),
                "{error:?}"
            );
        }
        let invalid = CommandError::from(CoreError::Fetch(FetchError::NoHost {
            url: "https://".into(),
        }));
        assert_eq!(invalid.code, ErrorCode::InvalidUrl);
        assert_eq!(
            invalid.params.get("input").map(String::as_str),
            Some("https://")
        );
    }

    #[test]
    fn rate_limits_carry_the_host_and_the_minutes_to_wait() {
        let error = CommandError::from(CoreError::RateLimited {
            host: "www.reddit.com".into(),
            retry_after_secs: 61,
        });
        assert_eq!(error.code, ErrorCode::RateLimited);
        assert_eq!(
            error.params.get("host").map(String::as_str),
            Some("www.reddit.com")
        );
        assert_eq!(error.params.get("minutes").map(String::as_str), Some("2"));
    }

    #[test]
    fn an_expired_path_token_has_its_own_code() {
        let registry = crate::ipc_policy::PathRegistry::new();
        let error = registry
            .redeem("never-minted", crate::ipc_policy::PathIntent::ImportFile)
            .err()
            .unwrap_or_else(|| panic!("an unknown token redeemed"));
        assert_eq!(
            (error.kind, error.code, error.recoverable),
            (ErrorKind::User, ErrorCode::PathTokenExpired, true)
        );
    }
}
