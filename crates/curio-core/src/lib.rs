//! The Curio reading engine.
//!
//! `curio-core` owns fetch, ingest, store, article state, vault export,
//! and the behavioral event log. Every user-facing surface (CLI, desktop,
//! a future server) is a thin head over the same [`CoreHandle`] service
//! object — heads contain no business logic.
//!
//! Boundary rule (mechanically enforced by `deny.toml` and
//! `cargo run -p xtask -- boundary`): this crate must never depend on tauri
//! or any webview — it builds and tests headless, everywhere.
//!
//! The engine, module by module: [`fetch`] (the policed client — the only
//! outbound network path), [`feeds`] (RSS/Atom/JSON-Feed parsing, dedupe,
//! OPML), [`content`] (sanitize-at-ingest → readability → markdown),
//! [`storage`] (WAL `SQLite` behind a single-writer/pooled-reader facade),
//! [`events`] (the `curio.events.v1` JSONL log with crash-recoverable
//! emission), [`export`] (the `curio.frontmatter.v1` note writer), and
//! [`handle`] (the [`CoreHandle`] facade wiring them together).

pub mod content;
pub mod events;
pub mod export;
pub mod feeds;
pub mod fetch;
mod handle;
pub mod model;
pub mod storage;

pub use curio_types as types;
pub use handle::{
    CoreError, CoreHandle, CoreOptions, OpmlImportOutcome, RefreshOutcome, SaveOutcome,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_workspace_version() {
        assert_eq!(CoreHandle::version(), env!("CARGO_PKG_VERSION"));
    }
}
