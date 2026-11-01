//! Feed commands: subscribe, remove, list, status, metadata, refresh,
//! and the feed-health fetch log.

use std::sync::Arc;

use curio_core::feeds::{DiscoverOptions, normalize_input};
use curio_core::model::{FeedId, FeedStatus, NewFeed};
use curio_core::{CoreHandle, RefreshOptions};
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::discovery::DiscoveryDto;
use crate::dto::{FeedDto, FeedStatusDto, FetchRecordDto, NewFeedDto, RefreshOutcomeDto};
use crate::error::{CommandError, ErrorCode};
use crate::events::{
    ArticlesChanged, ConnectivityChanged, FeedsChanged, RefreshFinished, RefreshProgress,
    emit_or_log,
};
use crate::refresh_scheduler::RefreshScheduler;

/// Autodiscovers feeds (and a favicon) for what the user typed — a bare
/// domain, a site, a page, or a feed URL. The core's discovery normalizes
/// it, applies the fetch-free rewrites, scans and probes through the
/// policed client and offers only validated feeds; the Google favicon
/// fallback is the frontend's opt-in.
#[tauri::command]
#[specta::specta]
pub async fn discover_feeds(
    core: State<'_, SharedCore>,
    url: String,
) -> Result<DiscoveryDto, CommandError> {
    let core = Arc::clone(core.inner());
    crate::discovery::discover(&core, &url).await
}

/// Subscribes to a feed and kicks off its first fetch in the background —
/// a new subscription must not sit empty until the next refresh cycle.
/// By default the typed URL is resolved through the core's validating
/// discovery first, so a homepage or a bare domain subscribes to its feed
/// and a page with none is refused (`NO_FEED`) instead of becoming a
/// subscription that fails on every refresh; `validate: false` is the
/// explicit "subscribe anyway" (normalized only). A URL already subscribed
/// — including one a homepage resolved to — is `ALREADY_SUBSCRIBED`.
// The arg is `new_feed`, not `new` — `new` is a reserved word in the
// generated TypeScript.
#[tauri::command]
#[specta::specta]
pub async fn add_feed(
    app: AppHandle,
    core: State<'_, SharedCore>,
    new_feed: NewFeedDto,
) -> Result<FeedDto, CommandError> {
    let core = Arc::clone(core.inner());
    let feed = subscribe_impl(&core, new_feed).await?;
    emit_or_log(&app, &FeedsChanged);
    // Fire-and-forget so the add returns as soon as the row exists; a failed
    // first fetch surfaces through feed health, like any scheduled refresh.
    let feed_id = feed.id;
    tauri::async_runtime::spawn(async move {
        if refresh_feed_impl(&core, feed_id).await.is_ok() {
            emit_or_log(&app, &FeedsChanged);
            emit_or_log(
                &app,
                &ArticlesChanged {
                    feed_id: Some(feed_id),
                },
            );
        }
    });
    Ok(feed)
}

/// Unsubscribes. Stored articles survive (they lose their `feed_id`).
#[tauri::command]
#[specta::specta]
pub async fn remove_feed(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || remove_feed_impl(&core, feed_id)).await?;
    emit_or_log(&app, &FeedsChanged);
    emit_or_log(&app, &ArticlesChanged { feed_id: None });
    Ok(())
}

/// Every subscription.
#[tauri::command]
#[specta::specta]
pub async fn list_feeds(core: State<'_, SharedCore>) -> Result<Vec<FeedDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || list_feeds_impl(&core)).await
}

/// One subscription, if it exists.
#[tauri::command]
#[specta::specta]
pub async fn get_feed(
    core: State<'_, SharedCore>,
    feed_id: i64,
) -> Result<Option<FeedDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_feed_impl(&core, feed_id)).await
}

/// Lookup by subscription URL — the add-feed dedupe check.
#[tauri::command]
#[specta::specta]
pub async fn get_feed_by_url(
    core: State<'_, SharedCore>,
    url: String,
) -> Result<Option<FeedDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_feed_by_url_impl(&core, &url)).await
}

