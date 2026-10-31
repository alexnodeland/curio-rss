//! End-to-end discovery against wiremock (hermetic: 127.0.0.1 only).

#![allow(clippy::unwrap_used)]

use std::collections::HashSet;
use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr};
use std::time::Duration;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::fetch::FetchConfig;

/// A client that trusts the loopback fixture servers, with no politeness
/// delay.
fn client() -> PolicedClient {
    PolicedClient::new(FetchConfig {
        politeness_delay: Duration::ZERO,
        trusted_addrs: HashSet::from([IpAddr::V4(Ipv4Addr::LOCALHOST)]),
        ..FetchConfig::default()
    })
}

/// A fixture file, read at run time (issue #183: never a compile-time path).
fn fixture(relative: &str) -> String {
    let crate_dir = std::env::var_os("CARGO_MANIFEST_DIR")
        .unwrap_or_else(|| panic!("CARGO_MANIFEST_DIR unset: run the tests through cargo"));
    std::fs::read_to_string(
        std::path::Path::new(&crate_dir)
            .join("../../fixtures")
            .join(relative),
    )
    .unwrap()
}

fn rss() -> String {
    fixture("feeds/rss2.xml")
}

async fn serve(server: &MockServer, at: &str, body: impl Into<String>) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(200).set_body_string(body.into()))
        .mount(server)
        .await;
}

async fn discover_at(server: &MockServer, at: &str) -> Discovery {
    discover(
        &client(),
        &format!("{}{at}", server.uri()),
        DiscoverOptions::default(),
    )
    .await
    .unwrap()
}

/// The paths the server was asked for, in order.
async fn requested(server: &MockServer) -> Vec<String> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .iter()
        .map(|request| request.url.path().to_owned())
        .collect()
}

#[tokio::test]
async fn a_wordpress_post_yields_only_the_real_feed() {
    let server = MockServer::start().await;
    let page =
        fixture("discovery/wordpress-post.html").replace("https://blog.example", &server.uri());
    serve(&server, "/2026/10/hello-world/", page).await;
    // Every alternate the post declares answers — only the site feed counts.
    serve(&server, "/feed/", rss()).await;
    serve(&server, "/comments/feed/", rss()).await;
    serve(&server, "/2026/10/hello-world/feed/", rss()).await;
    serve(
        &server,
        "/wp-json/wp/v2/posts/1",
        r#"{"id":1,"title":{"rendered":"x"}}"#,
    )
    .await;

    let out = discover_at(&server, "/2026/10/hello-world/").await;
    assert_eq!(
        out.feeds,
        vec![DiscoveredFeed {
            url: format!("{}/feed/", server.uri()),
            title: Some("Example Blog".into()),
        }]
    );
    assert_eq!(
        requested(&server).await,
        vec!["/2026/10/hello-world/", "/feed/"],
        "comment, REST and oEmbed alternates are never even fetched"
    );
    assert_eq!(
        out.favicon,
        Some(format!(
            "{}/wp-content/uploads/2026/01/cropped-icon-32x32.png",
            server.uri()
        ))
    );
}

#[tokio::test]
async fn a_homepage_without_links_finds_feed_xml_by_probing() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/",
        "<html><head><title>Home</title></head><body>hi</body></html>",
    )
    .await;
    serve(&server, "/feed.xml", rss()).await;

    let out = discover_at(&server, "/").await;
    assert_eq!(
        out.feeds,
        vec![DiscoveredFeed {
            url: format!("{}/feed.xml", server.uri()),
            title: Some("Example Blog".into()),
        }]
    );
    // The page, `/feed` (404), then the hit — probing stops at the first.
    assert_eq!(requested(&server).await, vec!["/", "/feed", "/feed.xml"]);
}

