//! End-to-end facade tests: profile open → subscribe → refresh (against
//! a wiremock fixture feed on 127.0.0.1, exercising the W1 exemption) →
//! state flips with negation events → save-to-destination → the JSONL
//! stream folds back to the same state.

#![allow(clippy::unwrap_used)]

mod common;

use std::time::Duration;

use curio_core::events::{FoldedState, read_all};
use curio_core::export::{ExportDisposition, ExportInput, stage_export_note};
use curio_core::fetch::FetchConfig;
use curio_core::model::{
    ArticleContent, FeedStatus, FetchErrorKind, FetchStatus, NewArticle, NewFeed,
};
use curio_core::storage::{ListArticles, StorageError};
use curio_core::{CoreHandle, CoreOptions, ImportSource};
use curio_types::{Destination, DestinationName, EventPayload};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSS: &str = include_str!("../../../fixtures/feeds/rss2.xml");

/// Re-imports an exported OPML document (round-trip helper).
fn crate_reimport(xml: &str) -> Vec<curio_core::feeds::OpmlFeed> {
    curio_core::feeds::import_opml(xml).unwrap()
}

fn open_core(profile: &std::path::Path) -> CoreHandle {
    CoreHandle::open_with(
        profile,
        CoreOptions {
            fetch: FetchConfig {
                politeness_delay: Duration::ZERO,
                ..FetchConfig::default()
            },
            ..CoreOptions::default()
        },
    )
    .unwrap()
}

/// A core whose SSRF guard trusts `127.0.0.1` — for paths like
/// [`CoreHandle::save_url`] that carry no per-feed W1 exemption but must
/// still be exercised against hermetic localhost fixtures.
fn open_core_trusting_localhost(profile: &std::path::Path) -> CoreHandle {
    CoreHandle::open_with(
        profile,
        CoreOptions {
            fetch: FetchConfig {
                politeness_delay: Duration::ZERO,
                trusted_addrs: std::iter::once(std::net::IpAddr::from([127, 0, 0, 1])).collect(),
                ..FetchConfig::default()
            },
            ..CoreOptions::default()
        },
    )
    .unwrap()
}

/// A manual (feedless) article for facade tests that need rows without
/// a network round trip.
fn manual_article(key: &str, title: &str) -> NewArticle {
    NewArticle {
        feed_id: None,
        dedupe_key: key.to_owned(),
        title: title.to_owned(),
        source_url: format!("https://example.com/{key}"),
        author: None,
        published_at: None,
        content: ArticleContent {
            html: "<p>body</p>".to_owned(),
            text: "body".to_owned(),
        },
        lang: None,
        word_count: None,
        source_updated_at: None,
        lead_image: None,
    }
}

/// Reconciles a one-destination head configuration.
fn reconcile(
    core: &CoreHandle,
    name: &DestinationName,
    root: &std::path::Path,
) -> Result<(), curio_core::CoreError> {
    core.reconcile_destinations([(name.clone(), root.to_path_buf())].into())
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

/// The conditional-GET validator race (docs/design/known-issues.md): an
/// error-path preserve used to write back its pre-fetch etag snapshot,
/// clobbering fresher validators a parallel successful refresh had just
/// stored. Per-feed serialization closes it: the queued refresh re-reads
/// the feed inside the lock, so its preserve carries the fresh pair.
#[tokio::test]
async fn concurrent_refreshes_of_one_feed_cannot_clobber_validators() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Request 1 (setup): 200 + etag v1. Request 2: 200 + etag v2.
    /// Request 3: a slow 500 — the error path whose validator preserve
    /// used to clobber.
    struct Script(AtomicUsize);
    impl wiremock::Respond for Script {
        fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
            match self.0.fetch_add(1, Ordering::SeqCst) {
                0 => ResponseTemplate::new(200)
                    .insert_header("etag", "\"v1\"")
                    .set_body_raw(RSS, "application/rss+xml"),
                1 => ResponseTemplate::new(200)
                    .insert_header("etag", "\"v2\"")
                    .set_body_raw(RSS, "application/rss+xml"),
                _ => ResponseTemplate::new(500).set_delay(Duration::from_millis(150)),
            }
        }
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(Script(AtomicUsize::new(0)))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    let setup = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(setup.status, FetchStatus::Ok);

    // Two concurrent refreshes: one lands fresh validators (200, v2),
    // the other errors (500, deliberately slower). Unserialized, the
    // error path would preserve its stale pre-fetch snapshot (v1) AFTER
    // the success stored v2.
    let (a, b) = tokio::join!(core.refresh_feed(feed.id), core.refresh_feed(feed.id));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(
        (a.status == FetchStatus::Ok) ^ (b.status == FetchStatus::Ok),
        "exactly one refresh succeeds: {a:?} / {b:?}"
    );
    assert!(a.status == FetchStatus::Error || b.status == FetchStatus::Error);

    // The fresher validators won.
    let stored = core.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.etag.as_deref(), Some("\"v2\""));

    // Serialization proof: the queued refresh re-read the feed inside
    // the lock, so its conditional GET already carried the new etag.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let last_inm = requests[2]
        .headers
        .get("if-none-match")
        .map(|v| v.to_str().unwrap().to_owned());
    assert_eq!(last_inm.as_deref(), Some("\"v2\""));
}

/// The same race across processes (#114 / #234): the desktop app and a
/// CLI `curio fetch` are two handles on one profile, with separate
/// in-process locks — only the per-feed lock file serializes them. Two
/// handles in one process stand in for the two processes: file locks are
/// per open file, so they exclude each other exactly as across processes.
#[tokio::test]
async fn concurrent_refreshes_from_two_handles_cannot_clobber_validators() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Request 1 (setup): 200 + etag v1. Request 2: 200 + etag v2.
    /// Request 3: a slow 500 — the error path whose validator preserve
    /// would clobber.
    struct Script(AtomicUsize);
    impl wiremock::Respond for Script {
        fn respond(&self, _: &wiremock::Request) -> ResponseTemplate {
            match self.0.fetch_add(1, Ordering::SeqCst) {
                0 => ResponseTemplate::new(200)
                    .insert_header("etag", "\"v1\"")
                    .set_body_raw(RSS, "application/rss+xml"),
                1 => ResponseTemplate::new(200)
                    .insert_header("etag", "\"v2\"")
                    .set_body_raw(RSS, "application/rss+xml"),
                _ => ResponseTemplate::new(500).set_delay(Duration::from_millis(150)),
            }
        }
    }

    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(Script(AtomicUsize::new(0)))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let desktop = open_core(profile.path());
    let cli = open_core(profile.path());
    let feed = subscribe(&desktop, &format!("{}/feed.xml", server.uri()));
    let setup = desktop.refresh_feed(feed.id).await.unwrap();
    assert_eq!(setup.status, FetchStatus::Ok);

    let (a, b) = tokio::join!(desktop.refresh_feed(feed.id), cli.refresh_feed(feed.id));
    let (a, b) = (a.unwrap(), b.unwrap());
    assert!(
        (a.status == FetchStatus::Ok) ^ (b.status == FetchStatus::Ok),
        "exactly one refresh succeeds: {a:?} / {b:?}"
    );

    // The fresher validators won, whichever handle reads them.
    for core in [&desktop, &cli] {
        let stored = core.get_feed(feed.id).unwrap().unwrap();
        assert_eq!(stored.etag.as_deref(), Some("\"v2\""));
    }
    // Serialization proof: the queued refresh read the feed only once it
    // held the lock, so its conditional GET already carried the new etag.
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 3);
    let last_inm = requests[2]
        .headers
        .get("if-none-match")
        .map(|v| v.to_str().unwrap().to_owned());
    assert_eq!(last_inm.as_deref(), Some("\"v2\""));
    // The lock files live under .curio/locks/, which is gitignored.
    assert!(profile.path().join(".curio/locks").is_dir());
    let gitignore = std::fs::read_to_string(profile.path().join(".curio/.gitignore")).unwrap();
    assert!(gitignore.lines().any(|line| line == "locks/"));
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
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::Gone));
    assert_eq!(
        core.get_feed(feed.id).unwrap().unwrap().status,
        FeedStatus::Dead
    );

    // A dead feed is never fetched again — even explicitly.
    let again = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(again.error_kind, Some(FetchErrorKind::Gone));
    assert!(again.error.unwrap().contains("dead"));
    // And refresh_all skips it entirely.
    assert_eq!(
        core.refresh_all().await.unwrap(),
        [] as [curio_core::RefreshOutcome; 0]
    );
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
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::Blocked));
    assert!(outcome.error.unwrap().contains("not public"));
    assert_eq!(
        core.list_articles(ListArticles::default()).unwrap(),
        [] as [curio_core::model::Article; 0]
    );
}