/// Pause / resume / un-dead a feed.
#[tauri::command]
#[specta::specta]
pub async fn set_feed_status(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
    status: FeedStatusDto,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_feed_status_impl(&core, feed_id, status)).await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Edit title / site URL / description. Core never clobbers already-set
/// values with `None` — human edits win.
#[tauri::command]
#[specta::specta]
pub async fn update_feed_metadata(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
    title: Option<String>,
    site_url: Option<String>,
    description: Option<String>,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || update_feed_metadata_impl(&core, feed_id, title, site_url, description))
        .await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Replaces a feed's tags (move-to-folder / re-tag). Tags are `/`-path
/// strings; the sidebar renders them as a folder tree. Wholesale overwrite,
/// DB-local (no event).
#[tauri::command]
#[specta::specta]
pub async fn set_feed_tags(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
    tags: Vec<String>,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_feed_tags_impl(&core, feed_id, tags)).await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Renames a feed (unconditional title overwrite; an empty title clears it,
/// falling back to the URL). DB-local, no event.
#[tauri::command]
#[specta::specta]
pub async fn set_feed_title(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
    title: Option<String>,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_feed_title_impl(&core, feed_id, title)).await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Overwrites a feed's site URL and description (edit-feed modal). Unlike
/// `update_feed_metadata` — the fetch-fill path, which COALESCE-fills a NULL
/// so a refresh never clobbers a human edit — this is an unconditional user
/// overwrite; an empty value clears the field.
#[tauri::command]
#[specta::specta]
pub async fn set_feed_metadata(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
    site_url: Option<String>,
    description: Option<String>,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_feed_metadata_impl(&core, feed_id, site_url, description)).await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Flips a feed's full-text mode: when on, every refresh hydrates the
/// feed's new articles from their source pages (readability-extracted
/// through the policed client) — for feeds that ship only excerpts.
#[tauri::command]
#[specta::specta]
pub async fn set_feed_full_text(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
    enabled: bool,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_feed_full_text_impl(&core, feed_id, enabled)).await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Rewrites the sidebar feed order (drag-to-reorder): `feed_ids` is the
/// complete new sequence. DB-local, no event.
#[tauri::command]
#[specta::specta]
pub async fn reorder_feeds(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_ids: Vec<i64>,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || reorder_feeds_impl(&core, &feed_ids)).await?;
    emit_or_log(&app, &FeedsChanged);
    Ok(())
}

/// Refreshes one feed. Fetch/parse failures are *outcomes*, not errors;
/// same-feed refreshes are serialized core-side (validator-race fix).
#[tauri::command]
#[specta::specta]
pub async fn refresh_feed(
    app: AppHandle,
    core: State<'_, SharedCore>,
    feed_id: i64,
) -> Result<RefreshOutcomeDto, CommandError> {
    let core = Arc::clone(core.inner());
    let outcome = refresh_feed_impl(&core, feed_id).await?;
    emit_or_log(&app, &FeedsChanged);
    emit_or_log(
        &app,
        &ArticlesChanged {
            feed_id: Some(feed_id),
        },
    );
    Ok(outcome)
}

/// Refreshes every `active` feed as one core sweep (bounded concurrency),
/// emitting [`RefreshProgress`] per feed and [`RefreshFinished`] at the
/// end. Pressing refresh while a sweep runs joins it — no feed is fetched
/// twice. Any feed answering clears the scheduler's "Offline" verdict right
/// away; a manual refresh resets the background scheduler's interval clock,
/// except one made while offline that reached nothing (the overdue sweep
/// stays due).
#[tauri::command]
#[specta::specta]
pub async fn refresh_all(
    app: AppHandle,
    core: State<'_, SharedCore>,
    scheduler: State<'_, Arc<RefreshScheduler>>,
) -> Result<Vec<RefreshOutcomeDto>, CommandError> {
    let core = Arc::clone(core.inner());
    let sweep = refresh_all_and_emit(&app, &core).await?;
    scheduler
        .record_manual_refresh(&core, &sweep.finished.outcomes, |online| {
            emit_or_log(&app, &ConnectivityChanged { online });
        })
        .await;
    Ok(sweep.finished.outcomes)
}