#[tokio::test]
async fn probing_never_leaves_the_origin_or_exceeds_the_cap() {
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    serve(&elsewhere, "/feed.xml", rss()).await;
    // More feed-shaped same-origin anchors than the budget, plus one
    // cross-origin anchor (a different port is a different origin) that
    // actually serves a feed.
    let mut body = format!(r#"<a href="{}/feed.xml">elsewhere</a>"#, elsewhere.uri());
    for n in 0..10 {
        write!(body, r#"<a href="/section-{n}/feed/">feed {n}</a>"#).unwrap();
    }
    serve(&server, "/", body).await;

    let out = discover_at(&server, "/").await;
    assert!(out.feeds.is_empty(), "nothing on this origin is a feed");
    let paths = requested(&server).await;
    assert_eq!(paths.first().map(String::as_str), Some("/"));
    assert_eq!(paths.len() - 1, MAX_PROBES, "probes are capped: {paths:?}");
    assert!(
        requested(&elsewhere).await.is_empty(),
        "a cross-origin anchor is never probed"
    );
}

#[tokio::test]
async fn with_nothing_anywhere_every_common_path_is_tried_once() {
    let server = MockServer::start().await;
    serve(&server, "/blog/", "<html><body>no feeds</body></html>").await;

    let out = discover_at(&server, "/blog/").await;
    assert_eq!(out.feeds, Vec::new());
    let mut expected = vec!["/blog/".to_owned()];
    expected.extend(COMMON_PATHS.iter().map(|p| (*p).to_owned()));
    assert_eq!(requested(&server).await, expected);
    assert_eq!(out.favicon, Some(format!("{}/favicon.ico", server.uri())));
}

#[tokio::test]
async fn a_probe_redirected_off_the_origin_is_discarded() {
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    serve(&elsewhere, "/feed", rss()).await;
    serve(&server, "/", "<html></html>").await;
    Mock::given(method("GET"))
        .and(path("/feed"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/feed", elsewhere.uri())),
        )
        .mount(&server)
        .await;

    let out = discover_at(&server, "/").await;
    assert!(out.feeds.is_empty(), "an off-origin landing is not offered");
}

#[tokio::test]
async fn a_dead_declared_feed_falls_back_to_probing() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/",
        r#"<link rel="alternate" type="application/rss+xml" title="Old" href="/old.rss">"#,
    )
    .await;
    serve(&server, "/feed", rss()).await;

    let out = discover_at(&server, "/").await;
    assert_eq!(out.feeds.len(), 1);
    assert_eq!(out.feeds[0].url, format!("{}/feed", server.uri()));
    assert_eq!(requested(&server).await, vec!["/", "/old.rss", "/feed"]);
}

#[tokio::test]
async fn a_declared_link_that_is_not_a_feed_is_not_offered() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/",
        r#"<link rel="alternate" type="application/rss+xml" href="/feed.xml">
           <link rel="alternate" type="application/atom+xml" title="Atom" href="/atom">"#,
    )
    .await;
    // Serves HTML, not a feed — parse validation rejects it.
    serve(&server, "/feed.xml", "<html><body>moved</body></html>").await;
    serve(&server, "/atom", fixture("feeds/atom.xml")).await;

    let out = discover_at(&server, "/").await;
    assert_eq!(
        out.feeds,
        vec![DiscoveredFeed {
            url: format!("{}/atom", server.uri()),
            title: Some("Atom Example".into()),
        }]
    );
}

#[tokio::test]
async fn redirect_twins_dedupe_to_the_permanent_target() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/",
        r#"<link rel="alternate" type="application/rss+xml" title="RSS" href="/feed">
           <link rel="alternate" type="application/rss+xml" title="RSS 2" href="/feed.xml">"#,
    )
    .await;
    Mock::given(method("GET"))
        .and(path("/feed"))
        .respond_with(
            ResponseTemplate::new(301)
                .insert_header("location", format!("{}/feed.xml", server.uri())),
        )
        .mount(&server)
        .await;
    serve(&server, "/feed.xml", rss()).await;

    let out = discover_at(&server, "/").await;
    assert_eq!(
        out.feeds,
        vec![DiscoveredFeed {
            url: format!("{}/feed.xml", server.uri()),
            title: Some("Example Blog".into()),
        }]
    );
}

#[tokio::test]
async fn a_url_that_is_a_feed_returns_itself_with_its_title() {
    let server = MockServer::start().await;
    serve(&server, "/feed.xml", rss()).await;
    let out = discover_at(&server, "/feed.xml").await;
    assert_eq!(
        out.feeds,
        vec![DiscoveredFeed {
            url: format!("{}/feed.xml", server.uri()),
            title: Some("Example Blog".into()),
        }]
    );
    assert_eq!(
        requested(&server).await,
        vec!["/feed.xml"],
        "one fetch, no probes"
    );
}

