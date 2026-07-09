//! Opt-in LIVE-NETWORK integration tests — these hit the real internet.
//!
//! They are `#[ignore]`d so `just ci` and a bare `cargo test` stay hermetic
//! per the repo's binding rule (no real network in the default suite). Run
//! them on demand:
//!
//! ```text
//! just test-live
//! # or: cargo test -p curio-core --test live_network -- --ignored --nocapture
//! ```
//!
//! Purpose: prove the fetch policy works end-to-end against the sources Alex
//! named — Reddit, YouTube, Hacker News, Mastodon — exercising the real
//! `PolicedClient` defaults (including the reddit.com politeness override)
//! and the real feed parser. Because they touch the live network they can be
//! flaky (rate limits, feed churn); a failure here is a signal to check the
//! endpoint, never a CI gate.

#![allow(clippy::unwrap_used)]
// Prose docs name YouTube/Mastodon etc.; don't demand backticks on them.
#![allow(clippy::doc_markdown)]

use curio_core::feeds::parse_feed;
use curio_core::fetch::{FetchRequest, PolicedClient};

/// Fetch a real feed with production defaults, returning (status, entry count).
async fn fetch_and_parse(url: &str) -> (u16, usize) {
    // Real defaults: honest UA, spec-correct Accept, reddit.com politeness.
    let client = PolicedClient::default();
    let response = client.fetch(&FetchRequest::new(url)).await.unwrap();
    let entries = if response.is_success() {
        parse_feed(&response.body, Some(url)).map_or(0, |feed| feed.entries.len())
    } else {
        0
    };
    (response.status, entries)
}

#[tokio::test]
#[ignore = "hits the real network; run via `just test-live`"]
async fn reddit_rss_fetches_with_the_reddit_override() {
    // Reddit blocks curio at two layers: the rustls TLS fingerprint (a hard
    // 403) and the honest curio UA. The default reddit.com override fixes both
    // — it uses the platform-native TLS stack plus a browser UA (verified
    // live: native-TLS + browser UA -> 200). The only residual failure is a
    // 429 rate-limit under heavy repeated fetching, which normal usage + the
    // 2s politeness delay keep clear of.
    //
    // So a 403 here is a regression (the fix broke); 429 is an acceptable
    // rate-limit; 200 must parse into entries.
    let (status, entries) = fetch_and_parse("https://www.reddit.com/r/rust/.rss").await;
    assert_ne!(
        status, 403,
        "the native-TLS + browser-UA override must defeat Reddit's 403"
    );
    assert!(
        matches!(status, 200 | 429),
        "expected 200 (or a 429 rate-limit) from Reddit, got {status}"
    );
    if status == 200 {
        assert!(entries > 0, "a 200 from Reddit must parse into >=1 entry");
    } else {
        eprintln!("NOTE: Reddit returned 429 (rate-limited) — retry with more spacing.");
    }
}

#[tokio::test]
#[ignore = "hits the real network; run via `just test-live`"]
async fn youtube_channel_atom_fetches_and_parses() {
    // Fireship's channel Atom feed (a stable, high-traffic channel).
    let (status, entries) = fetch_and_parse(
        "https://www.youtube.com/feeds/videos.xml?channel_id=UCsBjURrPoezykLs9EqgamOA",
    )
    .await;
    assert_eq!(
        status, 200,
        "YouTube Atom feed should return 200 (got {status})"
    );
    assert!(entries > 0, "YouTube Atom feed should parse into >=1 entry");
}

#[tokio::test]
#[ignore = "hits the real network; run via `just test-live`"]
async fn hacker_news_rss_fetches_and_parses() {
    let (status, entries) = fetch_and_parse("https://hnrss.org/frontpage").await;
    assert_eq!(
        status, 200,
        "hnrss frontpage should return 200 (got {status})"
    );
    assert!(entries > 0, "hnrss frontpage should parse into >=1 entry");
}

#[tokio::test]
#[ignore = "hits the real network; run via `just test-live`"]
async fn mastodon_user_rss_fetches_and_parses() {
    let (status, entries) = fetch_and_parse("https://mastodon.social/@Gargron.rss").await;
    assert_eq!(
        status, 200,
        "Mastodon user .rss should return 200 (got {status})"
    );
    assert!(
        entries > 0,
        "Mastodon user .rss should parse into >=1 entry"
    );
}