/// Cancels the refresh sweep in flight: no further feed starts and the
/// fetches in flight are dropped, so the sweep finishes early
/// ([`RefreshFinished::cancelled`]). `false` when no sweep is running.
#[tauri::command]
#[specta::specta]
pub async fn cancel_refresh(core: State<'_, SharedCore>) -> Result<bool, CommandError> {
    Ok(core.cancel_refresh())
}

/// One sweep as the head reports it.
pub(crate) struct Sweep {
    /// This request joined a sweep already in flight, whose own request
    /// emits the events.
    pub joined: bool,
    /// The finish event (also the command's result).
    pub finished: RefreshFinished,
}

/// Refreshes every active feed and emits the progress/finished/data-changed
/// events — the shared body of the manual `refresh_all` command and the
/// background scheduler, so both drive the exact same UI updates. A request
/// that joined a running sweep emits nothing: that sweep's own request does.
pub(crate) async fn refresh_all_and_emit(
    app: &AppHandle,
    core: &CoreHandle,
) -> Result<Sweep, CommandError> {
    let sweep = refresh_all_impl(core, |progress| emit_or_log(app, &progress)).await?;
    if !sweep.joined {
        emit_or_log(app, &sweep.finished);
        emit_or_log(app, &FeedsChanged);
        emit_or_log(app, &ArticlesChanged { feed_id: None });
    }
    Ok(sweep)
}

/// The recent fetch log for one feed, newest first — the feed-health
/// surface.
#[tauri::command]
#[specta::specta]
pub async fn recent_fetches(
    core: State<'_, SharedCore>,
    feed_id: i64,
    limit: u32,
) -> Result<Vec<FetchRecordDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || recent_fetches_impl(&core, feed_id, limit)).await
}

// ------------------------------------------------------------------ impls

/// The subscribe path: resolve (validate) or merely normalize what the
/// user typed, then add it. Discovery runs on the async runtime (it is
/// network-bound); only the storage write is blocking.
async fn subscribe_impl(core: &Arc<CoreHandle>, new: NewFeedDto) -> Result<FeedDto, CommandError> {
    let url = if new.validate {
        core.resolve_feed_url(&new.url, DiscoverOptions::default())
            .await?
            .url
    } else {
        normalize_input(&new.url)?.into()
    };
    let core = Arc::clone(core);
    run_blocking(move || {
        add_feed_impl(
            &core,
            NewFeed {
                url,
                title: new.title,
                tags: new.tags,
            },
        )
    })
    .await
}

/// Adds an already-resolved feed URL, refusing one already subscribed with
/// a code the UI can branch on (the UNIQUE violation alone would read as
/// an internal storage failure).
fn add_feed_impl(core: &CoreHandle, new: NewFeed) -> Result<FeedDto, CommandError> {
    if core.get_feed_by_url(&new.url)?.is_some() {
        return Err(CommandError::user(
            ErrorCode::AlreadySubscribed,
            format!("already subscribed to {}", new.url),
            false,
        ));
    }
    Ok(core.add_feed(new)?.into())
}

fn remove_feed_impl(core: &CoreHandle, feed_id: i64) -> Result<(), CommandError> {
    Ok(core.remove_feed(FeedId(feed_id))?)
}

fn list_feeds_impl(core: &CoreHandle) -> Result<Vec<FeedDto>, CommandError> {
    Ok(core.list_feeds()?.into_iter().map(FeedDto::from).collect())
}

fn get_feed_impl(core: &CoreHandle, feed_id: i64) -> Result<Option<FeedDto>, CommandError> {
    Ok(core.get_feed(FeedId(feed_id))?.map(FeedDto::from))
}

fn get_feed_by_url_impl(core: &CoreHandle, url: &str) -> Result<Option<FeedDto>, CommandError> {
    Ok(core.get_feed_by_url(url)?.map(FeedDto::from))
}