#[tokio::test]
async fn a_feed_that_sniffs_right_but_does_not_parse_yields_nothing() {
    let server = MockServer::start().await;
    serve(&server, "/broken.xml", "<rss><channel><title>unterminated").await;
    let out = discover_at(&server, "/broken.xml").await;
    assert_eq!(out.feeds, Vec::new());
}

#[tokio::test]
async fn a_validated_rewrite_skips_the_page_fetch() {
    // The Mastodon rule is host-agnostic, so it runs against the fixture.
    let server = MockServer::start().await;
    serve(&server, "/@alice.rss", rss()).await;
    let out = discover_at(&server, "/@alice").await;
    assert_eq!(
        out.feeds,
        vec![DiscoveredFeed {
            url: format!("{}/@alice.rss", server.uri()),
            title: Some("Example Blog".into()),
        }]
    );
    assert_eq!(requested(&server).await, vec!["/@alice.rss"]);
    assert_eq!(out.favicon, Some(format!("{}/favicon.ico", server.uri())));
}

#[tokio::test]
async fn a_rewrite_that_fails_validation_falls_through_to_the_page() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/@bob",
        r#"<link rel="alternate" type="application/rss+xml" href="/users/bob.rss">"#,
    )
    .await;
    serve(&server, "/users/bob.rss", rss()).await;
    let out = discover_at(&server, "/@bob").await;
    assert_eq!(out.feeds.len(), 1);
    assert_eq!(out.feeds[0].url, format!("{}/users/bob.rss", server.uri()));
    assert_eq!(
        requested(&server).await,
        vec!["/@bob.rss", "/@bob", "/users/bob.rss"]
    );
}

#[tokio::test]
async fn blank_and_non_http_input_are_input_errors() {
    for input in ["", "   "] {
        let error = discover(&client(), input, DiscoverOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(error, DiscoverError::Empty), "{input:?}");
        assert!(error.is_input_error());
    }
    let error = discover(
        &client(),
        "ftp://example.test/x",
        DiscoverOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DiscoverError::InvalidUrl { .. }));
    assert!(error.is_input_error());
}

#[tokio::test]
async fn a_non_success_page_is_a_status_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let error = discover(
        &client(),
        &format!("{}/", server.uri()),
        DiscoverOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(error, DiscoverError::Status { status: 500, .. }),
        "{error}"
    );
    assert!(!error.is_input_error());
}

#[tokio::test]
async fn the_private_network_guard_applies_unless_explicitly_waived() {
    let server = MockServer::start().await;
    serve(&server, "/feed.xml", rss()).await;
    let url = format!("{}/feed.xml", server.uri());
    // A client that does NOT trust loopback: the SSRF guard refuses it…
    let guarded = PolicedClient::new(FetchConfig {
        politeness_delay: Duration::ZERO,
        ..FetchConfig::default()
    });
    let error = discover(&guarded, &url, DiscoverOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(error, DiscoverError::Fetch { .. }), "{error}");
    assert!(error.to_string().contains("could not reach"));
    // …unless the W1 exemption rides the options.
    let out = discover(
        &guarded,
        &url,
        DiscoverOptions {
            allow_private_network: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(out.feeds.len(), 1);
}

#[test]
fn input_normalization() {
    let ok = [
        ("example.test", "https://example.test/"),
        ("  example.test/blog  ", "https://example.test/blog"),
        ("//example.test", "https://example.test/"),
        ("example.test:8080/x", "https://example.test:8080/x"),
        ("localhost:8080", "https://localhost:8080/"),
        (
            "example.test/?u=http://other.test",
            "https://example.test/?u=http://other.test",
        ),
        ("http://example.test", "http://example.test/"),
        ("HTTPS://Example.Test/Feed", "https://example.test/Feed"),
        ("feed://example.test/rss", "https://example.test/rss"),
        ("feed:https://example.test/rss", "https://example.test/rss"),
        ("FEED:http://example.test/rss", "http://example.test/rss"),
    ];
    for (input, expected) in ok {
        assert_eq!(
            normalize_input(input).unwrap().as_str(),
            expected,
            "{input:?}"
        );
    }
    let bad = [
        "ftp://example.test",
        "javascript://x",
        "https://",
        "http:// /",
        "a b c",
    ];
    for input in bad {
        assert!(
            matches!(
                normalize_input(input),
                Err(DiscoverError::InvalidUrl { .. })
            ),
            "{input:?} should be rejected"
        );
    }
    assert!(matches!(normalize_input(" \t"), Err(DiscoverError::Empty)));
}