/// A subscription that points at a web page is reported as one — with the
/// feeds the page links to counted — not as an XML parse error.
#[tokio::test]
async fn a_web_page_at_a_feed_url_is_not_a_feed_and_counts_its_feeds() {
    let server = MockServer::start().await;
    let page = |links: &str| {
        format!(
            "<!DOCTYPE html><html><head><title>Blog</title>{links}</head>\
             <body><p>Hello</p></body></html>"
        )
    };
    let two = page(
        r#"<link rel="alternate" type="application/rss+xml" href="/feed.xml">
           <link rel="alternate" type="application/atom+xml" href="/atom.xml">"#,
    );
    let one = page(r#"<link rel="alternate" type="application/rss+xml" href="/feed.xml">"#);
    for (route, body) in [("/two", two), ("/one", one), ("/none", page(""))] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, "text/html"))
            .mount(&server)
            .await;
    }
    // HTML5 lets a page omit <html>/<head>/<body>: the doctype marks it,
    // even under a Content-Type that says nothing about HTML.
    let tagless = r#"<!DOCTYPE html><meta charset=utf-8><title>Blog</title>
        <link rel="alternate" type="application/rss+xml" href="/feed.xml"><p>Hi"#;
    let other_routes = [
        ("/broken.xml", "<rss><channel>", "application/rss+xml"),
        ("/tagless", tagless, "text/plain"),
        // Served as HTML: a page, whatever its markup looks like…
        (
            "/served-html",
            "Under construction",
            "text/html; charset=utf-8",
        ),
        // …unless it sniffs as a feed — then it is a broken feed.
        ("/broken-as-html", "<rss><channel>", "text/html"),
    ];
    for (route, body, content_type) in other_routes {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_raw(body, content_type))
            .mount(&server)
            .await;
    }

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let refresh = async |route: &str| {
        let feed = subscribe(&core, &format!("{}{route}", server.uri()));
        let outcome = core.refresh_feed(feed.id).await.unwrap();
        (feed, outcome)
    };

    let (feed, outcome) = refresh("/two").await;
    assert_eq!(outcome.status, FetchStatus::Error);
    assert_eq!(outcome.http_status, Some(200));
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::NotAFeed));
    let message = outcome.error.unwrap();
    assert!(
        message.starts_with("this is a web page, not a feed — found 2 feeds"),
        "{message}"
    );
    assert!(
        message.contains(&format!("{}/feed.xml", server.uri())),
        "{message}"
    );
    // Persisted: the feed's health carries the kind across a cold start.
    let stored = core.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.last_error_kind, Some(FetchErrorKind::NotAFeed));
    assert_eq!(
        core.recent_fetches(feed.id, 1).unwrap()[0].error_kind,
        Some(FetchErrorKind::NotAFeed)
    );

    let (_, outcome) = refresh("/one").await;
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::NotAFeed));
    assert!(
        outcome.error.unwrap().contains("found 1 feed on it: "),
        "singular"
    );

    let (_, outcome) = refresh("/none").await;
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::NotAFeed));
    assert_eq!(
        outcome.error.as_deref(),
        Some("this is a web page, not a feed — found no feeds on it")
    );

    let (_, outcome) = refresh("/tagless").await;
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::NotAFeed));
    assert!(
        outcome.error.unwrap().contains("found 1 feed on it: "),
        "the tagless page's declared feed"
    );

    let (_, outcome) = refresh("/served-html").await;
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::NotAFeed));

    // A body that is feed-shaped but broken stays a parse error, whatever
    // it is served as.
    for route in ["/broken.xml", "/broken-as-html"] {
        let (_, outcome) = refresh(route).await;
        assert_eq!(outcome.error_kind, Some(FetchErrorKind::Parse), "{route}");
    }
}

#[tokio::test]
async fn a_refused_refresh_is_typed_and_persisted() {
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/feed.xml");
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &url);

    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Error);
    assert_eq!(outcome.http_status, None);
    assert_eq!(outcome.error_kind, Some(FetchErrorKind::Refused));
    let message = outcome.error.unwrap();
    assert_eq!(message.matches(&url).count(), 1, "{message}");
    let stored = core.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.last_error.as_deref(), Some(message.as_str()));
    assert_eq!(stored.last_error_kind, Some(FetchErrorKind::Refused));
}

#[test]
fn subscribing_to_a_feed_twice_is_a_typed_duplicate() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let new = || NewFeed {
        url: "https://example.com/feed.xml".to_owned(),
        title: None,
        tags: Vec::new(),
    };
    core.add_feed(new()).unwrap();
    let err = core.add_feed(new()).unwrap_err();
    assert!(
        matches!(
            err,
            curio_core::CoreError::Storage(StorageError::Duplicate { entity: "feed", .. })
        ),
        "{err:?}"
    );
    assert_eq!(core.list_feeds().unwrap().len(), 1);
}

/// A 301-adopted URL rewrite is a subscription identity change: the fold
/// must end with exactly one live subscription, keyed by the new URL —
/// not a phantom under the original one.
#[tokio::test]
async fn permanent_redirect_adoption_keeps_the_event_fold_consistent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/old.xml"))
        .respond_with(
            ResponseTemplate::new(301)
                .insert_header("location", format!("{}/new.xml", server.uri()).as_str()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/new.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(RSS, "application/rss+xml"))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let old_url = format!("{}/old.xml", server.uri());
    let new_url = format!("{}/new.xml", server.uri());
    let feed = subscribe(&core, &old_url);

    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Ok);
    assert_eq!(core.get_feed(feed.id).unwrap().unwrap().url, new_url);

    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    let types: Vec<&str> = events.iter().map(|e| e.event.event_type()).collect();
    assert_eq!(types, vec!["feed.added", "feed.removed", "feed.added"]);
    let folded = FoldedState::fold(events);
    assert!(
        !folded.feeds.contains_key(&old_url),
        "no phantom subscription under the pre-redirect URL"
    );
    let live = folded.feeds.get(&new_url).unwrap();
    assert_eq!(live.tags, vec!["fixtures".to_owned()], "tags carry over");

    // Removing the feed now negates cleanly.
    core.remove_feed(feed.id).unwrap();
    let folded = FoldedState::fold(read_all(&profile.path().join(".curio/events")).unwrap());
    assert!(folded.feeds.is_empty());
}

