//! [`resolve`]: the validating subscribe path over discovery (wiremock,
//! 127.0.0.1 only).

use super::*;

#[tokio::test]
async fn resolve_picks_the_best_validated_feed_and_keeps_the_rest_as_alternates() {
    let server = MockServer::start().await;
    let page = format!(
        r#"<html><head>
        <link rel="alternate" type="application/rss+xml" title="Posts" href="{0}/posts.xml">
        <link rel="alternate" type="application/atom+xml" title="Notes" href="{0}/notes.xml">
        </head><body></body></html>"#,
        server.uri()
    );
    serve(&server, "/", page).await;
    serve(&server, "/posts.xml", rss()).await;
    serve(&server, "/notes.xml", fixture("feeds/atom.xml")).await;

    let resolution = resolve(
        &client(),
        &format!("  {}/ ", server.uri()),
        DiscoverOptions::default(),
    )
    .await
    .unwrap();
    assert_eq!(resolution.url, format!("{}/posts.xml", server.uri()));
    assert_eq!(
        resolution.alternates,
        vec![format!("{}/notes.xml", server.uri())]
    );
}

#[tokio::test]
async fn resolve_refuses_a_page_that_offers_no_feed() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/",
        "<html><head><title>Home</title></head></html>",
    )
    .await;
    let input = format!("{}/", server.uri());

    let error = resolve(&client(), &format!(" {input} "), DiscoverOptions::default())
        .await
        .unwrap_err();
    assert!(
        matches!(&error, DiscoverError::NoFeed { input: echoed } if *echoed == input),
        "{error}"
    );
    assert_eq!(error.to_string(), format!("no feed found at {input}"));
    // Not the input's fault: the address is fine, the site has no feed.
    assert!(!error.is_input_error());
}

#[tokio::test]
async fn resolve_passes_discovery_errors_through() {
    let error = resolve(
        &client(),
        "mailto:a@example.test",
        DiscoverOptions::default(),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DiscoverError::InvalidUrl { .. }), "{error}");
}

/// A feed with no entries yet — a freshly created email-to-feed inbox
/// (docs/newsletters.md) — is still a valid feed to subscribe to.
#[tokio::test]
async fn resolve_accepts_a_feed_with_no_entries_yet() {
    let server = MockServer::start().await;
    let empty = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <title>Weekly Digest Inbox</title>
  <id>urn:inbox:k7xq2p</id>
  <updated>2026-11-01T00:00:00Z</updated>
</feed>"#;
    serve(&server, "/feeds/k7xq2p.xml", empty).await;
    let url = format!("{}/feeds/k7xq2p.xml", server.uri());

    let resolution = resolve(&client(), &url, DiscoverOptions::default())
        .await
        .unwrap();
    assert_eq!(resolution.url, url);
}
