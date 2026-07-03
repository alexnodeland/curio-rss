//! The Curio reading engine.
//!
//! `curio-core` owns fetch, ingest, store, article state, vault export, and
//! the behavioral event log. Every user-facing surface (CLI, desktop, a
//! future server) is a thin head over the same service object.
//!
//! Boundary rule (mechanically enforced by `deny.toml` and
//! `cargo run -p xtask -- boundary`): this crate must never depend on tauri
//! or any webview — it builds and tests headless, everywhere.
//!
//! Shipped so far: [`model`] (domain types), [`storage`] (WAL `SQLite`
//! behind a single-writer/pooled-reader facade), and [`events`] (the
//! `curio.events.v1` JSONL log with crash-recoverable emission). The
//! remaining engine modules (`fetch/`, `ingest/`, `export/`, `enrich/`)
//! land per `docs/design/roadmap.md`.

pub mod events;
pub mod fetch;
pub mod model;
pub mod storage;

pub use curio_types as types;

/// The engine service object. Heads hold an `Arc<CurioCore>` and delegate;
/// they contain no business logic of their own.
#[derive(Debug, Default)]
pub struct CurioCore {
    _private: (),
}

impl CurioCore {
    /// Create a new engine handle.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The engine crate version.
    #[must_use]
    pub fn version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructs_headless() {
        let _core = CurioCore::new();
    }

    #[test]
    fn reports_the_workspace_version() {
        assert_eq!(CurioCore::version(), env!("CARGO_PKG_VERSION"));
    }
}
