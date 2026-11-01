//! Refresh sweeps against wiremock fixture feeds on 127.0.0.1: bounded
//! concurrency, per-feed errors as outcomes, one sweep at a time,
//! cancellation, the runtime staying responsive, and the roadmap's
//! 1000-feed budget (#121).

#![allow(clippy::unwrap_used)]

use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use curio_core::fetch::FetchConfig;
use curio_core::model::{FeedId, FetchStatus};
use curio_core::{CoreHandle, CoreOptions, RefreshOptions, SweepReport};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RSS: &str = include_str!("../../../fixtures/feeds/rss2.xml");

/// A core that trusts 127.0.0.1 (the fixtures' host) with no politeness
/// delay and the given request timeout.
fn open_core(profile: &std::path::Path, request_timeout: Duration) -> CoreHandle {
    CoreHandle::open_with(
        profile,
        CoreOptions {
            fetch: FetchConfig {
                politeness_delay: Duration::ZERO,
                connect_timeout: request_timeout,
                request_timeout,
                trusted_addrs: std::iter::once(std::net::IpAddr::from([127, 0, 0, 1])).collect(),
                ..FetchConfig::default()
            },
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

fn rss() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_raw(RSS, "application/rss+xml")
}

fn count(report: &SweepReport, status: FetchStatus) -> usize {
    report
        .outcomes
        .iter()
        .filter(|outcome| outcome.status == status)
        .count()
}

/// Five hung hosts among 200 feeds: in flight together, they cost about
/// one request timeout between them — not five (the sequential sweep
/// took at least 5 × the timeout).
#[tokio::test]
async fn hung_hosts_time_out_together_not_one_after_another() {
    const TIMEOUT: Duration = Duration::from_secs(6);
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("^/ok/"))
        .respond_with(rss())
        .expect(195)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/hung/"))
        .respond_with(rss().set_delay(Duration::from_secs(30)))
        .mount(&server)
        .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), TIMEOUT);
    // The hung feeds are spread through the list, as in real life.
    let paths: Vec<String> = (0..200)
        .map(|i| {
            if i % 40 == 7 {
                format!("/hung/{i}.xml")
            } else {
                format!("/ok/{i}.xml")
            }
        })
        .collect();
    let ids = subscribe_all(&core, &server, &paths);

    let mut progress = Vec::new();
    let started = Instant::now();
    let report = core
        .refresh_all_with(RefreshOptions::default(), |step| {
            progress.push((step.sweep_id, step.done, step.total));
        })
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert!(
        elapsed < TIMEOUT * 3,
        "200 feeds with 5 hung hosts took {elapsed:?} (the timeout is {TIMEOUT:?})"
    );
    assert_eq!(report.total, 200);
    assert_eq!(report.outcomes.len(), 200);
    assert_eq!(count(&report, FetchStatus::Ok), 195);
    assert_eq!(count(&report, FetchStatus::Error), 5);
    // Outcomes come back in the requested order, whatever finished first.
    let reported: Vec<FeedId> = report.outcomes.iter().map(|o| o.feed_id).collect();
    assert_eq!(reported, ids);
    // Progress counted every feed off, 1..=200, under one sweep id.
    assert_eq!(progress.len(), 200);
    assert!(
        progress
            .iter()
            .enumerate()
            .all(|(i, &(sweep, done, total))| sweep == report.sweep_id
                && done == i + 1
                && total == 200)
    );
    assert!(!report.cancelled && !report.joined);
    server.verify().await;
}

/// A feed unsubscribed while the sweep runs is that feed's error
/// outcome; every other feed still refreshes (the old `?` aborted the
/// sweep at it).
#[tokio::test]
async fn a_feed_removed_mid_sweep_does_not_abort_the_sweep() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(rss())
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(5));
    let paths: Vec<String> = (0..5).map(|i| format!("/f/{i}.xml")).collect();
    let ids = subscribe_all(&core, &server, &paths);

    // One at a time, so the removal lands before feed 2 starts.
    let report = core
        .refresh_many(ids.clone(), RefreshOptions { concurrency: 1 }, |step| {
            if step.done == 1 {
                core.remove_feed(ids[2]).unwrap();
            }
        })
        .await;

    assert_eq!(report.outcomes.len(), 5, "{report:#?}");
    for (index, outcome) in report.outcomes.iter().enumerate() {
        assert_eq!(outcome.feed_id, ids[index]);
        if index == 2 {
            assert_eq!(outcome.status, FetchStatus::Error);
            assert!(outcome.error.as_deref().unwrap().contains("removed"));
        } else {
            assert_eq!(outcome.status, FetchStatus::Ok, "{outcome:?}");
            assert_eq!(outcome.new_articles, 3);
        }
    }
}

