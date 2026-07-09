//! Feed commands: subscribe, remove, list, status, metadata, refresh,
//! and the feed-health fetch log.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::{FeedId, FeedStatus};
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::discovery::{Discovery, DiscoveryDto};
use crate::dto::{FeedDto, FeedStatusDto, FetchRecordDto, NewFeedDto, RefreshOutcomeDto};
use crate::error::CommandError;
use crate::events::{ArticlesChanged, FeedsChanged, RefreshFinished, RefreshProgress, emit_or_log};

/// Autodiscovers feeds (and a favicon) for a URL the user typed — fetches
/// the page once through the policed client and scans it. Head-local
/// platform policy; the Google favicon fallback is the frontend's opt-in.
#[tauri::command]
#[specta::specta]
pub async fn discover_feeds(
    discovery: State<'_, Discovery>,
    url: String,
) -> Result<DiscoveryDto, CommandError> {
    discovery.discover(&url).await
}

/// Subscribes to a feed. Duplicate URLs are a storage error (user tier).
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
    let feed = run_blocking(move || add_feed_impl(&core, new_feed)).await?;
    emit_or_log(&app, &FeedsChanged);
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

/// Refreshes every `active` feed, emitting [`RefreshProgress`] per feed
/// (core's own `refresh_all` is sequential and silent — the head loops
/// `refresh_feed` so the UI can watch; orchestration, not business logic).
#[tauri::command]
#[specta::specta]
pub async fn refresh_all(
    app: AppHandle,
    core: State<'_, SharedCore>,
) -> Result<Vec<RefreshOutcomeDto>, CommandError> {
    let core = Arc::clone(core.inner());
    let outcomes = refresh_all_impl(&core, |feed_id, outcome| {
        emit_or_log(
            &app,
            &RefreshProgress {
                feed_id,
                outcome: outcome.clone(),
            },
        );
    })
    .await?;
    emit_or_log(
        &app,
        &RefreshFinished {
            outcomes: outcomes.clone(),
        },
    );
    emit_or_log(&app, &FeedsChanged);
    emit_or_log(&app, &ArticlesChanged { feed_id: None });
    Ok(outcomes)
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

fn add_feed_impl(core: &CoreHandle, new: NewFeedDto) -> Result<FeedDto, CommandError> {
    Ok(core.add_feed(new.into())?.into())
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
    Ok(core.storage().get_feed_by_url(url)?.map(FeedDto::from))
}

fn set_feed_status_impl(
    core: &CoreHandle,
    feed_id: i64,
    status: FeedStatusDto,
) -> Result<(), CommandError> {
    Ok(core
        .storage()
        .set_feed_status(FeedId(feed_id), FeedStatus::from(status))?)
}

fn update_feed_metadata_impl(
    core: &CoreHandle,
    feed_id: i64,
    title: Option<String>,
    site_url: Option<String>,
    description: Option<String>,
) -> Result<(), CommandError> {
    Ok(core
        .storage()
        .update_feed_metadata(FeedId(feed_id), title, site_url, description)?)
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

async fn refresh_all_impl(
    core: &Arc<CoreHandle>,
    mut on_progress: impl FnMut(i64, &RefreshOutcomeDto),
) -> Result<Vec<RefreshOutcomeDto>, CommandError> {
    let listing = Arc::clone(core);
    let active: Vec<i64> = run_blocking(move || {
        Ok(listing
            .list_feeds()
            .map_err(CommandError::from)?
            .into_iter()
            .filter(|feed| feed.status == FeedStatus::Active)
            .map(|feed| feed.id.0)
            .collect())
    })
    .await?;
    let mut outcomes = Vec::with_capacity(active.len());
    for feed_id in active {
        let outcome = refresh_feed_impl(core, feed_id).await?;
        on_progress(feed_id, &outcome);
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

fn recent_fetches_impl(
    core: &CoreHandle,
    feed_id: i64,
    limit: u32,
) -> Result<Vec<FetchRecordDto>, CommandError> {
    Ok(core
        .storage()
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
    use crate::error::{ErrorCode, ErrorKind};

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>T</title><link>https://t.test</link>
<item><guid>g1</guid><title>One</title><link>https://t.test/1</link><description>first</description></item>
</channel></rss>"#;

    #[test]
    fn add_list_get_and_dedupe_lookup() {
        let (_dir, core) = temp_core();
        let feed = add_feed_impl(
            &core,
            NewFeedDto {
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
            NewFeedDto {
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
            NewFeedDto {
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
            NewFeedDto {
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
            NewFeedDto {
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
                NewFeedDto {
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
            NewFeedDto {
                url: format!("{}/a.xml", server.uri()),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();
        let paused = add_feed_impl(
            &core,
            NewFeedDto {
                url: format!("{}/paused.xml", server.uri()),
                title: None,
                tags: vec![],
            },
        )
        .unwrap();
        set_feed_status_impl(&core, paused.id, FeedStatusDto::Paused).unwrap();

        let mut progressed = Vec::new();
        let outcomes = refresh_all_impl(&core, |feed_id, outcome| {
            progressed.push((feed_id, outcome.status));
        })
        .await
        .unwrap();

        assert_eq!(outcomes.len(), 1, "paused feeds are not refreshed");
        assert_eq!(outcomes[0].feed_id, active.id);
        assert_eq!(outcomes[0].status, FetchStatusDto::Ok);
        assert_eq!(outcomes[0].new_articles, 1);
        assert_eq!(progressed, vec![(active.id, FetchStatusDto::Ok)]);

        let records = recent_fetches_impl(&core, active.id, 10).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].status, FetchStatusDto::Ok);
    }
}
