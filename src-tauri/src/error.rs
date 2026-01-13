//! Unified error types for Curio Reader.
//!
//! This module provides a hierarchical error system with:
//! - `CoreError`: Business logic failures
//! - `InfraError`: I/O and external service failures
//! - `CommandError`: User-facing, serializable errors

use serde::Serialize;
use std::time::Duration;
use uuid::Uuid;

/// Core errors - business logic failures
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Feed not found: {0}")]
    FeedNotFound(Uuid),

    #[error("Article not found: {0}")]
    ArticleNotFound(Uuid),

    #[error("Folder not found: {0}")]
    FolderNotFound(Uuid),

    #[error("Profile not found: {0}")]
    ProfileNotFound(String),

    #[error("Invalid feed URL: {0}")]
    InvalidFeedUrl(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Validation error: {0}")]
    ValidationError(String),

    #[error("Duplicate feed URL: {0}")]
    DuplicateFeed(String),
}

/// Infrastructure errors - I/O and external failures
#[derive(Debug, thiserror::Error)]
pub enum InfraError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("yt-dlp error: {0}")]
    YtDlp(String),

    #[error("File I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },

    #[error("Feed parse error: {0}")]
    FeedParse(String),
}

/// Error codes for frontend consumption
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    NetworkError,
    RateLimited,
    ParseError,
    StorageError,
    ExternalServiceError,
    DuplicateEntry,
}

/// Command errors - user-facing, serializable
#[derive(Debug, Serialize, thiserror::Error)]
pub enum CommandError {
    #[error("{message}")]
    User {
        message: String,
        code: ErrorCode,
        recoverable: bool,
    },

    #[error("Internal error: {message}")]
    Internal {
        message: String,
        #[serde(skip)]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

impl CommandError {
    /// Create a validation error
    pub fn validation(message: impl Into<String>) -> Self {
        Self::User {
            message: message.into(),
            code: ErrorCode::InvalidInput,
            recoverable: true,
        }
    }

    /// Create a not found error
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::User {
            message: message.into(),
            code: ErrorCode::NotFound,
            recoverable: false,
        }
    }

    /// Create a storage error
    pub fn storage(message: impl Into<String>) -> Self {
        Self::User {
            message: message.into(),
            code: ErrorCode::StorageError,
            recoverable: false,
        }
    }

    /// Create an internal error from any error type
    pub fn internal(source: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Internal {
            message: source.to_string(),
            source: Some(Box::new(source)),
        }
    }
}

impl From<CoreError> for CommandError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::FeedNotFound(id) => Self::User {
                message: format!("Feed not found: {}", id),
                code: ErrorCode::NotFound,
                recoverable: false,
            },
            CoreError::ArticleNotFound(id) => Self::User {
                message: format!("Article not found: {}", id),
                code: ErrorCode::NotFound,
                recoverable: false,
            },
            CoreError::FolderNotFound(id) => Self::User {
                message: format!("Folder not found: {}", id),
                code: ErrorCode::NotFound,
                recoverable: false,
            },
            CoreError::ProfileNotFound(name) => Self::User {
                message: format!("Profile not found: {}", name),
                code: ErrorCode::NotFound,
                recoverable: false,
            },
            CoreError::InvalidFeedUrl(url) => Self::User {
                message: format!("Invalid feed URL: {}", url),
                code: ErrorCode::InvalidInput,
                recoverable: true,
            },
            CoreError::ParseError(msg) => Self::User {
                message: format!("Parse error: {}", msg),
                code: ErrorCode::ParseError,
                recoverable: false,
            },
            CoreError::ValidationError(msg) => Self::User {
                message: msg,
                code: ErrorCode::InvalidInput,
                recoverable: true,
            },
            CoreError::DuplicateFeed(url) => Self::User {
                message: format!("Feed already exists: {}", url),
                code: ErrorCode::DuplicateEntry,
                recoverable: true,
            },
        }
    }
}