/// Two sweeps asked for at once (R pressed twice, or the scheduler
/// firing during a manual refresh) run as one: every feed is fetched
/// exactly once, and the second request reports the first one's result.
#[tokio::test]
async fn concurrent_sweep_requests_fetch_each_feed_once() {
    let server = MockServer::start().await;
    let paths: Vec<String> = (0..20).map(|i| format!("/f/{i}.xml")).collect();
    for feed_path in &paths {
        Mock::given(method("GET"))
            .and(path(feed_path.as_str()))
            .respond_with(rss().set_delay(Duration::from_millis(100)))
            .expect(1)
            .mount(&server)
            .await;
    }
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(5));
    subscribe_all(&core, &server, &paths);

    let (first, second) = tokio::join!(
        core.refresh_all_with(RefreshOptions::default(), |_| {}),
        async {
            // Let the first sweep claim the slot.
            tokio::time::sleep(Duration::from_millis(30)).await;
            core.refresh_all_with(RefreshOptions::default(), |_| {
                unreachable!("a joined request reports no progress of its own")
            })
            .await
        }
    );
    let (first, second) = (first.unwrap(), second.unwrap());

    assert!(!first.joined);
    assert!(second.joined);
    assert_eq!(first.sweep_id, second.sweep_id);
    assert_eq!(first.outcomes, second.outcomes);
    assert_eq!(count(&first, FetchStatus::Ok), 20);
    server.verify().await;

    // Once it is over, the next request is a sweep of its own.
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(rss())
        .mount(&server)
        .await;
    let third = core
        .refresh_all_with(RefreshOptions::default(), |_| {})
        .await
        .unwrap();
    assert!(!third.joined);
    assert_eq!(third.sweep_id, first.sweep_id + 1);
}

/// Cancelling stops new fetches and drops the ones in flight at once —
/// nothing recorded for them, and no outcome either: an abandoned feed
/// was not refreshed, it did not fail — instead of waiting out a slow
/// host.
#[tokio::test]
async fn cancel_stops_new_fetches_and_drops_the_ones_in_flight() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f/0.xml"))
        .respond_with(rss())
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/f/"))
        .respond_with(rss().set_delay(Duration::from_secs(20)))
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(30));
    let paths: Vec<String> = (0..30).map(|i| format!("/f/{i}.xml")).collect();
    let ids = subscribe_all(&core, &server, &paths);
    assert!(!core.cancel_refresh(), "nothing to cancel yet");

    let started = Instant::now();
    let mut progressed = 0;
    let report = core
        .refresh_many(ids.clone(), RefreshOptions { concurrency: 2 }, |step| {
            progressed = step.done;
            // The fast feed is done; the second is stuck on a slow host.
            if step.done == 1 {
                assert!(core.cancel_refresh());
            }
        })
        .await;
    assert_eq!(progressed, 1, "the abandoned feed never finished");

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(report.cancelled);
    assert_eq!(report.total, 30);
    assert_eq!(report.outcomes.len(), 1, "{report:#?}");
    assert_eq!(report.outcomes[0].feed_id, ids[0]);
    assert_eq!(report.outcomes[0].status, FetchStatus::Ok);
    assert!(
        core.recent_fetches(ids[1], 10).unwrap().is_empty(),
        "an abandoned fetch records nothing"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert!(!core.cancel_refresh(), "the sweep is over");
}

/// A large feed's parse/sanitize/ingest runs off the async runtime: on a
/// current-thread runtime (the CLI's) a timer task keeps ticking while it
/// is refreshed.
#[tokio::test(flavor = "current_thread")]
async fn a_large_feed_refresh_keeps_a_current_thread_runtime_responsive() {
    let mut items = String::new();
    for i in 0..300 {
        write!(
            items,
            "<item><title>Post {i}</title><link>https://example.com/p/{i}</link>\
             <guid>urn:p:{i}</guid><description><![CDATA[{}]]></description></item>",
            "<p>Some <strong>body</strong> text with <a href=\"/x\">a link</a> and \
             <img src=\"/i.png\" onerror=\"alert(1)\">.</p>"
                .repeat(20)
        )
        .unwrap();
    }
    let feed = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Big</title>{items}</channel></rss>"#
    );
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed, "application/rss+xml"))
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(30));
    let ids = subscribe_all(&core, &server, &["/big.xml".to_owned()]);

    let longest_gap = Arc::new(Mutex::new(Duration::ZERO));
    let ticker = {
        let longest_gap = Arc::clone(&longest_gap);
        tokio::spawn(async move {
            loop {
                let before = Instant::now();
                tokio::time::sleep(Duration::from_millis(5)).await;
                let gap = before.elapsed();
                let mut longest = longest_gap.lock().unwrap();
                *longest = (*longest).max(gap);
            }
        })
    };
    let started = Instant::now();
    let outcome = core.refresh_feed(ids[0]).await.unwrap();
    let took = started.elapsed();
    // Let the ticker take the tick that was due during the refresh — a
    // stalled runtime only shows once the ticker gets to run again.
    tokio::time::sleep(Duration::from_millis(50)).await;
    ticker.abort();

    assert_eq!(outcome.status, FetchStatus::Ok, "{outcome:?}");
    assert_eq!(outcome.new_articles, 300);
    let longest = *longest_gap.lock().unwrap();
    assert!(
        longest < Duration::from_millis(250),
        "the timer stalled for {longest:?} during a {took:?} refresh"
    );
}

