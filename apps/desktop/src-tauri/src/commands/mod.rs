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
pub mod recovery;
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

pub(crate) use crate::error::logged;
use crate::error::{CommandError, ErrorCode};

/// The shared core handle, managed as Tauri state.
pub type SharedCore = Arc<CoreHandle>;

/// Runs command `command`'s synchronous core call off the async runtime
/// (facade mutations block briefly on the writer thread; never stall the
/// IPC reactor), logging an internal-tier failure once through
/// [`logged`]. The name is the command's own — a test holds every command
/// to naming itself exactly once, here or in a direct [`logged`] call.
pub(crate) async fn run_blocking<T, F>(command: &'static str, job: F) -> Result<T, CommandError>
where
    F: FnOnce() -> Result<T, CommandError> + Send + 'static,
    T: Send + 'static,
{
    logged(command, blocking(job).await)
}

/// [`run_blocking`] without the log: for helpers shared with callers that
/// are not commands (the background scheduler logs its own failures).
pub(crate) async fn blocking<T, F>(job: F) -> Result<T, CommandError>
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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::sync::{Arc, Mutex};

    use tracing::field::{Field, Visit};
    use tracing_subscriber::layer::{Context, SubscriberExt as _};
    use tracing_subscriber::{Layer, Registry};

    use super::*;
    use crate::error::ErrorKind;

    /// One captured log record: its level and its fields.
    #[derive(Debug, Clone)]
    struct Record {
        level: tracing::Level,
        fields: Vec<(String, String)>,
    }

    /// Collects every event emitted while it is the default subscriber.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<Record>>>);

    impl Capture {
        fn errors(&self) -> Vec<Record> {
            let records = self.0.lock().unwrap();
            records
                .iter()
                .filter(|record| record.level == tracing::Level::ERROR)
                .cloned()
                .collect()
        }
    }

    struct Fields<'a>(&'a mut Vec<(String, String)>);

    impl Visit for Fields<'_> {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.0.push((field.name().to_owned(), format!("{value:?}")));
        }
        fn record_str(&mut self, field: &Field, value: &str) {
            self.0.push((field.name().to_owned(), value.to_owned()));
        }
    }

    impl<S: tracing::Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &tracing::Event<'_>, _: Context<'_, S>) {
            let mut fields = Vec::new();
            event.record(&mut Fields(&mut fields));
            self.0.lock().unwrap().push(Record {
                level: *event.metadata().level(),
                fields,
            });
        }
    }

    fn field<'a>(record: &'a Record, name: &str) -> Option<&'a str> {
        record
            .fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// Runs `body` with a capturing subscriber as this thread's default.
    /// The tests run on a current-thread runtime, so the command's
    /// continuation after the blocking job — where it logs — is on this
    /// thread too.
    async fn capturing<F: std::future::Future<Output = ()>>(body: F) -> Capture {
        let capture = Capture::default();
        let subscriber = Registry::default().with(capture.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        body.await;
        capture
    }

    #[tokio::test]
    async fn an_internal_error_in_a_command_is_logged_exactly_once_with_its_name() {
        let capture = capturing(async {
            let error = run_blocking("vacuum_db", || -> Result<(), CommandError> {
                Err(CommandError::internal(
                    ErrorCode::Storage,
                    "sqlite error: disk I/O error",
                ))
            })
            .await
            .unwrap_err();
            assert_eq!(error.kind, ErrorKind::Internal);
        })
        .await;
        let errors = capture.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(field(&errors[0], "command"), Some("vacuum_db"));
        assert_eq!(field(&errors[0], "code"), Some("Storage"));
        assert!(
            field(&errors[0], "message").is_some_and(|m| m.contains("disk I/O error")),
            "{errors:?}"
        );
    }

    #[tokio::test]
    async fn a_panicked_command_job_is_logged_once_as_internal() {
        let capture = capturing(async {
            let error = run_blocking("integrity_check", || -> Result<(), CommandError> {
                panic!("a bug in the job")
            })
            .await
            .unwrap_err();
            assert_eq!(
                (error.kind, error.code),
                (ErrorKind::Internal, ErrorCode::Internal)
            );
        })
        .await;
        let errors = capture.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(field(&errors[0], "command"), Some("integrity_check"));
    }

    #[tokio::test]
    async fn a_real_command_failure_is_logged_once() {
        // A destination registry the facade cannot parse is an internal
        // failure of the real `list_destinations` core call.
        let (_dir, core) = testutil::temp_core();
        core.storage()
            .set_setting("destinations", "{not json")
            .unwrap();
        let capture = capturing(async {
            let error = run_blocking("list_destinations", move || {
                core.destinations().map_err(CommandError::from)
            })
            .await
            .unwrap_err();
            assert_eq!(error.kind, ErrorKind::Internal, "{error:?}");
        })
        .await;
        let errors = capture.errors();
        assert_eq!(errors.len(), 1, "{errors:?}");
        assert_eq!(field(&errors[0], "command"), Some("list_destinations"));
    }

    #[tokio::test]
    async fn user_errors_and_successes_stay_out_of_the_log() {
        let capture = capturing(async {
            let user = run_blocking("add_feed", || -> Result<(), CommandError> {
                Err(CommandError::invalid_input("not a URL"))
            })
            .await;
            assert!(user.is_err());
            assert_eq!(run_blocking("list_feeds", || Ok(3)).await.unwrap(), 3);
            // The unlogged primitive stays silent even for internal errors:
            // its callers (the scheduler) log their own.
            let quiet = blocking(|| -> Result<(), CommandError> {
                Err(CommandError::internal(ErrorCode::Internal, "x"))
            })
            .await;
            assert!(quiet.is_err());
        })
        .await;
        assert!(capture.errors().is_empty(), "{:?}", capture.errors());
    }

    /// Every `#[tauri::command]` names itself exactly once, in the call
    /// that logs its internal failures — so none is silent, none logs
    /// twice, and none logs under another command's name.
    #[test]
    fn every_command_routes_its_result_through_the_log_under_its_own_name() {
        let root =
            std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
                .join("src");
        let mut files: Vec<_> = std::fs::read_dir(root.join("commands"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.push(root.join("notify.rs"));
        let mut checked = 0;
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let mut lines = source.lines();
            while let Some(line) = lines.next() {
                if line.trim() != "#[tauri::command]" {
                    continue;
                }
                let signature = lines
                    .by_ref()
                    .find(|line| line.starts_with("pub async fn "))
                    .unwrap();
                let name = signature["pub async fn ".len()..]
                    .split('(')
                    .next()
                    .unwrap();
                // The rest of a wrapped signature, up to its opening brace.
                if !signature.ends_with('{') {
                    lines.by_ref().find(|line| line.ends_with('{')).unwrap();
                }
                let body: Vec<&str> = lines.by_ref().take_while(|line| *line != "}").collect();
                // Collapse whitespace so a call rustfmt wrapped still matches.
                let body = body
                    .join(" ")
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                checked += 1;
                // An infallible body — one that ends in `Ok(…)` and has no
                // `?` or `Err` on the way (`Ok(state.clone())`,
                // `pending.stage(id); Ok(())`) — has nothing to log.
                let tail = body.rsplit("; ").next().unwrap_or_default();
                if tail.starts_with("Ok(") && !body.contains('?') && !body.contains("Err") {
                    continue;
                }
                let literal = format!("\"{name}\"");
                assert_eq!(
                    body.matches(&literal).count(),
                    1,
                    "{}: `{name}` must name itself exactly once (run_blocking / logged)",
                    file.display()
                );
                assert!(
                    ["run_blocking(", "logged(", "with_gate(", "run_recovery("]
                        .iter()
                        .any(|call| body.contains(&format!("{call}{literal}"))
                            || body.contains(&format!("{call} {literal}"))),
                    "{}: `{name}` passes its name to something other than the logger",
                    file.display()
                );
            }
        }
        assert!(checked >= 70, "only {checked} commands found");
    }
}
