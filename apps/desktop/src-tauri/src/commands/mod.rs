//! The IPC command surface: ~45 thin `#[tauri::command]` wrappers over
//! `Arc<CoreHandle>` with zero business logic.
//!
//! Shape rules (build spec §1):
//! - Every command is `async fn` returning `Result<T, CommandError>`.
//! - Sync core calls (all facade mutations block on a writer-thread round
//!   trip) run inside the crate-private `run_blocking` helper below;
//!   only `refresh_*` is async-native.
//! - Each command delegates to a plain `*_impl` function taking
//!   `&CoreHandle` — that is the unit-tested layer (temp-profile core, no
//!   webview, no app handle).
//! - Mutating commands emit invalidation events **after** success, gated
//!   on core's `changed` bool where one exists (see [`crate::events`]).

// Every command fails the same one way — `CommandError`, whose tiers and
// mapping are documented once in `crate::error` — so per-command
// `# Errors` sections would be 45 copies of the same sentence.
#[allow(clippy::missing_errors_doc)]
pub mod articles;
#[allow(clippy::missing_errors_doc)]
pub mod destinations;
#[allow(clippy::missing_errors_doc)]
pub mod feeds;
#[allow(clippy::missing_errors_doc)]
pub mod health;
#[allow(clippy::missing_errors_doc)]
pub mod opml;
#[allow(clippy::missing_errors_doc)]
pub mod platform;
#[allow(clippy::missing_errors_doc)]
pub mod reddit;
#[allow(clippy::missing_errors_doc)]
pub mod settings;
#[allow(clippy::missing_errors_doc)]
pub mod state;
#[allow(clippy::missing_errors_doc)]
pub mod tags;
#[allow(clippy::missing_errors_doc)]
pub mod theme;

use std::sync::Arc;

use curio_core::CoreHandle;

use crate::error::{CommandError, ErrorCode};

/// The shared core handle, managed as Tauri state.
pub type SharedCore = Arc<CoreHandle>;

/// Runs a synchronous core call off the async runtime (facade mutations
/// block briefly on the writer thread; never stall the IPC reactor).
pub(crate) async fn run_blocking<T, F>(job: F) -> Result<T, CommandError>
where
    F: FnOnce() -> Result<T, CommandError> + Send + 'static,
    T: Send + 'static,
{
    tauri::async_runtime::spawn_blocking(job)
        .await
        .map_err(|error| {
            CommandError::internal(ErrorCode::Internal, format!("blocking task join: {error}"))
        })?
}

#[cfg(test)]
pub(crate) mod testutil {
    #![allow(clippy::unwrap_used)]

    use std::sync::Arc;

    use curio_core::model::NewArticle;
    use curio_core::{CoreHandle, CoreOptions};
    use curio_types::Timestamp;

    /// A core handle over a throwaway profile directory. Keep the
    /// `TempDir` alive for the duration of the test.
    pub(crate) fn temp_core() -> (tempfile::TempDir, Arc<CoreHandle>) {
        let dir = tempfile::tempdir().unwrap();
        let core = CoreHandle::open(dir.path().join("profile")).unwrap();
        (dir, Arc::new(core))
    }

    /// Like [`temp_core`] but with 127.0.0.1 exempted from the SSRF
    /// guard and politeness disabled, for hermetic wiremock fetches.
    pub(crate) fn temp_core_trusting_localhost() -> (tempfile::TempDir, Arc<CoreHandle>) {
        use std::collections::HashSet;
        use std::net::{IpAddr, Ipv4Addr};

        let dir = tempfile::tempdir().unwrap();
        let mut options = CoreOptions::default();
        options.fetch.politeness_delay = std::time::Duration::ZERO;
        options.fetch.trusted_addrs = HashSet::from([IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        let core = CoreHandle::open_with(dir.path().join("profile"), options).unwrap();
        (dir, Arc::new(core))
    }

    /// Stores one minimal article directly (no network), returning its
    /// row id.
    pub(crate) fn seed_article(core: &CoreHandle, title: &str) -> i64 {
        let outcome = core
            .storage()
            .upsert_articles(vec![NewArticle {
                feed_id: None,
                dedupe_key: format!("test:{title}"),
                title: title.to_owned(),
                source_url: format!("https://example.test/{title}"),
                author: None,
                published_at: None,
                content: curio_core::model::ArticleContent {
                    html: format!("<p>{title}</p>"),
                    text: title.to_owned(),
                },
                lang: None,
                word_count: Some(1),
                source_updated_at: Some(Timestamp::now()),
                lead_image: None,
            }])
            .unwrap();
        assert_eq!(outcome.inserted, 1);
        // Newest row id — the article we just inserted.
        let listed = core
            .list_articles(curio_core::storage::ListArticles::default())
            .unwrap();
        listed.first().unwrap().id.0
    }
}
