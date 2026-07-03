//! Curio Reader - A fast, themeable RSS reader.
//!
//! This crate provides the Rust backend for Curio Reader, including:
//! - Feed parsing (RSS/Atom)
//! - OPML import/export
//! - SQLite storage
//! - Content sanitization and Markdown export
//! - Platform integrations (YouTube, Reddit)

pub mod commands;
pub mod core;
pub mod error;
pub mod platform;
pub mod services;
pub mod storage;

// Re-export commonly used types
pub use error::{CommandError, CoreError, InfraError};