#[tokio::test]
async fn opml_round_trips_through_the_facade() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());

    let opml =
        std::fs::read_to_string(common::manifest_dir().join("../../fixtures/opml/nested.opml"))
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
    // Folder/category structure survives the full facade cycle: tags are
    // persisted on the feed row, not only in the one-time feed.added
    // event, so export rebuilds the folder path as nested <outline> folders.
    assert!(
        exported.contains(r#"<outline text="Tech">"#)
            && exported.contains(r#"<outline text="Databases">"#),
        "the folder path must export as nested <outline> folders:\n{exported}"
    );
    let reimported = crate_reimport(&exported);
    assert_eq!(
        reimported
            .iter()
            .find(|f| f.xml_url == "https://sqlite.example/news.xml")
            .unwrap()
            .tags,
        vec!["Tech/Databases".to_owned()]
    );

    // Every import emitted feed.added with its folder tags.
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    let folded = FoldedState::fold(events);
    assert_eq!(folded.feeds.len(), 4);
    assert_eq!(
        folded.feeds["https://sqlite.example/news.xml"].tags,
        vec!["Tech/Databases".to_owned()]
    );
}

#[tokio::test]
async fn feeds_keep_their_subscription_order_until_reordered() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());

    let a = core
        .add_feed(NewFeed {
            url: "https://a.example/feed".into(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let b = core
        .add_feed(NewFeed {
            url: "https://b.example/feed".into(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let c = core
        .add_feed(NewFeed {
            url: "https://c.example/feed".into(),
            title: None,
            tags: vec![],
        })
        .unwrap();

    // New feeds append in subscription order.
    let order = |core: &CoreHandle| {
        core.list_feeds()
            .unwrap()
            .into_iter()
            .map(|f| f.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(order(&core), vec![a.id, b.id, c.id]);

    // Reorder to c, a, b — the sidebar sends the whole new sequence.
    core.reorder_feeds(&[c.id, a.id, b.id]).unwrap();
    assert_eq!(order(&core), vec![c.id, a.id, b.id]);

    // A newly added feed still appends after the reordered set.
    let d = core
        .add_feed(NewFeed {
            url: "https://d.example/feed".into(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    assert_eq!(order(&core), vec![c.id, a.id, b.id, d.id]);
}

#[tokio::test]
async fn pocket_csv_imports_as_tagged_read_later_articles() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());

    let csv =
        std::fs::read_to_string(common::manifest_dir().join("../../fixtures/import/pocket.csv"))
            .unwrap();

    let outcome = core.import_file(ImportSource::PocketCsv, &csv).unwrap();
    // Five rows carry a URL (the blank-URL and fully-empty rows drop out).
    assert_eq!(outcome.articles_added, 5);
    assert_eq!(outcome.feeds_added, 0);

    // They are feedless read-later articles, findable through the queue.
    let read_later = core
        .list_articles(ListArticles {
            read_later: Some(true),
            ..ListArticles::default()
        })
        .unwrap();
    assert_eq!(read_later.len(), 5);
    assert!(
        read_later.iter().all(|a| a.feed_id.is_none()),
        "imported saves have no feed"
    );

    // The multi-tag row split its `|`-separated tags and applied both.
    let ddia = read_later
        .iter()
        .find(|a| a.source_url == "https://example.com/ddia")
        .unwrap();
    assert_eq!(
        ddia.title,
        "Designing Data-Intensive Applications, a review"
    );
    let mut tags = core.article_tags(ddia.id).unwrap();
    tags.sort();
    assert_eq!(tags, vec!["databases", "distributed-systems"]);

    // The read-later flag lives in the event log, folded back per article.
    let folded = FoldedState::fold(read_all(&profile.path().join(".curio/events")).unwrap());
    assert!(folded.read_later.contains(&ddia.curio_id));

    // Re-importing the same export is idempotent — nothing added twice.
    let again = core.import_file(ImportSource::PocketCsv, &csv).unwrap();
    assert_eq!(again.articles_added, 0);
    assert_eq!(again.articles_skipped, 5);
    assert_eq!(
        core.list_articles(ListArticles::default()).unwrap().len(),
        5,
        "no duplicate rows on re-import"
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

    // Second refresh updates the revised article in place (same dedupe
    // key); the two unchanged entries are not rewritten or counted.
    let second_refresh = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(second_refresh.new_articles, 0);
    assert_eq!(second_refresh.updated_articles, 1);

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
                lead_image: None,
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
    // The article's own note is already on disk, so the retry is an
    // update — never a second article.saved.
    let saved = core.save_to_destination(article_id, &dest_name).unwrap();
    assert_eq!(saved.disposition, ExportDisposition::Updated);
    let folded = FoldedState::fold(read_all(&profile.path().join(".curio/events")).unwrap());
    assert_eq!(folded.articles[&curio_id].path, saved.path);
}

/// Concurrent registrations must all survive a reopen: the registry is
/// serialized and persisted inside the write-lock critical section, so a
/// slower thread can no longer persist a map missing a faster thread's
/// entry.
#[test]
fn concurrent_destination_registrations_all_persist() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    {
        let core = std::sync::Arc::new(open_core(profile.path()));
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let core = std::sync::Arc::clone(&core);
                let root = vault.path().join(format!("dest-{i}"));
                std::thread::spawn(move || {
                    core.add_destination(format!("dest-{i}").parse().unwrap(), root)
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap().unwrap();
        }
    }
    let core = open_core(profile.path());
    assert_eq!(
        core.destinations().unwrap().len(),
        8,
        "every registration persisted"
    );
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
    let destinations = core.destinations().unwrap();
    assert_eq!(destinations.len(), 1);
    assert_eq!(destinations[0].name, dest);
    assert_eq!(core.list_feeds().unwrap().len(), 1);
    // The events dir is gitignored by construction.
    let gitignore = std::fs::read_to_string(profile.path().join(".curio/.gitignore")).unwrap();
    assert!(gitignore.contains("events/"));
}

/// Removing a destination forgets the name (persisted across reopen) but
/// never touches the notes already exported under its root.
#[test]
fn remove_destination_forgets_the_name_but_keeps_notes() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    let note_path = {
        let core = open_core(profile.path());
        core.add_destination(dest.clone(), vault.path().to_path_buf())
            .unwrap();
        core.storage()
            .upsert_articles(vec![manual_article("k1", "One")])
            .unwrap();
        let article = core.list_articles(ListArticles::default()).unwrap()[0].id;
        let saved = core.save_to_destination(article, &dest).unwrap();

        core.remove_destination(&dest).unwrap();
        assert_eq!(
            core.destinations().unwrap(),
            [] as [curio_types::Destination; 0]
        );
        // The name is gone from the operation surface…
        let err = core.save_to_destination(article, &dest).unwrap_err();
        assert!(matches!(
            err,
            curio_core::CoreError::UnknownDestination { .. }
        ));
        // …and removing it again reports the same.
        assert!(matches!(
            core.remove_destination(&dest).unwrap_err(),
            curio_core::CoreError::UnknownDestination { .. }
        ));
        saved.path
    };
    // The exported note survives, and the removal persisted.
    assert!(vault.path().join(&note_path).is_file());
    let core = open_core(profile.path());
    assert!(
        core.destinations().unwrap().is_empty(),
        "removal survives reopen"
    );
}

/// #116: once a destination has exported, a vault that moved or was
/// unmounted is refused — not recreated empty at the old path and fully
/// re-exported. The marker check survives a reopen; registering the
/// destination again is the recovery path.
#[test]
fn an_exported_destination_that_vanished_is_destination_missing() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("vault");
    let dest: DestinationName = "vault".parse().unwrap();
    let (first, second) = {
        let core = open_core(profile.path());
        core.add_destination(dest.clone(), root.clone()).unwrap();
        core.storage()
            .upsert_articles(vec![
                manual_article("k1", "One"),
                manual_article("k2", "Two"),
            ])
            .unwrap();
        let ids: Vec<_> = core
            .list_articles(ListArticles::default())
            .unwrap()
            .iter()
            .map(|a| a.id)
            .collect();
        core.save_to_destination(ids[0], &dest).unwrap();
        (ids[0], ids[1])
    };
    let is_missing = |err: curio_core::CoreError| {
        matches!(
            err,
            curio_core::CoreError::Export(
                curio_core::export::ExportError::DestinationMissing { .. }
            )
        )
    };

    // The whole folder goes away (unmounted / moved): refused, and the
    // tree is never recreated.
    std::fs::remove_dir_all(&root).unwrap();
    let core = open_core(profile.path());
    assert!(is_missing(
        core.save_to_destination(second, &dest).unwrap_err()
    ));
    assert!(!root.exists(), "the old path must not be recreated");

    // An empty directory back at the path (an unmounted mount point) is
    // no better: the `.curio/` marker of the earlier exports is missing.
    std::fs::create_dir(&root).unwrap();
    assert!(is_missing(
        core.save_to_destination(second, &dest).unwrap_err()
    ));
    assert!(std::fs::read_dir(&root).unwrap().next().is_none());

    // Registering it again starts the destination afresh.
    core.add_destination(dest.clone(), root.clone()).unwrap();
    let saved = core.save_to_destination(first, &dest).unwrap();
    assert_eq!(saved.disposition, ExportDisposition::Created);
    assert!(root.join(&saved.path).is_file());
}

/// A head re-applying its own configuration (the CLI's curio.toml, on
/// every invocation) through `reconcile_destinations` never undoes the
/// missing-destination check: an unchanged, exported destination is left
/// alone — root not recreated, flag kept — while a new name, a new root or
/// a not-yet-exported destination registers as `add_destination` does.
#[test]
fn reconciling_an_exported_destination_never_recreates_it() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("vault");
    let dest: DestinationName = "vault".parse().unwrap();
    let core = open_core(profile.path());
    // Not registered yet: reconciling registers it and creates the root.
    reconcile(&core, &dest, &root).unwrap();
    assert!(root.is_dir());
    core.storage()
        .upsert_articles(vec![
            manual_article("k1", "One"),
            manual_article("k2", "Two"),
        ])
        .unwrap();
    let ids: Vec<_> = core
        .list_articles(ListArticles::default())
        .unwrap()
        .iter()
        .map(|a| a.id)
        .collect();
    core.save_to_destination(ids[0], &dest).unwrap();

    std::fs::remove_dir_all(&root).unwrap();
    drop(core);
    let core = open_core(profile.path());
    reconcile(&core, &dest, &root).unwrap();
    assert!(!root.exists(), "reconciling must not recreate the root");
    assert!(matches!(
        core.save_to_destination(ids[1], &dest).unwrap_err(),
        curio_core::CoreError::Export(curio_core::export::ExportError::DestinationMissing { .. })
    ));

    // A changed root is a new destination: registered and started afresh.
    let moved = parent.path().join("moved");
    reconcile(&core, &dest, &moved).unwrap();
    assert_eq!(core.destinations().unwrap()[0].root, moved);
    let saved = core.save_to_destination(ids[1], &dest).unwrap();
    assert!(moved.join(&saved.path).is_file());
}

/// A declared root that cannot be created fails the reconcile before
/// anything is registered or recorded as seeded (roots are created ahead
/// of the settings transaction, as `add_destination` does): fixing the
/// path and running again applies it as new.
#[test]
fn a_root_that_cannot_be_created_registers_nothing() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let blocker = parent.path().join("not-a-dir");
    std::fs::write(&blocker, "a file, not a directory").unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    let core = open_core(profile.path());

    let err = reconcile(&core, &dest, &blocker.join("vault")).unwrap_err();
    assert!(matches!(err, curio_core::CoreError::Io { .. }), "{err:?}");
    assert_eq!(core.destinations().unwrap(), [] as [Destination; 0]);

    // Not recorded as seeded either: the same declaration, once its path
    // can be created, registers.
    std::fs::remove_file(&blocker).unwrap();
    reconcile(&core, &dest, &blocker.join("vault")).unwrap();
    assert!(blocker.join("vault").is_dir());
    assert_eq!(core.destinations().unwrap()[0].root, blocker.join("vault"));
}

/// Removing a destination clears its exported flag too: a destination
/// later registered under the same name starts afresh.
#[test]
fn a_removed_then_re_added_destination_starts_afresh() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("vault");
    let dest: DestinationName = "vault".parse().unwrap();
    let core = open_core(profile.path());
    core.add_destination(dest.clone(), root.clone()).unwrap();
    core.storage()
        .upsert_articles(vec![manual_article("k1", "One")])
        .unwrap();
    let id = core.list_articles(ListArticles::default()).unwrap()[0].id;
    core.save_to_destination(id, &dest).unwrap();

    core.remove_destination(&dest).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
    reconcile(&core, &dest, &root).unwrap();
    assert!(root.is_dir(), "a fresh registration creates its root");
    core.save_to_destination(id, &dest).unwrap();
}

/// #114 / #234: curio.toml seeds a destination, it does not own it. A
/// destination the desktop app removed (another handle on the profile)
/// stays removed through every later CLI run (a fresh handle reconciling
/// the same curio.toml), and a re-root made elsewhere stands too.
#[test]
fn a_destination_removed_by_another_head_stays_removed_after_a_cli_run() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let vault = parent.path().join("vault");
    let notes = parent.path().join("notes");
    let declared: std::collections::BTreeMap<DestinationName, std::path::PathBuf> = [
        ("vault".parse().unwrap(), vault.clone()),
        ("notes".parse().unwrap(), notes.clone()),
    ]
    .into();
    let names = |core: &CoreHandle| -> Vec<(String, std::path::PathBuf)> {
        core.destinations()
            .unwrap()
            .into_iter()
            .map(|d| (d.name.to_string(), d.root))
            .collect()
    };

    let desktop = open_core(profile.path());
    {
        let cli = open_core(profile.path());
        cli.reconcile_destinations(declared.clone()).unwrap();
    }
    // The desktop sees what the CLI seeded without reopening.
    assert_eq!(
        names(&desktop),
        [
            ("notes".to_owned(), notes.clone()),
            ("vault".to_owned(), vault.clone())
        ]
    );

    // The desktop removes one destination and re-roots the other.
    let elsewhere = parent.path().join("elsewhere");
    desktop
        .remove_destination(&"vault".parse().unwrap())
        .unwrap();
    desktop
        .add_destination("notes".parse().unwrap(), elsewhere.clone())
        .unwrap();

    // Two more CLI runs with the unchanged curio.toml change nothing.
    for _ in 0..2 {
        let cli = open_core(profile.path());
        cli.reconcile_destinations(declared.clone()).unwrap();
        assert_eq!(names(&cli), [("notes".to_owned(), elsewhere.clone())]);
    }
    assert_eq!(names(&desktop), [("notes".to_owned(), elsewhere.clone())]);

    // Editing curio.toml is a new declaration: a changed root applies.
    let moved = parent.path().join("vault-moved");
    let mut edited = declared.clone();
    edited.insert("vault".parse().unwrap(), moved.clone());
    open_core(profile.path())
        .reconcile_destinations(edited)
        .unwrap();
    assert!(names(&desktop).contains(&("vault".to_owned(), moved)));

    // Dropping a name from curio.toml and declaring it again later
    // applies it afresh.
    desktop
        .remove_destination(&"notes".parse().unwrap())
        .unwrap();
    open_core(profile.path())
        .reconcile_destinations(std::collections::BTreeMap::new())
        .unwrap();
    open_core(profile.path())
        .reconcile_destinations(declared)
        .unwrap();
    assert!(names(&desktop).contains(&("notes".to_owned(), notes)));
}

/// #234: no handle writes back a stale copy of the registry. Changes made
/// through one handle while another is open survive every later change
/// the other one makes, and the other one sees them at once.
#[test]
fn a_destination_added_through_another_handle_is_never_erased() {
    let profile = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let first = open_core(profile.path());
    let second = open_core(profile.path());
    let names = |core: &CoreHandle| -> Vec<String> {
        core.destinations()
            .unwrap()
            .into_iter()
            .map(|d| d.name.to_string())
            .collect()
    };

    first
        .add_destination("x".parse().unwrap(), parent.path().join("x"))
        .unwrap();
    second
        .add_destination("y".parse().unwrap(), parent.path().join("y"))
        .unwrap();
    assert_eq!(names(&first), ["x", "y"]);
    second.remove_destination(&"y".parse().unwrap()).unwrap();
    first
        .add_destination("z".parse().unwrap(), parent.path().join("z"))
        .unwrap();
    assert_eq!(names(&second), ["x", "z"]);
    // A name another handle added is known to this one's operations too.
    second.remove_destination(&"z".parse().unwrap()).unwrap();
    drop((first, second));
    assert_eq!(names(&open_core(profile.path())), ["x"]);
}

/// The registrations of several handles racing on one profile (several
/// processes, as far as the registry can tell) all persist.
#[test]
fn concurrent_registrations_through_separate_handles_all_persist() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    std::thread::scope(|scope| {
        for handle in 0..4 {
            let core = open_core(profile.path());
            let vault = vault.path();
            scope.spawn(move || {
                for i in 0..4 {
                    let name = format!("h{handle}-{i}");
                    core.add_destination(name.parse().unwrap(), vault.join(&name))
                        .unwrap();
                }
            });
        }
    });
    assert_eq!(open_core(profile.path()).destinations().unwrap().len(), 16);
}

/// The seeded-destinations record is facade-owned like the registry.
#[test]
fn the_facade_refuses_its_reserved_seeded_destinations_setting() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    assert!(matches!(
        core.set_setting("destinations.seeded", "{}"),
        Err(curio_core::CoreError::ReservedSettingKey { .. })
    ));
    assert!(matches!(
        core.get_setting("destinations.seeded"),
        Err(curio_core::CoreError::ReservedSettingKey { .. })
    ));
}

/// The exported-destinations record is facade-owned like the registry.
#[test]
fn the_facade_refuses_its_reserved_exported_destinations_setting() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    assert!(matches!(
        core.set_setting("destinations.exported", "[]"),
        Err(curio_core::CoreError::ReservedSettingKey { .. })
    ));
    assert!(matches!(
        core.get_setting("destinations.exported"),
        Err(curio_core::CoreError::ReservedSettingKey { .. })
    ));
}

