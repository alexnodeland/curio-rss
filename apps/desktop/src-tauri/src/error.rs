//! The one error type that crosses IPC.
//!
//! Every command returns `Result<T, CommandError>`; the wire shape is the
//! sketch's proven three-tier model ported onto the generated contract:
//! `{ kind, code, message, recoverable }`. `kind: "user"` means the
//! frontend should surface the message to the human (bad input, missing
//! row, network trouble); `kind: "internal"` means something the user
//! cannot fix — show a generic failure and point at diagnostics.
//!
//! Mapping policy (from the build spec): `NotFound` / `UnknownDestination`
//! / `InvalidTag` / `SchemaTooNew` (`OpenError::TooNew`) are user-tier, as
//! is a missing destination folder (`ExportError::DestinationMissing`, code
//! `IO`) and a profile on network or sync-managed storage
//! (`OpenError::NetworkFs`, code `IO`); `Sqlite` / `WriterGone` / `JobPanicked` / `PoolTimeout` /
//! `IntegrityFailed` / other IO / a corrupt or failed-upgrade database
//! are internal-tier.
//!
//! Contention with another Curio process on the profile — a busy database
//! (`StorageError::Busy`, `OpenError::Locked`) or a held events lock
//! (`EventLogError::ProfileLocked`) — is user-tier, recoverable, code
//! `PROFILE_BUSY`. The frontend shows its own localized copy for that
//! code rather than `message`, which is English and names the lock file.

use curio_core::CoreError;
use curio_core::export::ExportError;
use curio_core::storage::{OpenError, StorageError};
use curio_types::ParseDestinationNameError;
use serde::{Deserialize, Serialize};

/// Which tier of the UI handles the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "lowercase")]
pub enum ErrorKind {
    /// Actionable by the user — surface `message` verbatim.
    User,
    /// Not actionable by the user — show a generic failure.
    Internal,
}

/// Stable machine-readable code the frontend can branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// The referenced feed/article/row does not exist.
    NotFound,
    /// The input failed validation (bad tag, bad name, bad token…).
    InvalidInput,
    /// No destination is registered under that name.
    UnknownDestination,
    /// The database was written by a newer build of Curio.
    SchemaTooNew,
    /// An outbound fetch failed (policed client).
    Network,
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
    /// Another Curio process (a CLI run, a cron job) holds the profile —
    /// its database stayed busy or its events lock stayed held. Retrying
    /// once it finishes succeeds. The UI shows its own localized message
    /// for this code; `message` is the English diagnostic.
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
    /// Machine-readable code.
    pub code: ErrorCode,
    /// Human-readable description (shown verbatim for `kind: user`).
    pub message: String,
    /// Whether retrying the same action can plausibly succeed.
    pub recoverable: bool,
}

impl CommandError {
    /// A user-tier error: surfaced verbatim in the UI.
    pub fn user(code: ErrorCode, message: impl Into<String>, recoverable: bool) -> Self {
        Self {
            kind: ErrorKind::User,
            code,
            message: message.into(),
            recoverable,
        }
    }

    /// An internal-tier error: logged, shown generically.
    pub fn internal(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            kind: ErrorKind::Internal,
            code,
            message: message.into(),
            recoverable: false,
        }
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
            StorageError::Sqlite(_)
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
        match error {
            CoreError::Storage(storage) => storage.into(),
            CoreError::Open(open) => open.into(),
            CoreError::NotFound { .. } => Self::user(ErrorCode::NotFound, error.to_string(), false),
            CoreError::UnknownDestination { .. } => {
                Self::user(ErrorCode::UnknownDestination, error.to_string(), true)
            }
            CoreError::Fetch(_) => Self::user(ErrorCode::Network, error.to_string(), true),
            CoreError::InvalidUrl { .. } | CoreError::ReservedSettingKey { .. } => {
                Self::user(ErrorCode::InvalidInput, error.to_string(), true)
            }
            CoreError::RateLimited { .. } | CoreError::RedditAuth { .. } => {
                Self::user(ErrorCode::Network, error.to_string(), true)
            }
            CoreError::FeedParse(_) | CoreError::Opml(_) | CoreError::Import(_) => {
                Self::user(ErrorCode::Parse, error.to_string(), true)
            }
            // A vanished vault (moved, renamed, unmounted), or a note whose
            // every candidate path is taken by other files, is the user's to
            // fix — reconnect the folder or register the destination again;
            // rename or move the files on the named path — so its message is
            // shown, not a generic failure.
            CoreError::Export(
                ExportError::DestinationMissing { .. } | ExportError::PathsTaken { .. },
            ) => Self::user(ErrorCode::Io, error.to_string(), true),
            CoreError::Io { .. } | CoreError::Export(_) | CoreError::Events(_) => {
                Self::internal(ErrorCode::Io, error.to_string())
            }
            CoreError::Content(_) | CoreError::Settings(_) => {
                Self::internal(ErrorCode::Internal, error.to_string())
            }
        }
    }
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
        assert_eq!(error.code, ErrorCode::Io);
        assert!(error.recoverable);
        assert!(error.message.contains("missing"), "{}", error.message);
        let taken = CommandError::from(CoreError::Export(ExportError::PathsTaken {
            path: "curio/busy-0197b2c4.md".into(),
        }));
        assert_eq!(taken.kind, ErrorKind::User);
        assert!(taken.recoverable);
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
    }
}
