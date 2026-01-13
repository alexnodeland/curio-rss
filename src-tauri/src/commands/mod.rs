//! Tauri command handlers.
//!
//! This module provides the IPC interface between the frontend and backend.
//! Commands are thin wrappers that delegate to core business logic.

mod feeds;
mod articles;
mod folders;
mod reddit;
mod youtube;
mod export;

pub use feeds::*;
pub use articles::*;
pub use folders::*;
pub use reddit::*;
pub use youtube::*;
pub use export::*;

use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::core::models::Settings;
use crate::storage::Database;

/// Application state shared across commands
pub struct AppState {
    pub db: Database,
    pub settings: Arc<RwLock<Settings>>,
    pub config_dir: PathBuf,
}

impl AppState {
    /// Create new application state
    pub fn new(config_dir: PathBuf) -> Result<Self, crate::error::InfraError> {
        let db_path = config_dir.join("curio.db");

        // Ensure config directory exists
        std::fs::create_dir_all(&config_dir)?;

        let db = Database::open(&db_path)?;
        let settings = Settings::default();

        Ok(Self {
            db,
            settings: Arc::new(RwLock::new(settings)),
            config_dir,
        })
    }
}