impl From<InfraError> for CommandError {
    fn from(e: InfraError) -> Self {
        match e {
            InfraError::Database(err) => Self::User {
                message: format!("Database error: {}", err),
                code: ErrorCode::StorageError,
                recoverable: false,
            },
            InfraError::Http(err) => Self::User {
                message: format!("Network error: {}", err),
                code: ErrorCode::NetworkError,
                recoverable: true,
            },
            InfraError::YtDlp(msg) => Self::User {
                message: format!("YouTube extraction error: {}", msg),
                code: ErrorCode::ExternalServiceError,
                recoverable: true,
            },
            InfraError::Io(err) => Self::User {
                message: format!("File error: {}", err),
                code: ErrorCode::StorageError,
                recoverable: false,
            },
            InfraError::Json(err) => Self::User {
                message: format!("JSON error: {}", err),
                code: ErrorCode::ParseError,
                recoverable: false,
            },
            InfraError::RateLimited { retry_after } => Self::User {
                message: format!(
                    "Rate limited. {}",
                    retry_after
                        .map(|d| format!("Retry after {} seconds", d.as_secs()))
                        .unwrap_or_default()
                ),
                code: ErrorCode::RateLimited,
                recoverable: true,
            },
            InfraError::FeedParse(msg) => Self::User {
                message: format!("Failed to parse feed: {}", msg),
                code: ErrorCode::ParseError,
                recoverable: false,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_core_error_to_command_error_feed_not_found() {
        let id = Uuid::new_v4();
        let core_err = CoreError::FeedNotFound(id);
        let cmd_err: CommandError = core_err.into();

        match cmd_err {
            CommandError::User {
                message,
                code,
                recoverable,
            } => {
                assert!(message.contains(&id.to_string()));
                assert_eq!(code, ErrorCode::NotFound);
                assert!(!recoverable);
            }
            _ => panic!("Expected User error"),
        }
    }

    #[test]
    fn test_core_error_to_command_error_validation() {
        let core_err = CoreError::ValidationError("Invalid input".to_string());
        let cmd_err: CommandError = core_err.into();

        match cmd_err {
            CommandError::User {
                message,
                code,
                recoverable,
            } => {
                assert_eq!(message, "Invalid input");
                assert_eq!(code, ErrorCode::InvalidInput);
                assert!(recoverable);
            }
            _ => panic!("Expected User error"),
        }
    }

    #[test]
    fn test_command_error_validation_helper() {
        let err = CommandError::validation("test message");

        match err {
            CommandError::User {
                message,
                code,
                recoverable,
            } => {
                assert_eq!(message, "test message");
                assert_eq!(code, ErrorCode::InvalidInput);
                assert!(recoverable);
            }
            _ => panic!("Expected User error"),
        }
    }

    #[test]
    fn test_command_error_not_found_helper() {
        let err = CommandError::not_found("item not found");

        match err {
            CommandError::User {
                message,
                code,
                recoverable,
            } => {
                assert_eq!(message, "item not found");
                assert_eq!(code, ErrorCode::NotFound);
                assert!(!recoverable);
            }
            _ => panic!("Expected User error"),
        }
    }

    #[test]
    fn test_command_error_serialization() {
        let err = CommandError::validation("test");
        let json = serde_json::to_string(&err).unwrap();

        assert!(json.contains("INVALID_INPUT"));
        assert!(json.contains("test"));
        assert!(json.contains("recoverable"));
    }

    #[test]
    fn test_infra_error_rate_limited() {
        let err = InfraError::RateLimited {
            retry_after: Some(Duration::from_secs(60)),
        };
        let cmd_err: CommandError = err.into();

        match cmd_err {
            CommandError::User {
                message,
                code,
                recoverable,
            } => {
                assert!(message.contains("60"));
                assert_eq!(code, ErrorCode::RateLimited);
                assert!(recoverable);
            }
            _ => panic!("Expected User error"),
        }
    }
}