/// The retention sweep is facade-owned (single-writer event log): expired
/// files go, the live file stays.
#[test]
fn sweep_event_retention_removes_only_expired_files() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    // Today's file exists because add_feed emits feed.added.
    core.add_feed(NewFeed {
        url: "https://example.com/feed.xml".to_owned(),
        title: None,
        tags: Vec::new(),
    })
    .unwrap();
    let events_dir = profile.path().join(".curio/events");
    let expired = events_dir.join("events-20200101.jsonl");
    std::fs::write(&expired, "").unwrap();
    let foreign = events_dir.join("notes.txt");
    std::fs::write(&foreign, "not a log file").unwrap();

    let removed = core.sweep_event_retention().unwrap();
    assert_eq!(removed, vec![expired.clone()]);
    assert!(!expired.exists(), "expired log swept");
    assert!(foreign.exists(), "foreign files are ignored");
    let survivors: Vec<_> = std::fs::read_dir(&events_dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with("events-"))
        .collect();
    assert_eq!(survivors.len(), 1, "today's file survives: {survivors:?}");

    // Idempotent: nothing left to sweep.
    assert_eq!(
        core.sweep_event_retention().unwrap(),
        [] as [std::path::PathBuf; 0]
    );
}

/// The facade serves backend-owned unread counts — heads render the map,
/// they never re-derive badge math client-side.
#[test]
fn unread_counts_are_served_by_the_facade() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    core.storage()
        .upsert_articles(vec![
            manual_article("k1", "One"),
            manual_article("k2", "Two"),
        ])
        .unwrap();
    let articles = core.list_articles(ListArticles::default()).unwrap();
    assert!(core.mark_read(articles[0].id, true).unwrap());
    let counts = core.unread_counts().unwrap();
    assert_eq!(counts.get(&None), Some(&1));
    assert_eq!(counts.values().sum::<u64>(), 1);
}

#[tokio::test]
async fn hydrate_article_replaces_the_excerpt_with_the_extracted_full_body() {
    let server = MockServer::start().await;
    let page = r"<html><body>
        <nav>menu · menu · menu</nav>
        <article>
          <h1>Full Title</h1>
          <p>The complete article body that the feed only excerpted, written with
             enough genuine sentences that the readability scorer locks onto this
             container as the densest, most meaningful block of text on the page.</p>
          <p>A second substantial paragraph continues the piece so the extraction is
             unambiguous and comfortably longer than the short feed excerpt it
             replaces, which is exactly the point of loading the full article.</p>
        </article>
        <footer>footer chrome and related links</footer></body></html>";
    Mock::given(method("GET"))
        .and(path("/post"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    // The feed carries the W1 exemption (its content lives on 127.0.0.1); the
    // article's source page is on the same host, so hydrate is allowed.
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    let mut article = manual_article("k1", "Short excerpt");
    article.feed_id = Some(feed.id);
    article.source_url = format!("{}/post", server.uri());
    core.storage().upsert_articles(vec![article]).unwrap();
    let stored = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);
    assert!(stored.content.text.len() < 20, "starts as a tiny excerpt");

    let hydrated = core.hydrate_article(stored.id).await.unwrap();
    assert!(
        hydrated.content.text.contains("complete article body"),
        "extracted the full body: {}",
        hydrated.content.text
    );
    assert!(
        hydrated
            .content
            .html
            .contains("second substantial paragraph")
    );
    assert!(
        !hydrated.content.html.contains("<nav"),
        "chrome stripped by extract + sanitize"
    );
    assert!(hydrated.word_count.unwrap_or(0) > 20);
}

/// End-to-end proof of the `enrich-reddit` seam: hydrating a
/// reddit-shaped article hits the post's `.json` endpoint (not the HTML
/// page), and the stored content is the sanitized full selftext.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn hydrating_a_reddit_post_uses_the_json_enrichment() {
    let server = MockServer::start().await;
    let post_json = include_str!("../../../fixtures/feeds/reddit_post.json");
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/abc123/title.json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(post_json, "application/json"))
        .expect(1)
        .mount(&server)
        .await;
    // The HTML page must never be fetched when enrichment succeeds.
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/abc123/title/"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let mut article = manual_article("r1", "Reddit stub");
    article.source_url = format!("{}/r/rust/comments/abc123/title/", server.uri());
    core.storage().upsert_articles(vec![article]).unwrap();
    let stored = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);

    let hydrated = core.hydrate_article(stored.id).await.unwrap();
    assert!(
        hydrated.content.text.contains("cut compile times in half"),
        "full selftext stored: {}",
        hydrated.content.text
    );
    assert!(
        hydrated
            .content
            .html
            .contains("https://preview.redd.it/buildchart.png"),
        "preview image inlined: {}",
        hydrated.content.html
    );
    assert!(!hydrated.content.html.contains("<script"));
}

/// BYO Reddit OAuth (D15) end-to-end: with credentials installed, the
/// core trades them for a bearer ONCE (the token is cached), and every
/// post fetch goes to the API origin carrying `Authorization: Bearer` —
/// the mocks only answer requests that actually carry it.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn authenticated_reddit_enrichment_grants_once_and_sends_the_bearer() {
    use wiremock::matchers::header;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/access_token"))
        // The grant carries HTTP Basic credentials.
        .and(wiremock::matchers::header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"access_token":"tok123","token_type":"bearer","expires_in":3600}"#,
            "application/json",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let post_json = include_str!("../../../fixtures/feeds/reddit_post.json");
    for comments_id in ["auth1", "auth2"] {
        Mock::given(method("GET"))
            .and(path(format!("/r/rust/comments/{comments_id}/title.json")))
            .and(header("authorization", "Bearer tok123"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(post_json, "application/json"))
            .expect(1)
            .mount(&server)
            .await;
    }

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let config = curio_core::enrich::reddit_auth::RedditApiConfig {
        client_id: "cid".to_owned(),
        client_secret: "s3cr3t-value".to_owned(),
        token_url: format!("{}/api/v1/access_token", server.uri()),
        api_origin: server.uri(),
    };
    // The secret never reaches a `{:?}` of the config (log lines,
    // panic messages)…
    let config_debug = format!("{config:?}");
    assert!(
        !config_debug.contains("s3cr3t-value"),
        "secret leaked: {config_debug}"
    );
    core.set_reddit_api(Some(config));
    assert_eq!(core.reddit_api_client_id().as_deref(), Some("cid"));

    let mk = |key: &str, comments_id: &str| {
        let mut article = manual_article(key, "Reddit stub");
        article.source_url = format!("{}/r/rust/comments/{comments_id}/title/", server.uri());
        article
    };
    core.storage()
        .upsert_articles(vec![mk("k1", "auth1"), mk("k2", "auth2")])
        .unwrap();
    let ids: Vec<_> = core
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();

    // Both hydrates enrich through the authenticated endpoint; the
    // expect(1) on the token mock proves the bearer was granted once.
    for id in ids {
        let hydrated = core.hydrate_article(id).await.unwrap();
        assert!(
            hydrated.content.text.contains("cut compile times in half"),
            "{}",
            hydrated.content.text
        );
    }

    // …nor does it — or the live, cached bearer — reach a `{:?}` of the
    // handle holding both.
    let core_debug = format!("{core:?} {core:#?}");
    assert!(core_debug.contains("<redacted>"), "{core_debug}");
    assert!(
        !core_debug.contains("s3cr3t-value"),
        "secret leaked: {core_debug}"
    );
    assert!(
        !core_debug.contains("tok123"),
        "bearer leaked: {core_debug}"
    );

    // Clearing credentials reports unconfigured again.
    core.set_reddit_api(None);
    assert!(core.reddit_api_client_id().is_none());
}

