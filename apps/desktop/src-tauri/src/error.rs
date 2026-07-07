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
//! / `InvalidTag` / `SchemaTooNew` are user-tier; `Sqlite` / `WriterGone` /
//! `IntegrityFailed` / IO are internal-tier.

use curio_core::CoreError;
use curio_core::storage::StorageError;
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
    /// The storage layer failed.
    Storage,
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
            StorageError::SchemaTooNew { .. } => {
                Self::user(ErrorCode::SchemaTooNew, error.to_string(), false)
            }
            StorageError::Sqlite(_)
            | StorageError::WriterGone
            | StorageError::WalUnavailable { .. }
            | StorageError::Corrupt { .. }
            | StorageError::IntegrityFailed { .. }
            | StorageError::Envelope(_) => Self::internal(ErrorCode::Storage, error.to_string()),
        }
    }
}

impl From<CoreError> for CommandError {
    fn from(error: CoreError) -> Self {
        match error {
            CoreError::Storage(storage) => storage.into(),
            CoreError::NotFound { .. } => Self::user(ErrorCode::NotFound, error.to_string(), false),
            CoreError::UnknownDestination { .. } => {
                Self::user(ErrorCode::UnknownDestination, error.to_string(), true)
            }
            CoreError::Fetch(_) => Self::user(ErrorCode::Network, error.to_string(), true),
            CoreError::FeedParse(_) | CoreError::Opml(_) | CoreError::Import(_) => {
                Self::user(ErrorCode::Parse, error.to_string(), true)
            }
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
    fn writer_gone_is_internal_storage() {
        let error = CommandError::from(StorageError::WriterGone);
        assert_eq!(error.kind, ErrorKind::Internal);
        assert_eq!(error.code, ErrorCode::Storage);
        assert!(!error.recoverable);
    }

    #[test]
    fn schema_too_new_is_a_user_facing_stop() {
        let error = CommandError::from(StorageError::SchemaTooNew {
            found: 9,
            supported: 1,
        });
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::SchemaTooNew);
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
