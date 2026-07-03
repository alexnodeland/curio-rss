//! End-to-end facade tests: profile open → subscribe → refresh (against
//! a wiremock fixture feed on 127.0.0.1, exercising the W1 exemption) →
//! state flips with negation events → save-to-destination → the JSONL
//! stream folds back to the same state.

#![allow(clippy::unwrap_used)]

use std::time::Duration;

use curio_core::events::{FoldedState, read_all};
use curio_core::export::{ExportDisposition, ExportInput, stage_export_note};
use curio_core::fetch::FetchConfig;
use curio_core::model::{ArticleContent, FeedStatus, FetchStatus, NewArticle, NewFeed};
use curio_core::storage::ListArticles;
use curio_core::{CoreHandle, CoreOptions};
use curio_types::{Destination, DestinationName, EventPayload};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSS: &str = include_str!("../../../fixtures/feeds/rss2.xml");

fn open_core(profile: &std::path::Path) -> CoreHandle {
    CoreHandle::open_with(
        profile,
        CoreOptions {
            fetch: FetchConfig {
                politeness_delay: Duration::ZERO,
                ..FetchConfig::default()
            },
        },
    )
    .unwrap()
}

/// Subscribes to the mock feed with the W1 exemption set (the fixture
/// lives on 127.0.0.1 — this is also the proof the exemption works).
fn subscribe(core: &CoreHandle, url: &str) -> curio_core::model::Feed {
    let feed = core
        .add_feed(NewFeed {
            url: url.to_owned(),
            title: None,
            tags: vec!["fixtures".to_owned()],
        })
        .unwrap();
    core.set_feed_allow_private_network(feed.id, true).unwrap();
    feed
}

#[tokio::test]
async fn the_full_reading_loop_works_end_to_end() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_raw(RSS, "application/rss+xml"),
        )
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());

    // Subscribe (emits feed.added) + refresh through the policed client.
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Ok);
    assert_eq!(outcome.new_articles, 3);
    assert_eq!(outcome.updated_articles, 0);

    // Feed metadata was learned from the parse; validators stored.
    let feed = core.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(feed.title.as_deref(), Some("Example Blog"));
    assert_eq!(feed.etag.as_deref(), Some("\"v1\""));

    // Articles are stored sanitized, with real FTS text.
    let articles = core.list_articles(ListArticles::default()).unwrap();
    assert_eq!(articles.len(), 3);
    let hit = core.search_articles("content", 10).unwrap();
    assert_eq!(hit.len(), 1, "content:encoded body must be FTS-indexed");
    let article = &hit[0];
    assert!(article.content.html.contains("<strong>content</strong>"));

    // State flips stage + emit events; negations honored.
    assert!(core.set_starred(article.id, true).unwrap());
    assert!(!core.set_starred(article.id, true).unwrap(), "idempotent");
    assert!(core.set_read_later(article.id, true).unwrap());
    assert!(core.set_read_later(article.id, false).unwrap());
    assert!(core.tag_article(article.id, "rust").unwrap());
    core.record_opened(article.id, Some(1234)).unwrap();
    let state = core.article_state(article.id).unwrap();
    assert!(state.starred);
    assert!(!state.read_later);

    // Export to a named destination → note + manifest + article.saved.
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();
    let saved = core.save_to_destination(article.id, &dest).unwrap();
    assert_eq!(saved.disposition, ExportDisposition::Created);
    let note = std::fs::read_to_string(vault.path().join(&saved.path)).unwrap();
    assert!(note.contains(&format!("curio_id: {}", article.curio_id)));
    assert!(note.contains("tags:\n- rust"), "tags ride the frontmatter");

    // Idempotent re-save: no event, no change.
    let resaved = core.save_to_destination(article.id, &dest).unwrap();
    assert_eq!(resaved.disposition, ExportDisposition::Unchanged);

    // The JSONL stream tells the whole story and folds back to state.
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.event.event_type()).collect();
    assert_eq!(
        types,
        vec![
            "feed.added",
            "article.starred",
            "article.read_later.added",
            "article.read_later.removed",
            "article.tagged",
            "article.opened",
            "article.saved",
        ]
    );
    let folded = FoldedState::fold(events.clone());
    assert!(folded.starred.contains(&article.curio_id));
    assert!(!folded.read_later.contains(&article.curio_id));
    assert!(folded.feeds.contains_key(&feed.url));
    let snapshot = folded.articles.get(&article.curio_id).unwrap();
    assert_eq!(snapshot.path, saved.path);
    assert_eq!(snapshot.checksum, saved.checksum);
    assert_eq!(snapshot.tags, vec!["rust".to_owned()], "tags-in-payload");

    // feed.added carried the subscription tags.
    let EventPayload::FeedAdded { tags, .. } = &events[0].event else {
        panic!("first event must be feed.added");
    };
    assert_eq!(tags, &vec!["fixtures".to_owned()]);
}