/// Reddit's 2026 lockdown 403-blocks anonymous .json requests. That is
/// deterministic, so the answer is guidance (add credentials), not a
/// junk fallback fetch of the same host's HTML shell.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn an_anonymous_reddit_403_asks_for_credentials_without_a_fallback_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/locked1/title.json"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;
    // The HTML fallback must never fire.
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/locked1/title/"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let mut article = manual_article("k1", "Reddit stub");
    article.source_url = format!("{}/r/rust/comments/locked1/title/", server.uri());
    core.storage().upsert_articles(vec![article]).unwrap();
    let stored = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);

    let err = core.hydrate_article(stored.id).await.unwrap_err();
    assert!(
        matches!(
            err,
            curio_core::CoreError::RedditAuth {
                reason: curio_core::RedditAuthReason::CredentialsMissing,
                status: 403,
            }
        ),
        "{err:?}"
    );
    assert!(
        err.to_string()
            .contains("add your own free Reddit API credentials"),
        "{err}"
    );
}

/// A reddit post article on `server`, stored in `core`; returns its id.
#[cfg(feature = "enrich-reddit")]
fn stored_reddit_post(
    core: &CoreHandle,
    server: &MockServer,
    comments_id: &str,
) -> curio_core::model::ArticleId {
    let mut article = manual_article(comments_id, "Reddit stub");
    article.source_url = format!("{}/r/rust/comments/{comments_id}/title/", server.uri());
    core.storage().upsert_articles(vec![article]).unwrap();
    core.list_articles(ListArticles::default())
        .unwrap()
        .remove(0)
        .id
}

/// Installed credentials the token endpoint refuses (a wrong secret, a
/// deleted app) degrade to the anonymous request — and when reddit blocks
/// that too, the user is told to fix their credentials, not to add some.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn refused_reddit_credentials_are_typed_as_rejected_not_missing() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/access_token"))
        .respond_with(ResponseTemplate::new(401))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/badcreds/title.json"))
        .respond_with(ResponseTemplate::new(403))
        .expect(1)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    core.set_reddit_api(Some(curio_core::enrich::reddit_auth::RedditApiConfig {
        client_id: "cid".to_owned(),
        client_secret: "wrong".to_owned(),
        token_url: format!("{}/api/v1/access_token", server.uri()),
        api_origin: server.uri(),
    }));
    let id = stored_reddit_post(&core, &server, "badcreds");

    let err = core.hydrate_article(id).await.unwrap_err();
    assert!(
        matches!(
            err,
            curio_core::CoreError::RedditAuth {
                reason: curio_core::RedditAuthReason::CredentialsRejected,
                status: 401,
            }
        ),
        "{err:?}"
    );
    let message = err.to_string();
    assert!(
        message.contains("check the client id and secret"),
        "{message}"
    );
    assert!(!message.contains("add your own"), "{message}");
}

/// Credentials that grant a token reddit then refuses on the post itself:
/// typed as a revoked token (dropped, so the next call re-grants), not as
/// missing or wrong credentials.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn a_refused_reddit_bearer_is_typed_as_a_revoked_token_and_re_grants() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v1/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"{"access_token":"tok-revoked","token_type":"bearer","expires_in":3600}"#,
            "application/json",
        ))
        // Granted once, dropped on the 401, granted again on the retry.
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/revoked/title.json"))
        .respond_with(ResponseTemplate::new(401))
        .expect(2)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    core.set_reddit_api(Some(curio_core::enrich::reddit_auth::RedditApiConfig {
        client_id: "cid".to_owned(),
        client_secret: "s3cr3t".to_owned(),
        token_url: format!("{}/api/v1/access_token", server.uri()),
        api_origin: server.uri(),
    }));
    let id = stored_reddit_post(&core, &server, "revoked");

    for _ in 0..2 {
        let err = core.hydrate_article(id).await.unwrap_err();
        assert!(
            matches!(
                err,
                curio_core::CoreError::RedditAuth {
                    reason: curio_core::RedditAuthReason::TokenRevoked,
                    status: 401,
                }
            ),
            "{err:?}"
        );
        let message = err.to_string();
        assert!(message.contains("revoked"), "{message}");
        assert!(message.contains("HTTP 401"), "{message}");
    }
}

/// A reddit 429 trips the breaker at once (honoring Retry-After), makes
/// NO fallback fetch of the HTML page, and every hydrate while the
/// breaker is open fails fast without touching the network — the
/// rate-limit answer.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn a_reddit_429_opens_the_breaker_and_stops_all_reddit_fetches() {
    let server = MockServer::start().await;
    // Exactly ONE request ever reaches the JSON endpoint.
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/limit1/title.json"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .expect(1)
        .mount(&server)
        .await;
    // The HTML fallback must never fire — it would be one more request
    // to the very host that just throttled us.
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/limit1/title/"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let mk = |key: &str, comments_id: &str| {
        let mut article = manual_article(key, "Reddit stub");
        article.source_url = format!("{}/r/rust/comments/{comments_id}/title/", server.uri());
        article
    };
    core.storage()
        .upsert_articles(vec![mk("k1", "limit1"), mk("k2", "limit2")])
        .unwrap();
    let ids: Vec<_> = core
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();

    // First hydrate: hits the endpoint once, gets 429, surfaces the
    // rate limit with the server's Retry-After (120s → "2 min").
    let err = core.hydrate_article(ids[1]).await.unwrap_err();
    assert!(
        matches!(err, curio_core::CoreError::RateLimited { .. }),
        "{err:?}"
    );
    assert!(err.to_string().contains("2 min"), "{err}");

    // Second hydrate (different post): breaker open → fails fast,
    // zero network (the mocks' expect() counts prove it).
    let err = core.hydrate_article(ids[0]).await.unwrap_err();
    assert!(matches!(err, curio_core::CoreError::RateLimited { .. }));
}

#[tokio::test]
async fn bulk_export_writes_every_matching_note_idempotently() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();

    core.storage()
        .upsert_articles(vec![
            manual_article("k1", "First note"),
            manual_article("k2", "Second note"),
            manual_article("k3", "Third note"),
        ])
        .unwrap();
    let ids: Vec<_> = core
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    core.set_read_later(ids[0], true).unwrap();

    // Scoped export first: only the read-later article lands.
    let later_only = core
        .save_all_to_destination(
            &ListArticles {
                read_later: Some(true),
                ..ListArticles::default()
            },
            &dest,
        )
        .unwrap();
    assert_eq!(later_only.created, 1);
    assert_eq!(later_only.total(), 1);

    // Export everything: the remaining two are created, the exported one
    // is an idempotency hit.
    let all = core
        .save_all_to_destination(&ListArticles::default(), &dest)
        .unwrap();
    assert_eq!(all.created, 2);
    assert_eq!(all.unchanged, 1);

    // Re-running the full export rewrites nothing.
    let again = core
        .save_all_to_destination(&ListArticles::default(), &dest)
        .unwrap();
    assert_eq!(again.created, 0);
    assert_eq!(again.updated, 0);
    assert_eq!(again.unchanged, 3);

    let notes: Vec<_> = std::fs::read_dir(vault.path().join("curio"))
        .unwrap()
        .collect();
    assert_eq!(notes.len(), 3, "one markdown note per article");

    let missing: DestinationName = "nowhere".parse().unwrap();
    let err = core
        .save_all_to_destination(&ListArticles::default(), &missing)
        .unwrap_err();
    assert!(matches!(
        err,
        curio_core::CoreError::UnknownDestination { .. }
    ));
}

#[tokio::test]
async fn full_text_mode_hydrates_new_articles_at_refresh() {
    let server = MockServer::start().await;
    let feed_xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel>
        <title>Excerpts</title><link>{0}</link>
        <item><guid>p1</guid><title>Post One</title><link>{0}/post</link>
        <description>Just a stub excerpt.</description></item>
        </channel></rss>"#,
        server.uri()
    );
    let page = r"<html><body><article>
        <h1>Post One</h1>
        <p>The full body of the post that the feed only excerpted, with enough
           genuine running prose that the readability scorer confidently keeps
           this container as the main content of the fetched page.</p>
        <p>A second paragraph makes the extraction unambiguous and pushes the
           word count well past the one-line stub the feed shipped.</p>
        </article></body></html>";
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed_xml, "application/rss+xml"))
        .mount(&server)
        .await;
    // expect(1): re-refreshing must NOT re-hydrate the already-known item.
    Mock::given(method("GET"))
        .and(path("/post"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .expect(1)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    core.set_feed_full_text(feed.id, true).unwrap();
    assert!(core.get_feed(feed.id).unwrap().unwrap().fetch_full_text);

    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.new_articles, 1);
    let article = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);
    assert!(
        article.content.text.contains("full body of the post"),
        "hydrated at refresh: {}",
        article.content.text
    );

    // A second refresh sees no new items and fetches no pages (expect(1))
    // — and must not revert the hydrated body to the feed's stub excerpt
    // (#111), nor count the unchanged, re-listed item as updated.
    let second = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((second.new_articles, second.updated_articles), (0, 0));
    let after = core.get_article(article.id).unwrap().unwrap();
    assert_eq!(after.content, article.content, "full text survives refresh");
    assert_eq!(after.word_count, article.word_count);
}

