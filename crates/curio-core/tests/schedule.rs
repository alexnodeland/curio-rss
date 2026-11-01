//! Per-feed refresh scheduling against wiremock fixture feeds on
//! 127.0.0.1 (#122): `Retry-After`, failure backoff, auto-pause, the
//! full-text batch's rate-limit stop, the bounded fetch log, and sweeps
//! that fetch only the feeds that are due.

#![allow(clippy::unwrap_used)]

use std::fmt::Write as _;
use std::time::Duration;

use curio_core::fetch::FetchConfig;
use curio_core::model::{Feed, FeedId, FeedStatus, FetchErrorKind, FetchStatus};
use curio_core::storage::FETCH_LOG_KEEP;
use curio_core::{CoreHandle, CoreOptions, RefreshOutcome, RefreshPolicy};
use curio_types::Timestamp;
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A sweep that fetched nothing.
const NOTHING: [RefreshOutcome; 0] = [];

/// A core that trusts 127.0.0.1 (the fixtures' host), with no politeness
/// delay and the given schedule.
fn open_core(profile: &std::path::Path, refresh: RefreshPolicy) -> CoreHandle {
    CoreHandle::open_with(
        profile,
        CoreOptions {
            fetch: FetchConfig {
                politeness_delay: Duration::ZERO,
                trusted_addrs: std::iter::once(std::net::IpAddr::from([127, 0, 0, 1])).collect(),
                ..FetchConfig::default()
            },
            refresh,
            ..CoreOptions::default()
        },
    )
    .unwrap()
}

/// Subscribes to every `paths` entry on `server` in one OPML import and
/// returns the feed ids in subscription order.
fn subscribe_all(core: &CoreHandle, server: &MockServer, paths: &[String]) -> Vec<FeedId> {
    let mut outlines = String::new();
    for (index, feed_path) in paths.iter().enumerate() {
        write!(
            outlines,
            r#"<outline type="rss" text="feed {index}" xmlUrl="{}{feed_path}"/>"#,
            server.uri()
        )
        .unwrap();
    }
    let opml = format!(
        r#"<?xml version="1.0"?><opml version="2.0"><head/><body>{outlines}</body></opml>"#
    );
    assert_eq!(core.import_opml(&opml).unwrap().added, paths.len());
    core.list_feeds()
        .unwrap()
        .into_iter()
        .map(|feed| feed.id)
        .collect()
}

fn feed(core: &CoreHandle, id: FeedId) -> Feed {
    core.get_feed(id).unwrap().unwrap()
}

/// `next_attempt_at − last_fetched_at` — the wait the last attempt set.
fn scheduled_wait(feed: &Feed) -> Duration {
    let next = feed.next_attempt_at.unwrap().unix_millis();
    let fetched = feed.last_fetched_at.unwrap().unix_millis();
    Duration::from_millis(u64::try_from(next - fetched).unwrap())
}

/// Sleeps until the feed's next attempt is due.
async fn wait_until_due(feed: &Feed) {
    let next = feed.next_attempt_at.unwrap().unix_millis();
    let left = next - Timestamp::now().unix_millis();
    if let Ok(left) = u64::try_from(left) {
        tokio::time::sleep(Duration::from_millis(left + 5)).await;
    }
}