#[tokio::test]
async fn conditional_refresh_hits_304_and_preserves_validators() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v1\"")
                .set_body_raw(RSS, "application/rss+xml"),
        )
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));

    let first = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(first.status, FetchStatus::Ok);
    let second = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(second.status, FetchStatus::NotModified);
    assert_eq!(second.new_articles, 0);

    // Validators survived the 304 for the next round.
    let feed = core.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(feed.etag.as_deref(), Some("\"v1\""));
    assert_eq!(
        core.list_articles(ListArticles::default()).unwrap().len(),
        3
    );
}

#[tokio::test]
async fn http_410_auto_pauses_the_feed_for_good() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(410))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));

    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Error);
    assert_eq!(outcome.http_status, Some(410));
    assert_eq!(
        core.get_feed(feed.id).unwrap().unwrap().status,
        FeedStatus::Dead
    );

    // A dead feed is never fetched again — even explicitly.
    let again = core.refresh_feed(feed.id).await.unwrap();
    assert!(again.error.unwrap().contains("dead"));
    // And refresh_all skips it entirely.
    assert!(core.refresh_all().await.unwrap().is_empty());
}

#[tokio::test]
async fn ssrf_guard_holds_inside_the_facade_without_the_w1_flag() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RSS, "application/rss+xml"))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    // No allow_private_network here — the default policy must refuse.
    let feed = core
        .add_feed(NewFeed {
            url: format!("{}/feed.xml", server.uri()),
            title: None,
            tags: Vec::new(),
        })
        .unwrap();

    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Error);
    assert!(outcome.error.unwrap().contains("not public"));
    assert!(
        core.list_articles(ListArticles::default())
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn opml_round_trips_through_the_facade() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());

    let opml = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../fixtures/opml/nested.opml"
    ))
    .unwrap();
    let imported = core.import_opml(&opml).unwrap();
    assert_eq!(imported.added, 4);
    assert_eq!(imported.skipped, 0);

    // Importing the same document again is a no-op.
    let again = core.import_opml(&opml).unwrap();
    assert_eq!(again.added, 0);
    assert_eq!(again.skipped, 4);

    let exported = core.export_opml().unwrap();
    for url in [
        "https://blog.rust-lang.org/feed.xml",
        "https://sqlite.example/news.xml",
        "https://bread.example/feed",
        "https://top.example/feed.xml",
    ] {
        assert!(exported.contains(url), "{url} missing from export");
    }

    // Every import emitted feed.added with its folder tags.
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    let folded = FoldedState::fold(events);
    assert_eq!(folded.feeds.len(), 4);
    assert_eq!(
        folded.feeds["https://sqlite.example/news.xml"].tags,
        vec!["Tech".to_owned(), "Databases".to_owned()]
    );
}

#[tokio::test]
async fn updated_content_re_export_emits_article_updated() {
    let server = MockServer::start().await;
    let v2 = RSS.replace(
        "<p>Full <strong>content</strong> body.</p>",
        "<p>Full <strong>content</strong> body, revised edition.</p>",
    );
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RSS, "application/rss+xml"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(v2, "application/rss+xml"))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();

    core.refresh_feed(feed.id).await.unwrap();
    let article = core.search_articles("content", 1).unwrap().remove(0);
    let first = core.save_to_destination(article.id, &dest).unwrap();
    assert_eq!(first.disposition, ExportDisposition::Created);

    // Second refresh updates the article in place (same dedupe key).
    let second_refresh = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(second_refresh.new_articles, 0);
    assert_eq!(second_refresh.updated_articles, 3);

    let second = core.save_to_destination(article.id, &dest).unwrap();
    assert_eq!(second.disposition, ExportDisposition::Updated);
    assert_eq!(second.path, first.path, "curio_id keeps its note path");
    assert_ne!(second.checksum, first.checksum);

    // The event stream carries saved → updated with the new checksum.
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    let saves: Vec<&str> = events
        .iter()
        .map(|e| e.event.event_type())
        .filter(|t| t.starts_with("article.s") || t.starts_with("article.u"))
        .collect();
    assert_eq!(saves, vec!["article.saved", "article.updated"]);
    let folded = FoldedState::fold(events);
    assert_eq!(folded.articles[&article.curio_id].checksum, second.checksum);
}