/// A page the readability scorer confidently extracts, longer than any
/// feed stub.
fn full_page(title: &str) -> String {
    format!(
        r"<html><body><article>
        <h1>{title}</h1>
        <p>The full body of the post that the feed only excerpted, with enough
           genuine running prose that the readability scorer confidently keeps
           this container as the main content of the fetched page.</p>
        <p>A second paragraph makes the extraction unambiguous and pushes the
           word count well past the one-line stub the feed shipped.</p>
        </article></body></html>"
    )
}

/// #130: a feed reusing one guid for three different posts stores three
/// articles (not one, overwritten twice) and full-text mode hydrates each
/// page exactly once — no duplicate hydrations queued for a shared key.
#[tokio::test]
async fn a_guid_reused_across_items_stores_each_and_hydrates_each_once() {
    let server = MockServer::start().await;
    let feed_xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel>
        <title>Reused</title><link>{0}</link>
        <item><guid>same</guid><title>A</title><link>{0}/a</link><description>stub</description></item>
        <item><guid>same</guid><title>B</title><link>{0}/b</link><description>stub</description></item>
        <item><guid>same</guid><title>C</title><link>{0}/c</link><description>stub</description></item>
        </channel></rss>"#,
        server.uri()
    );
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed_xml, "application/rss+xml"))
        .mount(&server)
        .await;
    for post in ["a", "b", "c"] {
        Mock::given(method("GET"))
            .and(path(format!("/{post}")))
            .respond_with(ResponseTemplate::new(200).set_body_raw(full_page(post), "text/html"))
            .expect(1)
            .mount(&server)
            .await;
    }

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    core.set_feed_full_text(feed.id, true).unwrap();

    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.new_articles, 3);
    let mut titles: Vec<String> = core
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .map(|article| article.title)
        .collect();
    titles.sort();
    assert_eq!(titles, ["A", "B", "C"]);

    // Stable across refreshes: the same three keys, nothing new or changed.
    let second = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((second.new_articles, second.updated_articles), (0, 0));
}

/// #130: whether a guid counts as reused is decided per fetch, but an item
/// keeps one article as the guid's sharing changes between fetches — no
/// duplicate rows, and read state stays put. The first fetch lists one item
/// under the guid, so it is stored under the plain `guid:` key, as every
/// reused-guid feed was before `guid+link:` keys existed.
#[tokio::test]
async fn a_reused_guid_keeps_each_item_one_article_as_its_sharing_changes() {
    let server = MockServer::start().await;
    let serve = |posts: &[&str]| {
        let items = posts
            .iter()
            .map(|post| {
                format!(
                    "<item><guid>same</guid><title>{}</title><link>{}/{post}</link>\
                     <description>stub</description></item>",
                    post.to_uppercase(),
                    server.uri()
                )
            })
            .collect::<Vec<_>>()
            .concat();
        let xml = format!(
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Reused</title>{items}</channel></rss>"#
        );
        Mock::given(method("GET"))
            .and(path("/feed.xml"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(xml, "application/rss+xml"))
    };
    let titles = |core: &CoreHandle| {
        let mut articles: Vec<(String, bool)> = core
            .list_articles(ListArticles::default())
            .unwrap()
            .into_iter()
            .map(|article| (article.title, core.article_state(article.id).unwrap().read))
            .collect();
        articles.sort();
        articles
    };

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));

    serve(&["c"]).mount(&server).await;
    assert_eq!(core.refresh_feed(feed.id).await.unwrap().new_articles, 1);
    let c = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);
    core.mark_read(c.id, true).unwrap();

    // The guid becomes shared: c keeps its row (and read state).
    server.reset().await;
    serve(&["a", "b", "c"]).mount(&server).await;
    let shared = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((shared.new_articles, shared.updated_articles), (2, 0));
    assert_eq!(
        titles(&core),
        [("A".into(), false), ("B".into(), false), ("C".into(), true)]
    );

    // Unshared again, by an item other than the plain row's: b keeps its
    // own row and never overwrites c's.
    server.reset().await;
    serve(&["b"]).mount(&server).await;
    let alone = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((alone.new_articles, alone.updated_articles), (0, 0));

    // And shared once more: still the same three articles.
    server.reset().await;
    serve(&["a", "b", "c"]).mount(&server).await;
    let again = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((again.new_articles, again.updated_articles), (0, 0));
    assert_eq!(
        titles(&core),
        [("A".into(), false), ("B".into(), false), ("C".into(), true)]
    );
    let c_now = core
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .find(|article| article.title == "C")
        .unwrap();
    assert_eq!(c_now.id, c.id, "c is still the row it was first stored as");
}

/// #130: an item without a link is stored under the feed URL (the contract
/// needs a source URI) but is never hydrated against it — not at refresh
/// in full-text mode, and not on an explicit load-full.
#[tokio::test]
async fn a_link_less_item_is_never_hydrated_against_the_feed_url() {
    let server = MockServer::start().await;
    let feed_xml = r#"<?xml version="1.0"?><rss version="2.0"><channel>
        <title>Link-less</title>
        <item><guid>n1</guid><title>Note</title><description>The whole note.</description></item>
        </channel></rss>"#;
    // expect(1): only the refresh itself may fetch the feed URL.
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed_xml, "application/rss+xml"))
        .expect(1)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed_url = format!("{}/feed.xml", server.uri());
    let feed = subscribe(&core, &feed_url);
    core.set_feed_full_text(feed.id, true).unwrap();

    assert_eq!(core.refresh_feed(feed.id).await.unwrap().new_articles, 1);
    let article = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);
    assert_eq!(article.source_url, feed_url);
    assert_eq!(article.content.text.trim(), "The whole note.");

    let after = core.hydrate_article(article.id).await.unwrap();
    assert_eq!(after.content, article.content, "nothing to load: unchanged");
    server.verify().await;
}

/// #130: a stored article whose fetch lands on its feed's URL (e.g. a
/// link-less entry stored under the feed's pre-redirect URL) keeps its
/// body — the feed XML is never extracted as the article.
#[tokio::test]
async fn a_hydrate_that_lands_on_the_feed_url_changes_nothing() {
    let server = MockServer::start().await;
    let feed_url = format!("{}/feed.xml", server.uri());
    Mock::given(method("GET"))
        .and(path("/old-feed.xml"))
        .respond_with(ResponseTemplate::new(301).insert_header("location", feed_url.as_str()))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(full_page("feed"), "text/html"))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &feed_url);
    let mut stale = manual_article("guid:n1", "Note");
    stale.feed_id = Some(feed.id);
    stale.source_url = format!("{}/old-feed.xml", server.uri());
    core.storage().upsert_articles(vec![stale]).unwrap();
    let article = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);

    let after = core.hydrate_article(article.id).await.unwrap();
    assert_eq!(after.content, article.content);
}

/// A feed that only ships an excerpt of one post whose source page lives
/// at `post_path` on the mock server.
fn excerpt_feed(server: &MockServer, post_path: &str) -> String {
    format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel>
        <title>Excerpts</title><link>{0}</link>
        <item><guid>p1</guid><title>Post One</title><link>{0}{post_path}</link>
        <description>Just a stub excerpt.</description></item>
        </channel></rss>"#,
        server.uri()
    )
}

/// #111: a manual load-full, then a refresh of the same (unchanged) feed
/// XML. The hydrated body must survive, the refresh must change nothing,
/// and the bulk export afterwards must be a pure idempotency hit — no
/// rewritten note, no `article.updated` carrying the excerpt to the vault.
#[tokio::test]
async fn hydrated_full_text_survives_refresh_and_re_export_is_unchanged() {
    let server = MockServer::start().await;
    let page = r"<html><body><article>
        <h1>Post One</h1>
        <p>The full body of the post that the feed only excerpted, with enough
           genuine running prose that the readability scorer confidently keeps
           this container as the main content of the fetched page.</p>
        <p>A second paragraph makes the extraction unambiguous and pushes the
           word count well past the one-line stub the feed shipped.</p>
        </article></body></html>";
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(excerpt_feed(&server, "/post"), "application/rss+xml"),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/post"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .expect(1)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();

    assert_eq!(core.refresh_feed(feed.id).await.unwrap().new_articles, 1);
    let stub = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);
    let hydrated = core.hydrate_article(stub.id).await.unwrap();
    assert!(hydrated.content.text.contains("full body of the post"));
    let first = core.save_to_destination(hydrated.id, &dest).unwrap();
    assert_eq!(first.disposition, ExportDisposition::Created);

    let refresh = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((refresh.new_articles, refresh.updated_articles), (0, 0));
    let after = core.get_article(hydrated.id).unwrap().unwrap();
    assert_eq!(
        after.content, hydrated.content,
        "full text survives refresh"
    );
    assert_eq!(after.word_count, hydrated.word_count);
    // The FTS index still serves the hydrated body.
    assert_eq!(core.search_articles("readability", 5).unwrap().len(), 1);
    core.fts_integrity_check().unwrap();

    let bulk = core
        .save_all_to_destination(&ListArticles::default(), &dest)
        .unwrap();
    assert_eq!((bulk.created, bulk.updated, bulk.unchanged), (0, 0, 1));
    let events = read_all(&profile.path().join(".curio/events")).unwrap();
    assert!(
        events
            .iter()
            .all(|e| e.event.event_type() != "article.updated"),
        "no article.updated after an unchanged refresh"
    );
}