/// A 429's `Retry-After` — either form — holds every later fetch of the
/// feed until the deadline: sweeps skip it and so does a manual refresh.
#[tokio::test]
async fn a_429_with_retry_after_is_not_refetched_before_the_deadline() {
    let server = MockServer::start().await;
    let in_two_hours = chrono::Utc::now() + chrono::Duration::hours(2);
    Mock::given(path("/seconds.xml"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "3600"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(path("/date.xml"))
        .respond_with(ResponseTemplate::new(429).insert_header(
            "Retry-After",
            in_two_hours.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), RefreshPolicy::default());
    let ids = subscribe_all(
        &core,
        &server,
        &["/seconds.xml".to_owned(), "/date.xml".to_owned()],
    );

    let first = core.refresh_all().await.unwrap();
    assert_eq!(first.len(), 2);
    for outcome in &first {
        assert_eq!(outcome.http_status, Some(429));
        assert!(
            outcome.error.as_deref().unwrap().contains("rate-limiting"),
            "{outcome:?}"
        );
    }
    let seconds = feed(&core, ids[0]);
    assert_eq!(seconds.last_error_kind, Some(FetchErrorKind::RateLimited));
    assert!(scheduled_wait(&seconds) >= Duration::from_secs(3_600));
    let date = feed(&core, ids[1]);
    let waited = scheduled_wait(&date);
    assert!(
        waited > Duration::from_secs(7_100) && waited <= Duration::from_secs(7_200),
        "the HTTP-date form: {waited:?}"
    );

    // Not before the deadline: the sweep has nothing due …
    assert_eq!(core.refresh_all().await.unwrap(), NOTHING);
    // … and a manual refresh says why instead of hitting the host.
    for id in ids {
        let held = core.refresh_feed(id).await.unwrap();
        assert_eq!(held.status, FetchStatus::Error);
        assert_eq!(held.http_status, None);
        assert!(
            held.error
                .as_deref()
                .unwrap()
                .contains("asked Curio to wait"),
            "{held:?}"
        );
    }
    assert_eq!(
        core.recent_fetches(seconds.id, 10).unwrap().len(),
        1,
        "a held refresh records nothing"
    );
}

/// Five 5xx in a row: each one doubles the wait, and until it has passed
/// a sweep leaves the feed alone. A success ends the run.
#[tokio::test]
async fn five_straight_5xx_back_off_exponentially() {
    let base = Duration::from_millis(40);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(5)
        .expect(5)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(304))
        .expect(1)
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(
        profile.path(),
        RefreshPolicy {
            backoff_base: base,
            ..RefreshPolicy::default()
        },
    );
    let id = subscribe_all(&core, &server, &["/feed.xml".to_owned()])[0];

    let mut expected = base;
    for failures in 1..=5 {
        let swept = core.refresh_all().await.unwrap();
        assert_eq!(swept.len(), 1, "failure {failures} is fetched");
        assert_eq!(swept[0].http_status, Some(503));
        assert_eq!(
            core.refresh_all().await.unwrap(),
            NOTHING,
            "failure {failures}: backing off"
        );
        let state = feed(&core, id);
        assert_eq!(state.consecutive_failures, failures);
        assert_eq!(state.last_error_kind, Some(FetchErrorKind::Server));
        let waited = scheduled_wait(&state);
        assert!(
            waited >= expected && waited <= expected + expected / 10,
            "failure {failures}: waited {waited:?}, expected about {expected:?}"
        );
        expected *= 2;
        wait_until_due(&state).await;
    }

    let recovered = core.refresh_all().await.unwrap();
    assert_eq!(recovered[0].status, FetchStatus::NotModified);
    let state = feed(&core, id);
    assert_eq!(state.consecutive_failures, 0);
    assert_eq!(state.next_attempt_at, None);
    assert_eq!(state.last_error_kind, None);
    assert_eq!(state.failing_since, None);
}

/// N failures in a row (here 3, over no minimum span) pause the feed by
/// themselves; sweeps then skip it until the one-click resume.
#[tokio::test]
async fn n_failures_in_a_row_auto_pause_the_feed_until_resumed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(404))
        .expect(4)
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(
        profile.path(),
        RefreshPolicy {
            backoff_base: Duration::ZERO,
            auto_pause_after: 3,
            auto_pause_min_failing: Duration::ZERO,
            ..RefreshPolicy::default()
        },
    );
    let id = subscribe_all(&core, &server, &["/gone-missing.xml".to_owned()])[0];

    for attempt in 1..=3 {
        assert_eq!(
            core.refresh_all().await.unwrap().len(),
            1,
            "attempt {attempt}"
        );
    }
    let paused = feed(&core, id);
    assert_eq!(paused.status, FeedStatus::Paused);
    assert_eq!(paused.consecutive_failures, 3);
    assert_eq!(paused.last_error_kind, Some(FetchErrorKind::Client));
    assert_eq!(paused.last_error.as_deref(), Some("HTTP 404"));
    assert!(paused.auto_paused_at.is_some());
    assert_eq!(core.refresh_all().await.unwrap(), NOTHING, "paused");

    core.set_feed_status(id, FeedStatus::Active).unwrap();
    let resumed = feed(&core, id);
    assert_eq!(resumed.auto_paused_at, None);
    assert_eq!(resumed.consecutive_failures, 0);
    assert_eq!(core.refresh_all().await.unwrap().len(), 1, "fetched again");
    assert_eq!(feed(&core, id).status, FeedStatus::Active);
}

/// Full-text mode on a generic (non-Reddit) page host that answers 429:
/// the batch stops after the first page instead of requesting every one.
#[tokio::test]
async fn a_full_text_batch_stops_after_a_page_host_429s() {
    let pages = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("Retry-After", "120"))
        .expect(1)
        .mount(&pages)
        .await;
    let mut items = String::new();
    for n in 0..5 {
        write!(
            items,
            "<item><title>post {n}</title><link>{}/post/{n}</link>\
             <guid>post-{n}</guid><description>excerpt {n}</description></item>",
            pages.uri()
        )
        .unwrap();
    }
    let rss = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