fn set_feed_status_impl(
    core: &CoreHandle,
    feed_id: i64,
    status: FeedStatusDto,
) -> Result<(), CommandError> {
    Ok(core.set_feed_status(FeedId(feed_id), FeedStatus::from(status))?)
}

fn update_feed_metadata_impl(
    core: &CoreHandle,
    feed_id: i64,
    title: Option<String>,
    site_url: Option<String>,
    description: Option<String>,
) -> Result<(), CommandError> {
    Ok(core.update_feed_metadata(FeedId(feed_id), title, site_url, description)?)
}

fn set_feed_tags_impl(
    core: &CoreHandle,
    feed_id: i64,
    tags: Vec<String>,
) -> Result<(), CommandError> {
    Ok(core.set_feed_tags(FeedId(feed_id), tags)?)
}

fn set_feed_metadata_impl(
    core: &CoreHandle,
    feed_id: i64,
    site_url: Option<String>,
    description: Option<String>,
) -> Result<(), CommandError> {
    Ok(core.set_feed_metadata(FeedId(feed_id), site_url, description)?)
}

fn set_feed_title_impl(
    core: &CoreHandle,
    feed_id: i64,
    title: Option<String>,
) -> Result<(), CommandError> {
    Ok(core.set_feed_title(FeedId(feed_id), title)?)
}

fn set_feed_full_text_impl(
    core: &CoreHandle,
    feed_id: i64,
    enabled: bool,
) -> Result<(), CommandError> {
    Ok(core.set_feed_full_text(FeedId(feed_id), enabled)?)
}

fn reorder_feeds_impl(core: &CoreHandle, feed_ids: &[i64]) -> Result<(), CommandError> {
    let ordered: Vec<FeedId> = feed_ids.iter().map(|&id| FeedId(id)).collect();
    Ok(core.reorder_feeds(&ordered)?)
}

async fn refresh_feed_impl(
    core: &CoreHandle,
    feed_id: i64,
) -> Result<RefreshOutcomeDto, CommandError> {
    Ok(core.refresh_feed(FeedId(feed_id)).await?.into())
}

/// The sweep itself: core's bounded-concurrency `refresh_all_with`, its
/// progress mapped to [`RefreshProgress`] events.
pub(crate) async fn refresh_all_impl(
    core: &CoreHandle,
    mut on_progress: impl FnMut(RefreshProgress),
) -> Result<Sweep, CommandError> {
    let report = core
        .refresh_all_with(RefreshOptions::default(), |step| {
            on_progress(RefreshProgress {
                sweep_id: step.sweep_id,
                done: count(step.done),
                total: count(step.total),
                feed_id: step.outcome.feed_id.0,
                outcome: step.outcome.clone().into(),
            });
        })
        .await?;
    Ok(Sweep {
        joined: report.joined,
        finished: RefreshFinished {
            sweep_id: report.sweep_id,
            cancelled: report.cancelled,
            outcomes: report.outcomes.into_iter().map(Into::into).collect(),
        },
    })
}