/// #111 for the enrichment path: a reddit post's enriched full selftext
/// survives a refresh of the feed that still lists the post.
#[cfg(feature = "enrich-reddit")]
#[tokio::test]
async fn enriched_reddit_body_survives_a_refresh_of_its_feed() {
    let server = MockServer::start().await;
    let post_path = "/r/rust/comments/abc123/title/";
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(excerpt_feed(&server, post_path), "application/rss+xml"),
        )
        .mount(&server)
        .await;
    let post_json = include_str!("../../../fixtures/feeds/reddit_post.json");
    Mock::given(method("GET"))
        .and(path("/r/rust/comments/abc123/title.json"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(post_json, "application/json"))
        .expect(1)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let feed = subscribe(&core, &format!("{}/feed.xml", server.uri()));
    core.refresh_feed(feed.id).await.unwrap();
    let stub = core
        .list_articles(ListArticles::default())
        .unwrap()
        .remove(0);
    let enriched = core.hydrate_article(stub.id).await.unwrap();
    assert!(enriched.content.text.contains("cut compile times in half"));

    let refresh = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!((refresh.new_articles, refresh.updated_articles), (0, 0));
    let after = core.get_article(enriched.id).unwrap().unwrap();
    assert_eq!(after.content, enriched.content, "enriched body survives");
    core.fts_integrity_check().unwrap();
}

#[tokio::test]
async fn save_url_clips_a_page_into_read_later_with_its_own_metadata() {
    let server = MockServer::start().await;
    let page = r#"<html><head>
        <title>Clipped Page</title>
        <meta property="og:image" content="https://cdn.example.com/lead.jpg">
        <meta name="author" content="Jane Writer">
        </head><body>
        <nav>menu · menu · menu</nav>
        <article>
          <h1>Clipped Page</h1>
          <p>The body of the page being clipped straight from a URL, written with
             enough genuine prose that the readability scorer selects this region
             as the main content over the navigation chrome around it.</p>
          <p>A second substantial paragraph keeps the extraction unambiguous and
             gives the word counter something honest to measure for the note.</p>
        </article></body></html>"#;
    Mock::given(method("GET"))
        .and(path("/clip"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(page, "text/html"))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let url = format!("{}/clip", server.uri());
    let saved = core.save_url(&url, vec!["clips".to_owned()]).await.unwrap();

    assert!(saved.created);
    assert!(saved.hydrated);
    assert_eq!(saved.article.feed_id, None, "a feedless manual save");
    assert_eq!(saved.article.title, "Clipped Page");
    assert!(saved.article.content.text.contains("body of the page"));
    assert_eq!(
        saved.article.lead_image.as_deref(),
        Some("https://cdn.example.com/lead.jpg"),
        "og:image becomes the lead image"
    );
    let state = core.article_state(saved.article.id).unwrap();
    assert!(state.read_later, "a URL save IS a read-later add");
    assert_eq!(
        core.article_tags(saved.article.id).unwrap(),
        vec!["clips".to_owned()]
    );
}

#[tokio::test]
async fn save_url_of_an_unreachable_page_still_saves_the_bare_link() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/gone"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let url = format!("{}/gone", server.uri());
    let saved = core.save_url(&url, vec![]).await.unwrap();

    assert!(saved.created);
    assert!(!saved.hydrated, "no content, but the link is not lost");
    assert_eq!(saved.article.title, url, "the URL stands in for a title");
    assert_eq!(saved.article.content.text, "");
    assert!(
        core.article_state(saved.article.id).unwrap().read_later,
        "still lands in read-later"
    );
}

#[tokio::test]
async fn save_url_twice_reflags_without_duplicating_or_refetching() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/once"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            "<html><body><article><p>A page fetched exactly once, with prose long
             enough for the extractor to score it as the main content.</p>
             <p>Padding paragraph so readability has two blocks to work with
             and the extraction remains stable across runs.</p></article></body></html>",
            "text/html",
        ))
        .expect(1)
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    let url = format!("{}/once", server.uri());
    let first = core.save_url(&url, vec![]).await.unwrap();
    assert!(first.created);

    // Un-flag, then re-save: the row is reused and re-flagged, not refetched
    // (the mock's expect(1) proves no second GET happened).
    core.set_read_later(first.article.id, false).unwrap();
    let second = core.save_url(&url, vec!["again".to_owned()]).await.unwrap();
    assert!(!second.created);
    assert!(!second.hydrated);
    assert_eq!(second.article.id, first.article.id, "same row");
    assert!(core.article_state(first.article.id).unwrap().read_later);
    assert_eq!(
        core.article_tags(first.article.id).unwrap(),
        vec!["again".to_owned()]
    );
    let all = core.list_articles(ListArticles::default()).unwrap();
    assert_eq!(all.len(), 1, "no duplicate row");
}

#[tokio::test]
async fn save_url_rejects_non_http_input() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core_trusting_localhost(profile.path());
    for bad in ["not a url", "ftp://example.com/x", "javascript:alert(1)"] {
        let err = core.save_url(bad, vec![]).await.unwrap_err();
        assert!(
            matches!(err, curio_core::CoreError::InvalidUrl { .. }),
            "{bad} must be rejected, got {err:?}"
        );
    }
}

// ------------------------------------------- the storage-free facade (#178)
//
// Heads never reach `CoreHandle::storage()` (it only exists under the
// test-support feature); these are the facade methods that replaced it.

#[tokio::test]
async fn feed_lookup_status_metadata_and_fetch_log_go_through_the_facade() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let url = format!("{}/feed.xml", server.uri());
    let feed = subscribe(&core, &url);

    // get_feed_by_url: exact URL match, None otherwise.
    assert_eq!(core.get_feed_by_url(&url).unwrap().unwrap().id, feed.id);
    assert!(
        core.get_feed_by_url("https://nope.test/")
            .unwrap()
            .is_none()
    );

    // set_feed_status: pause, reactivate; unknown id is NotFound.
    core.set_feed_status(feed.id, FeedStatus::Paused).unwrap();
    assert_eq!(
        core.get_feed(feed.id).unwrap().unwrap().status,
        FeedStatus::Paused
    );
    core.set_feed_status(feed.id, FeedStatus::Active).unwrap();
    assert_eq!(
        core.get_feed(feed.id).unwrap().unwrap().status,
        FeedStatus::Active
    );
    let unknown = curio_core::model::FeedId(feed.id.0 + 1000);
    assert!(matches!(
        core.set_feed_status(unknown, FeedStatus::Paused),
        Err(curio_core::CoreError::Storage(
            curio_core::storage::StorageError::NotFound { .. }
        ))
    ));

    // update_feed_metadata only fills NULL fields — a human edit wins.
    core.set_feed_title(feed.id, Some("Mine".to_owned()))
        .unwrap();
    core.update_feed_metadata(
        feed.id,
        Some("Parsed".to_owned()),
        Some("https://site.test/".to_owned()),
        Some("About".to_owned()),
    )
    .unwrap();
    let filled = core.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(filled.title.as_deref(), Some("Mine"));
    assert_eq!(filled.site_url.as_deref(), Some("https://site.test/"));
    assert_eq!(filled.description.as_deref(), Some("About"));
    assert!(
        core.update_feed_metadata(unknown, None, None, None)
            .is_err()
    );

    // recent_fetches: empty before a refresh, one record (newest first) after.
    assert_eq!(
        core.recent_fetches(feed.id, 10).unwrap(),
        [] as [curio_core::model::FetchRecord; 0]
    );
    core.refresh_feed(feed.id).await.unwrap();
    let log = core.recent_fetches(feed.id, 10).unwrap();
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].feed_id, feed.id);
    assert_eq!(log[0].http_status, Some(503));
}

#[test]
fn article_count_delete_and_tag_vocabulary_go_through_the_facade() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    assert_eq!(core.count_articles().unwrap(), 0);
    assert_eq!(core.list_tags().unwrap(), [] as [String; 0]);

    core.storage()
        .upsert_articles(vec![
            manual_article("k1", "One"),
            manual_article("k2", "Two"),
        ])
        .unwrap();
    assert_eq!(core.count_articles().unwrap(), 2);

    let articles = core.list_articles(ListArticles::default()).unwrap();
    core.tag_article(articles[0].id, "rust").unwrap();
    core.tag_article(articles[1].id, "async").unwrap();
    assert_eq!(core.list_tags().unwrap(), ["async", "rust"]);

    core.delete_article(articles[0].id).unwrap();
    assert_eq!(core.count_articles().unwrap(), 1);
    assert!(core.get_article(articles[0].id).unwrap().is_none());
    assert!(matches!(
        core.delete_article(articles[0].id),
        Err(curio_core::CoreError::Storage(
            curio_core::storage::StorageError::NotFound { .. }
        ))
    ));
}

#[test]
fn settings_round_trip_through_the_facade_and_survive_a_reopen() {
    let profile = tempfile::tempdir().unwrap();
    {
        let core = open_core(profile.path());
        assert_eq!(core.get_setting("ui.theme").unwrap(), None);
        core.set_setting("ui.theme", "dark").unwrap();
        core.set_setting("ui.theme", "light").unwrap();
        assert_eq!(
            core.get_setting("ui.theme").unwrap().as_deref(),
            Some("light")
        );
    }
    let core = open_core(profile.path());
    assert_eq!(
        core.get_setting("ui.theme").unwrap().as_deref(),
        Some("light")
    );
}

#[test]
fn the_facade_refuses_its_reserved_destinations_setting_both_ways() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let name: DestinationName = "vault".parse().unwrap();
    {
        let core = open_core(profile.path());
        core.add_destination(name.clone(), vault.path().to_path_buf())
            .unwrap();

        let refused_write = core.set_setting("destinations", "{}").unwrap_err();
        assert!(
            matches!(
                &refused_write,
                curio_core::CoreError::ReservedSettingKey { key } if key == "destinations"
            ),
            "{refused_write:?}"
        );
        assert!(refused_write.to_string().contains("facade-owned"));
        assert!(matches!(
            core.get_setting("destinations"),
            Err(curio_core::CoreError::ReservedSettingKey { .. })
        ));
        // The registry the refused write would have clobbered is intact.
        assert_eq!(core.destinations().unwrap().len(), 1);
    }
    // ... and still intact on disk after a reopen.
    let core = open_core(profile.path());
    let registered = core.destinations().unwrap();
    assert_eq!(registered.len(), 1);
    assert_eq!(registered[0].name, name);
}