/// Crash between staging the article.saved intent and the manifest write:
/// the event must survive (startup replay), and the next save re-converges
/// the manifest. Regression for the ordering bug where the manifest was
/// made durable before any intent existed — the retry's
/// `(curio_id, checksum)` idempotency hit then suppressed the event forever.
#[test]
fn a_crash_before_the_manifest_write_never_loses_article_saved() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let dest_name: DestinationName = "vault".parse().unwrap();

    let (article_id, curio_id) = {
        let core = open_core(profile.path());
        core.add_destination(dest_name.clone(), vault.path().to_path_buf())
            .unwrap();
        core.storage()
            .upsert_articles(vec![NewArticle {
                feed_id: None,
                dedupe_key: "k1".to_owned(),
                title: "Title".to_owned(),
                source_url: "https://example.com/a".to_owned(),
                author: None,
                published_at: None,
                content: ArticleContent {
                    html: "<p>body</p>".to_owned(),
                    text: "body".to_owned(),
                },
                lang: None,
                word_count: None,
                source_updated_at: None,
            }])
            .unwrap();
        let article = core
            .list_articles(ListArticles::default())
            .unwrap()
            .remove(0);

        // Simulate the crashing save: note written, intent staged — and
        // the process dies before the manifest commit and the emit.
        let input = ExportInput {
            curio_id: article.curio_id,
            title: article.title.clone(),
            source: article.source_url.clone(),
            feed: None,
            feed_title: None,
            author: None,
            published: None,
            saved: article.saved_at,
            tags: vec![],
            lang: None,
            word_count: None,
            markdown: "body".to_owned(),
        };
        let dest = Destination {
            name: dest_name.clone(),
            root: vault.path().to_path_buf(),
        };
        let staged = stage_export_note(&dest, &input).unwrap();
        let snapshot = article.snapshot(
            None,
            None,
            vec![],
            dest_name.clone(),
            staged.outcome().path.clone(),
            staged.outcome().checksum,
        );
        core.storage().record_article_saved(snapshot).unwrap();
        drop(staged); // the manifest write never happens
        (article.id, article.curio_id)
    }; // dropping the handle = the crash

    // Restart: the startup replay emits the staged article.saved.
    let core = open_core(profile.path());
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    assert!(
        events.iter().any(|e| matches!(
            &e.event,
            EventPayload::ArticleSaved(s) if s.curio_id == curio_id
        )),
        "the saved event must survive the crash window"
    );

    // And the next save re-converges the manifest without losing anything.
    let saved = core.save_to_destination(article_id, &dest_name).unwrap();
    assert_eq!(saved.disposition, ExportDisposition::Created);
    let folded = FoldedState::fold(read_all(&profile.path().join(".curio/events")).unwrap());
    assert_eq!(folded.articles[&curio_id].path, saved.path);
}

#[tokio::test]
async fn reopening_a_profile_restores_destinations_and_state() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    {
        let core = open_core(profile.path());
        core.add_destination(dest.clone(), vault.path().to_path_buf())
            .unwrap();
        core.add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: Some("Example".to_owned()),
            tags: Vec::new(),
        })
        .unwrap();
    }
    let core = open_core(profile.path());
    let destinations = core.destinations();
    assert_eq!(destinations.len(), 1);
    assert_eq!(destinations[0].name, dest);
    assert_eq!(core.list_feeds().unwrap().len(), 1);
    // The events dir is gitignored by construction.
    let gitignore = std::fs::read_to_string(profile.path().join(".curio/.gitignore")).unwrap();
    assert!(gitignore.contains("events/"));
}