/// A feed count for the wire (saturating — no profile has 4 billion feeds).
fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn recent_fetches_impl(
    core: &CoreHandle,
    feed_id: i64,
    limit: u32,
) -> Result<Vec<FetchRecordDto>, CommandError> {
    Ok(core
        .recent_fetches(FeedId(feed_id), limit)?
        .into_iter()
        .map(FetchRecordDto::from)
        .collect())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::commands::testutil::{temp_core, temp_core_trusting_localhost};
    use crate::dto::FetchStatusDto;
    use crate::error::ErrorKind;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>T</title><link>https://t.test</link>
<item><guid>g1</guid><title>One</title><link>https://t.test/1</link><description>first</description></item>
</channel></rss>"#;

    async fn serve(server: &MockServer, at: &str, body: String) {
        Mock::given(method("GET"))
            .and(path(at))
            .respond_with(ResponseTemplate::new(200).set_body_string(body))
            .mount(server)
            .await;
    }

    fn new_feed(url: String, validate: bool) -> NewFeedDto {
        NewFeedDto {
            url,
            title: None,
            tags: vec!["blogs".into()],
            validate,
        }
    }

    /// Subscribe validates by default: a homepage subscribes to the feed
    /// it declares, not to itself.
    #[tokio::test]
    async fn subscribe_resolves_a_homepage_to_its_feed() {
        let server = MockServer::start().await;
        let page = format!(
            r#"<link rel="alternate" type="application/rss+xml" href="{}/feed.xml">"#,
            server.uri()
        );
        serve(&server, "/", page).await;
        serve(&server, "/feed.xml", RSS.to_owned()).await;
        let (_dir, core) = temp_core_trusting_localhost();

        let feed = subscribe_impl(&core, new_feed(format!("{}/", server.uri()), true))
            .await
            .unwrap();
        assert_eq!(feed.url, format!("{}/feed.xml", server.uri()));
        assert_eq!(feed.tags, vec!["blogs".to_owned()]);
        assert_eq!(list_feeds_impl(&core).unwrap().len(), 1);
    }

    /// A page with no feed is refused with `NO_FEED` and creates nothing —
    /// not a subscription that fails on every refresh.
    #[tokio::test]
    async fn subscribe_refuses_a_feedless_page() {
        let server = MockServer::start().await;
        serve(&server, "/", "<p>just a homepage</p>".to_owned()).await;
        let (_dir, core) = temp_core_trusting_localhost();

        let error = subscribe_impl(&core, new_feed(format!("{}/", server.uri()), true))
            .await
            .unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NoFeed);
        assert!(error.message.contains("no feed found"), "{}", error.message);
        assert_eq!(list_feeds_impl(&core).unwrap().len(), 0);
    }

    /// The explicit "subscribe anyway" never fetches, but still normalizes:
    /// a bare domain is stored as an absolute https URL.
    #[tokio::test]
    async fn subscribe_without_validation_normalizes_a_bare_domain() {
        let (_dir, core) = temp_core();
        let feed = subscribe_impl(&core, new_feed(" example.test ".into(), false))
            .await
            .unwrap();
        assert_eq!(feed.url, "https://example.test/");

        let error = subscribe_impl(&core, new_feed("ftp://example.test/".into(), false))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    /// A homepage that resolves to a feed already subscribed is refused
    /// with a code the UI branches on, not an internal UNIQUE failure. This
    /// is the case the frontend's own `get_feed_by_url(typed)` pre-check
    /// cannot see: the typed homepage is not subscribed, the feed it
    /// declares is — so the resolved URL is what gets deduped.
    #[tokio::test]
    async fn subscribe_refuses_a_feed_already_subscribed_under_its_resolved_url() {
        let server = MockServer::start().await;
        let page = format!(
            r#"<link rel="alternate" type="application/rss+xml" href="{}/feed.xml">"#,
            server.uri()
        );
        serve(&server, "/", page).await;
        serve(&server, "/feed.xml", RSS.to_owned()).await;
        let (_dir, core) = temp_core_trusting_localhost();
        let homepage = format!("{}/", server.uri());
        let feed_url = format!("{}/feed.xml", server.uri());
        subscribe_impl(&core, new_feed(feed_url.clone(), false))
            .await
            .unwrap();
        assert!(get_feed_by_url_impl(&core, &homepage).unwrap().is_none());

        for typed in [homepage, feed_url] {
            let error = subscribe_impl(&core, new_feed(typed.clone(), true))
                .await
                .unwrap_err();
            assert_eq!(error.kind, ErrorKind::User, "{typed}");
            assert_eq!(error.code, ErrorCode::AlreadySubscribed, "{typed}");
        }
        assert_eq!(list_feeds_impl(&core).unwrap().len(), 1);
    }

    #[test]
    fn add_list_get_and_dedupe_lookup() {
        let (_dir, core) = temp_core();
        let feed = add_feed_impl(
            &core,
            NewFeed {
                url: "https://example.test/feed.xml".into(),
                title: Some("Example".into()),
                tags: vec!["news".into()],
            },
        )
        .unwrap();
        assert_eq!(feed.status, FeedStatusDto::Active);
        assert_eq!(feed.tags, vec!["news".to_owned()]);

        let listed = list_feeds_impl(&core).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(get_feed_impl(&core, feed.id).unwrap().unwrap().id, feed.id);
        let by_url = get_feed_by_url_impl(&core, "https://example.test/feed.xml").unwrap();
        assert_eq!(by_url.unwrap().id, feed.id);
        assert!(
            get_feed_by_url_impl(&core, "https://other.test/")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn remove_feed_maps_missing_rows_to_user_not_found() {
        let (_dir, core) = temp_core();
        let error = remove_feed_impl(&core, 999).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn status_and_metadata_round_trip() {
        let (_dir, core) = temp_core();
        let feed = add_feed_impl(
            &core,
            NewFeed {
                url: "https://example.test/feed.xml".into(),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();
        set_feed_status_impl(&core, feed.id, FeedStatusDto::Paused).unwrap();
        update_feed_metadata_impl(&core, feed.id, Some("Titled".into()), None, None).unwrap();
        let reloaded = get_feed_impl(&core, feed.id).unwrap().unwrap();
        assert_eq!(reloaded.status, FeedStatusDto::Paused);
        assert_eq!(reloaded.title.as_deref(), Some("Titled"));
    }

    #[test]
    fn set_feed_tags_overwrites_wholesale_and_normalizes() {
        let (_dir, core) = temp_core();
        let feed = add_feed_impl(
            &core,
            NewFeed {
                url: "https://example.test/feed.xml".into(),
                title: None,
                tags: vec!["Tech".into()],
            },
        )
        .unwrap();

        // Overwrite (not merge) with a nested path tag; trims, drops empties,
        // dedupes — the same normalization add_feed applies.
        set_feed_tags_impl(
            &core,
            feed.id,
            vec![
                "  Tech/Databases  ".into(),
                String::new(),
                "fav".into(),
                "fav".into(),
            ],
        )
        .unwrap();
        let reloaded = get_feed_impl(&core, feed.id).unwrap().unwrap();
        assert_eq!(
            reloaded.tags,
            vec!["Tech/Databases".to_owned(), "fav".to_owned()]
        );

        let error = set_feed_tags_impl(&core, 9999, vec!["x".into()]).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn set_feed_title_overwrites_unconditionally_and_clears() {
        let (_dir, core) = temp_core();
        let feed = add_feed_impl(
            &core,
            NewFeed {
                url: "https://example.test/feed.xml".into(),
                title: Some("Original".into()),
                tags: vec![],
            },
        )
        .unwrap();

        // Unconditional rename (unlike update_feed_metadata's COALESCE fill,
        // which would keep "Original").
        set_feed_title_impl(&core, feed.id, Some("Renamed".into())).unwrap();
        assert_eq!(
            get_feed_impl(&core, feed.id)
                .unwrap()
                .unwrap()
                .title
                .as_deref(),
            Some("Renamed")
        );

        // An empty/whitespace title clears it to None (the URL fallback).
        set_feed_title_impl(&core, feed.id, Some("   ".into())).unwrap();
        assert_eq!(get_feed_impl(&core, feed.id).unwrap().unwrap().title, None);

        let error = set_feed_title_impl(&core, 9999, Some("x".into())).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn set_feed_metadata_overwrites_where_update_only_fills() {
        let (_dir, core) = temp_core();
        let feed = add_feed_impl(
            &core,
            NewFeed {
                url: "https://example.test/feed.xml".into(),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();

        // The user overwrites site URL + description (unlike
        // update_feed_metadata's COALESCE fill, this is unconditional).
        set_feed_metadata_impl(
            &core,
            feed.id,
            Some("https://example.test".into()),
            Some("Edited note".into()),
        )
        .unwrap();
        let reloaded = get_feed_impl(&core, feed.id).unwrap().unwrap();
        assert_eq!(reloaded.site_url.as_deref(), Some("https://example.test"));
        assert_eq!(reloaded.description.as_deref(), Some("Edited note"));

        // A later fetch-fill must not clobber the human edit.
        update_feed_metadata_impl(
            &core,
            feed.id,
            None,
            Some("https://feed-said.test".into()),
            None,
        )
        .unwrap();
        assert_eq!(
            get_feed_impl(&core, feed.id)
                .unwrap()
                .unwrap()
                .site_url
                .as_deref(),
            Some("https://example.test")
        );

        let error = set_feed_metadata_impl(&core, 9999, None, None).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::NotFound);
    }

    #[test]
    fn reorder_feeds_rewrites_the_listed_order() {
        let (_dir, core) = temp_core();
        let mk = |url: &str| {
            add_feed_impl(
                &core,
                NewFeed {
                    url: url.into(),
                    title: None,
                    tags: vec![],
                },
            )
            .unwrap()
            .id
        };
        let a = mk("https://a.test/feed.xml");
        let b = mk("https://b.test/feed.xml");
        let c = mk("https://c.test/feed.xml");

        let ids = || {
            list_feeds_impl(&core)
                .unwrap()
                .into_iter()
                .map(|f| f.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(), vec![a, b, c], "subscription order by default");

        reorder_feeds_impl(&core, &[c, b, a]).unwrap();
        assert_eq!(ids(), vec![c, b, a]);
    }

    #[tokio::test]
    async fn refresh_all_reports_progress_and_skips_paused_feeds() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .mount(&server)
            .await;

        let (_dir, core) = temp_core_trusting_localhost();
        let active = add_feed_impl(
            &core,
            NewFeed {
                url: format!("{}/a.xml", server.uri()),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();
        let paused = add_feed_impl(
            &core,
            NewFeed {
                url: format!("{}/paused.xml", server.uri()),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();
        set_feed_status_impl(&core, paused.id, FeedStatusDto::Paused).unwrap();

        let mut progressed = Vec::new();
        let sweep = refresh_all_impl(&core, |progress| {
            progressed.push((
                progress.sweep_id,
                progress.done,
                progress.total,
                progress.feed_id,
                progress.outcome.status,
            ));
        })
        .await
        .unwrap();

        assert!(!sweep.joined && !sweep.finished.cancelled);
        let outcomes = &sweep.finished.outcomes;
        assert_eq!(outcomes.len(), 1, "paused feeds are not refreshed");
        assert_eq!(outcomes[0].feed_id, active.id);
        assert_eq!(outcomes[0].status, FetchStatusDto::Ok);
        assert_eq!(outcomes[0].new_articles, 1);
        assert_eq!(
            progressed,
            vec![(sweep.finished.sweep_id, 1, 1, active.id, FetchStatusDto::Ok)]
        );

        let records = recent_fetches_impl(&core, active.id, 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, FetchStatusDto::Ok);
    }

    /// A refresh pressed while a sweep runs joins it: one fetch per feed,
    /// and only the running sweep reports progress (the joined request
    /// emits nothing, so the UI never sees a sweep twice).
    #[tokio::test]
    async fn a_second_refresh_all_joins_the_running_sweep() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/a.xml"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(RSS)
                    .set_delay(std::time::Duration::from_millis(200)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (_dir, core) = temp_core_trusting_localhost();
        add_feed_impl(
            &core,
            NewFeed {
                url: format!("{}/a.xml", server.uri()),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();

        let mut joiner_progress = 0;
        let (owner, joiner) = tokio::join!(refresh_all_impl(&core, |_| {}), async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            refresh_all_impl(&core, |_| joiner_progress += 1).await
        });
        let (owner, joiner) = (owner.unwrap(), joiner.unwrap());

        assert!(!owner.joined);
        assert!(joiner.joined);
        assert_eq!(joiner_progress, 0);
        assert_eq!(owner.finished.sweep_id, joiner.finished.sweep_id);
        assert_eq!(owner.finished.outcomes, joiner.finished.outcomes);
        server.verify().await;
        assert!(!core.cancel_refresh(), "nothing left to cancel");
    }
}