#[test]
fn database_health_probes_go_through_the_facade() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    assert_eq!(core.db_path(), profile.path().join("curio.db"));
    assert!(core.db_path().is_file());
    assert_eq!(
        core.db_schema_version().unwrap(),
        curio_core::storage::Storage::schema_version()
    );
    core.storage()
        .upsert_articles(vec![manual_article("k1", "One")])
        .unwrap();
    core.integrity_check().unwrap();
    core.fts_integrity_check().unwrap();
    core.vacuum().unwrap();
    core.integrity_check().unwrap();
    // Every facade operation flushes its staged intents before returning.
    let feed = subscribe(&core, "https://example.com/feed.xml");
    core.set_feed_tags(feed.id, vec!["x".to_owned()]).unwrap();
    assert_eq!(core.pending_intent_count().unwrap(), 0);

    // A storage-level state change stages its intent without flushing
    // (the crash window the startup replay closes) — the count sees it.
    let article = core.list_articles(ListArticles::default()).unwrap()[0].id;
    core.storage().star_article(article).unwrap().unwrap();
    assert_eq!(core.pending_intent_count().unwrap(), 1);
    drop(core);
    // Reopening replays the staged intent into the log.
    let core = open_core(profile.path());
    assert_eq!(core.pending_intent_count().unwrap(), 0);
}

/// A hand-edited note that no longer parses stops a bulk export at that
/// article (as the CLI guide documents), even when its content is
/// unchanged: the export never claims a note it cannot account for is
/// intact. Notes already written stay, and once the user repairs (here:
/// deletes) the note, re-running the bulk export completes.
#[tokio::test]
async fn bulk_export_stops_at_a_note_that_no_longer_parses() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();
    core.storage()
        .upsert_articles(vec![
            manual_article("k1", "First note"),
            manual_article("k2", "Second note"),
        ])
        .unwrap();
    let first = core
        .save_all_to_destination(&ListArticles::default(), &dest)
        .unwrap();
    assert_eq!(first.created, 2);

    let garbled = vault.path().join("curio/second-note.md");
    std::fs::write(&garbled, "markers and frontmatter gone\n").unwrap();
    let err = core
        .save_all_to_destination(&ListArticles::default(), &dest)
        .unwrap_err();
    assert!(
        matches!(
            err,
            curio_core::CoreError::Export(curio_core::export::ExportError::NoteParse { .. })
        ),
        "{err:?}"
    );
    assert_eq!(
        std::fs::read_to_string(&garbled).unwrap(),
        "markers and frontmatter gone\n",
        "the unparseable note is never rewritten"
    );
    assert!(vault.path().join("curio/first-note.md").is_file());

    std::fs::remove_file(&garbled).unwrap();
    let repaired = core
        .save_all_to_destination(&ListArticles::default(), &dest)
        .unwrap();
    assert_eq!(repaired.total(), 2);
    assert!(garbled.is_file(), "the deleted note is healed");
}

/// Makes the profile's events dir unwritable for every user (root
/// included, unlike a permission bit): a regular file squats its path.
fn break_events_dir(profile: &std::path::Path) -> std::path::PathBuf {
    let events_dir = profile.join(".curio/events");
    std::fs::remove_dir_all(&events_dir).unwrap();
    std::fs::write(&events_dir, b"not a directory").unwrap();
    events_dir
}

fn repair_events_dir(events_dir: &std::path::Path) {
    std::fs::remove_file(events_dir).unwrap();
    std::fs::create_dir(events_dir).unwrap();
}

/// #118: a failed flush must not turn a committed change into a reported
/// failure. The intent stays staged, the health flag goes up, and the
/// next emission drains it.
#[test]
fn a_failed_event_flush_is_not_a_failed_mutation() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    core.storage()
        .upsert_articles(vec![manual_article("k1", "One")])
        .unwrap();
    let article = core.list_articles(ListArticles::default()).unwrap()[0].id;
    let events_dir = break_events_dir(profile.path());
    assert!(!core.has_pending_events());

    assert!(
        core.set_starred(article, true).unwrap(),
        "the star persisted"
    );
    assert!(core.article_state(article).unwrap().starred);
    assert_eq!(core.pending_intent_count().unwrap(), 1);
    assert!(core.has_pending_events());
    // A user retry is an idempotent no-op — and still not an error.
    assert!(!core.set_starred(article, true).unwrap());
    // The explicit retry reports the failure honestly.
    core.flush_pending_events().unwrap_err();
    assert!(core.has_pending_events());

    repair_events_dir(&events_dir);
    assert_eq!(core.flush_pending_events().unwrap(), 1);
    assert!(!core.has_pending_events());
    assert_eq!(core.pending_intent_count().unwrap(), 0);
    let events = read_all(&events_dir).unwrap();
    assert_eq!(events.len(), 1);
    assert!(matches!(
        events[0].event,
        EventPayload::ArticleStarred { .. }
    ));
}

/// The backlog also drains on the next mutation's own flush, in order.
#[test]
fn the_next_mutation_drains_a_staged_backlog() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    core.storage()
        .upsert_articles(vec![manual_article("k1", "One")])
        .unwrap();
    let article = core.list_articles(ListArticles::default()).unwrap()[0].id;
    let events_dir = break_events_dir(profile.path());
    assert!(core.set_starred(article, true).unwrap());
    assert!(core.has_pending_events());

    repair_events_dir(&events_dir);
    assert!(core.tag_article(article, "rust").unwrap());
    assert!(!core.has_pending_events());
    assert_eq!(core.pending_intent_count().unwrap(), 0);
    let events = read_all(&events_dir).unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].event,
        EventPayload::ArticleStarred { .. }
    ));
    assert!(matches!(
        events[1].event,
        EventPayload::ArticleTagged { .. }
    ));
}

/// #112: an events directory that cannot be opened no longer fails the
/// profile open. The handle comes up with the backlog flag raised, a
/// mutation commits with its intent staged, and once the directory is
/// repaired the next drain opens the log and emits — the intent a
/// previous session left staged first, in order.
#[test]
fn an_unopenable_events_dir_does_not_block_open() {
    let profile = tempfile::tempdir().unwrap();
    let (article, events_dir) = {
        let core = open_core(profile.path());
        core.storage()
            .upsert_articles(vec![manual_article("k1", "One")])
            .unwrap();
        let article = core.list_articles(ListArticles::default()).unwrap()[0].id;
        let events_dir = break_events_dir(profile.path());
        // Left staged by this session: its flush fails.
        assert!(core.set_starred(article, true).unwrap());
        assert_eq!(core.pending_intent_count().unwrap(), 1);
        (article, events_dir)
    };

    let core = open_core(profile.path());
    assert!(core.has_pending_events(), "nothing could be flushed");
    assert!(core.tag_article(article, "rust").unwrap());
    assert_eq!(core.pending_intent_count().unwrap(), 2);
    assert!(core.article_state(article).unwrap().starred);
    core.flush_pending_events().unwrap_err();
    core.sweep_event_retention().unwrap_err();

    repair_events_dir(&events_dir);
    assert_eq!(core.flush_pending_events().unwrap(), 2);
    assert!(!core.has_pending_events());
    let events = read_all(&events_dir).unwrap();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[0].event,
        EventPayload::ArticleStarred { .. }
    ));
    assert!(matches!(
        events[1].event,
        EventPayload::ArticleTagged { .. }
    ));
    // The log stays attached: later mutations emit as usual.
    assert!(core.set_archived(article, true).unwrap());
    assert!(!core.has_pending_events());
    assert_eq!(read_all(&events_dir).unwrap().len(), 3);
    assert_eq!(
        core.sweep_event_retention().unwrap(),
        Vec::<std::path::PathBuf>::new()
    );
}

/// #112: a `.curio/` that cannot hold the events directory at all (a
/// file squats it) still opens the profile; the gitignore and the log
/// appear once it is fixed.
#[test]
fn an_unusable_state_dir_does_not_block_open() {
    let profile = tempfile::tempdir().unwrap();
    let state_dir = profile.path().join(".curio");
    std::fs::write(&state_dir, b"not a directory").unwrap();

    let core = open_core(profile.path());
    assert!(core.has_pending_events());
    core.storage()
        .upsert_articles(vec![manual_article("k1", "One")])
        .unwrap();
    let article = core.list_articles(ListArticles::default()).unwrap()[0].id;
    assert!(core.set_starred(article, true).unwrap());
    assert_eq!(core.pending_intent_count().unwrap(), 1);

    std::fs::remove_file(&state_dir).unwrap();
    assert_eq!(core.flush_pending_events().unwrap(), 1);
    let gitignore = std::fs::read_to_string(state_dir.join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|line| line == "events/"),
        "{gitignore}"
    );
    assert_eq!(read_all(&state_dir.join("events")).unwrap().len(), 1);
}

/// #112: a profile on sync-managed storage is refused before anything is
/// created, with a typed error naming the kind; the explicit override
/// opens it.
#[test]
fn a_profile_on_sync_managed_storage_needs_the_override() {
    use curio_core::CoreError;
    use curio_core::storage::{NetworkFsKind, OpenError};

    let synced = tempfile::tempdir().unwrap();
    std::fs::create_dir(synced.path().join(".stfolder")).unwrap();
    let profile = synced.path().join("curio");

    let err = CoreHandle::open(&profile).unwrap_err();
    match &err {
        CoreError::Open(OpenError::NetworkFs { path, kind }) => {
            assert_eq!(*kind, NetworkFsKind::Syncthing);
            assert_eq!(*path, profile.join("curio.db"));
        }
        other => panic!("expected NetworkFs, got {other:?}"),
    }
    let message = err.to_string();
    assert!(message.contains("Syncthing"), "{message}");
    assert!(message.contains("CURIO_ALLOW_NETWORK_FS"), "{message}");
    assert!(!err.is_profile_busy());
    assert!(!profile.exists(), "a refused open creates nothing");

    let core = CoreHandle::open_with(
        &profile,
        CoreOptions {
            allow_network_fs: true,
            ..CoreOptions::default()
        },
    )
    .unwrap();
    assert!(core.db_path().is_file());
    assert!(!core.has_pending_events());
}