/// The roadmap's Phase 2 budget: a 1000-feed wiremock sweep in under
/// 60 s on CI.
#[tokio::test]
async fn a_thousand_feed_sweep_fits_the_ci_budget() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(rss())
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(10));
    let paths: Vec<String> = (0..1000).map(|i| format!("/f/{i}.xml")).collect();
    subscribe_all(&core, &server, &paths);

    let started = Instant::now();
    let report = core
        .refresh_all_with(RefreshOptions::default(), |_| {})
        .await
        .unwrap();
    let elapsed = started.elapsed();

    assert_eq!(count(&report, FetchStatus::Ok), 1000);
    assert!(
        elapsed < Duration::from_secs(60),
        "1000 feeds took {elapsed:?}"
    );
}

/// A sweep whose caller gives up on it half-way (its future dropped)
/// frees the slot: the request that had joined it runs a sweep of its own
/// instead of waiting forever.
#[tokio::test]
async fn a_joined_request_takes_over_when_the_running_sweep_is_dropped() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(rss().set_delay(Duration::from_millis(500)))
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(5));
    let paths: Vec<String> = (0..3).map(|i| format!("/f/{i}.xml")).collect();
    let ids = subscribe_all(&core, &server, &paths);

    let (abandoned, takeover) = tokio::join!(
        tokio::time::timeout(
            Duration::from_millis(150),
            core.refresh_many(ids.clone(), RefreshOptions::default(), |_| {}),
        ),
        async {
            tokio::time::sleep(Duration::from_millis(30)).await;
            core.refresh_many(ids.clone(), RefreshOptions::default(), |_| {})
                .await
        }
    );

    assert!(abandoned.is_err(), "the first sweep was dropped");
    assert!(!takeover.joined, "the joiner ran its own sweep");
    assert_eq!(takeover.sweep_id, 2);
    assert_eq!(count(&takeover, FetchStatus::Ok), 3);
}

/// A cancel that lands while a refreshed feed's full-text pages are being
/// fetched drops the page fetch in flight too — the sweep does not wait
/// out a slow article host. The feed itself was refreshed, so it keeps
/// its outcome; the feed after it never starts.
#[tokio::test]
async fn cancel_drops_a_full_text_page_fetch_in_flight() {
    let server = MockServer::start().await;
    let mut items = String::new();
    for i in 0..3 {
        write!(
            items,
            "<item><title>Post {i}</title><link>{}/page/{i}</link>\
             <guid>urn:p:{i}</guid><description>excerpt {i}</description></item>",
            server.uri()
        )
        .unwrap();
    }
    let feed = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Full</title>{items}</channel></rss>"#
    );
    Mock::given(method("GET"))
        .and(path_regex("^/f/"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(feed, "application/rss+xml"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path_regex("^/page/"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw("<html><body><p>full</p></body></html>", "text/html")
                .set_delay(Duration::from_secs(20)),
        )
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(30));
    let paths: Vec<String> = (0..2).map(|i| format!("/f/{i}.xml")).collect();
    let ids = subscribe_all(&core, &server, &paths);
    core.set_feed_full_text(ids[0], true).unwrap();

    let started = Instant::now();
    let (report, ()) = tokio::join!(
        core.refresh_many(ids.clone(), RefreshOptions { concurrency: 1 }, |_| {}),
        async {
            // Cancel once the first article page is being fetched.
            loop {
                let requests = server.received_requests().await.unwrap();
                if requests.iter().any(|r| r.url.path().starts_with("/page/")) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            assert!(core.cancel_refresh());
        }
    );

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    assert!(report.cancelled);
    assert_eq!(report.outcomes.len(), 1, "{report:#?}");
    assert_eq!(report.outcomes[0].feed_id, ids[0]);
    assert_eq!(report.outcomes[0].status, FetchStatus::Ok);
    assert_eq!(report.outcomes[0].new_articles, 3);
    let requests = server.received_requests().await.unwrap();
    let pages = requests
        .iter()
        .filter(|r| r.url.path().starts_with("/page/"))
        .count();
    assert_eq!(pages, 1, "hydration stopped at the cancel");
    assert!(
        requests.iter().all(|r| r.url.path() != "/f/1.xml"),
        "the second feed never started"
    );
}

/// A cancel that lands after every feed finished stopped nothing: the
/// report is complete and not marked cancelled.
#[tokio::test]
async fn a_cancel_after_the_last_feed_stops_nothing() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(rss())
        .mount(&server)
        .await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path(), Duration::from_secs(5));
    let paths: Vec<String> = (0..3).map(|i| format!("/f/{i}.xml")).collect();
    let ids = subscribe_all(&core, &server, &paths);

    let report = core
        .refresh_many(ids, RefreshOptions::default(), |step| {
            if step.done == step.total {
                assert!(core.cancel_refresh());
            }
        })
        .await;
    assert_eq!(count(&report, FetchStatus::Ok), 3);
    assert!(!report.cancelled, "{report:#?}");
}