<link>{}</link>{items}</channel></rss>"#,
        pages.uri()
    );
    let feeds = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(rss, "application/rss+xml"))
        .mount(&feeds)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), RefreshPolicy::default());
    let id = subscribe_all(&core, &feeds, &["/feed.xml".to_owned()])[0];
    core.set_feed_full_text(id, true).unwrap();
    core.set_feed_allow_private_network(id, true).unwrap();

    let outcome = core.refresh_feed(id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Ok);
    assert_eq!(outcome.new_articles, 5);
    // Every article keeps its feed excerpt.
    assert_eq!(
        core.search_articles("excerpt", 10).unwrap().len(),
        5,
        "excerpts kept"
    );
    // (`expect(1)` on the page host is verified when it drops.)
}

/// A manual load-full against a 429ing host reports the rate limit
/// instead of pretending it worked.
#[tokio::test]
async fn hydrating_against_a_429_reports_the_rate_limit() {
    let pages = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429))
        .mount(&pages)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), RefreshPolicy::default());
    let saved = core
        .save_url(&format!("{}/post", pages.uri()), Vec::new())
        .await
        .unwrap();
    let err = core.hydrate_article(saved.article.id).await.unwrap_err();
    assert!(
        matches!(
            &err,
            curio_core::CoreError::RateLimited { host, retry_after_secs: 60 } if host == "127.0.0.1"
        ),
        "{err:?}"
    );
}

/// 500 refreshes of one feed leave its fetch log at the per-feed cap.
#[tokio::test]
async fn the_fetch_log_stays_bounded_across_500_refreshes() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(304))
        .expect(500)
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), RefreshPolicy::default());
    let id = subscribe_all(&core, &server, &["/feed.xml".to_owned()])[0];
    for _ in 0..500 {
        core.refresh_feed(id).await.unwrap();
    }
    assert_eq!(
        core.recent_fetches(id, 1_000).unwrap().len(),
        usize::try_from(FETCH_LOG_KEEP).unwrap()
    );
}

/// A feed's `<ttl>` spaces its sweeps out.
#[tokio::test]
async fn a_feeds_own_ttl_spaces_its_sweeps() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
<link>https://example.com/</link><ttl>120</ttl>
<item><title>a</title><link>https://example.com/a</link></item></channel></rss>"#,
            "application/rss+xml",
        ))
        .expect(1)
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), RefreshPolicy::default());
    let id = subscribe_all(&core, &server, &["/feed.xml".to_owned()])[0];
    assert_eq!(core.refresh_all().await.unwrap().len(), 1);
    assert_eq!(
        scheduled_wait(&feed(&core, id)),
        Duration::from_secs(120 * 60)
    );
    assert_eq!(core.refresh_all().await.unwrap(), NOTHING);
}

/// 1000 feeds on mixed schedules: a third with a per-feed interval, a
/// third whose server grants `Cache-Control: max-age`, a third with
/// neither. The second sweep fetches only that last third.
#[tokio::test]
async fn a_mixed_interval_thousand_feed_sweep_fetches_only_due_feeds() {
    let server = MockServer::start().await;
    Mock::given(path_regex("^/hinted/"))
        .respond_with(ResponseTemplate::new(304).insert_header("Cache-Control", "max-age=3600"))
        .expect(333)
        .mount(&server)
        .await;
    Mock::given(path_regex("^/interval/"))
        .respond_with(ResponseTemplate::new(304))
        .expect(334)
        .mount(&server)
        .await;
    Mock::given(path_regex("^/plain/"))
        .respond_with(ResponseTemplate::new(304))
        .expect(2 * 333)
        .mount(&server)
        .await;
    let paths: Vec<String> = (0..1000)
        .map(|i| match i % 3 {
            0 => format!("/interval/{i}.xml"),
            1 => format!("/hinted/{i}.xml"),
            _ => format!("/plain/{i}.xml"),
        })
        .collect();
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), RefreshPolicy::default());
    let ids = subscribe_all(&core, &server, &paths);
    for id in ids.iter().step_by(3) {
        core.set_feed_refresh_interval(*id, Some(60)).unwrap();
    }

    let first = core.refresh_all().await.unwrap();
    assert_eq!(first.len(), 1000);
    let second = core.refresh_all().await.unwrap();
    assert_eq!(second.len(), 333);
    let plain: Vec<FeedId> = ids.iter().skip(2).step_by(3).copied().collect();
    assert_eq!(
        second.iter().map(|o| o.feed_id).collect::<Vec<_>>(),
        plain,
        "exactly the feeds with no schedule of their own"
    );
}
