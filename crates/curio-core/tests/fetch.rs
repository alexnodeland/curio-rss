//! Hermetic integration tests for the policed fetch client.
//!
//! Everything runs against wiremock servers on `127.0.0.1` — no real
//! network. Fixture feeds set `allow_private_network` (contract W1), which
//! simultaneously proves the exemption works; the per-hop re-validation
//! tests instead trust only the fixture's own loopback address via the
//! harness `trusted_addrs` escape hatch, so redirects *out* of it hit the
//! live deny-private policy.

#![allow(clippy::unwrap_used)]

use std::net::IpAddr;
use std::time::{Duration, Instant};

use curio_core::fetch::{
    DEFAULT_ACCEPT, FetchConfig, FetchError, FetchRequest, HostOverride, PolicedClient,
};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn fast_config() -> FetchConfig {
    FetchConfig {
        politeness_delay: Duration::ZERO,
        ..FetchConfig::default()
    }
}

/// A client that trusts exactly the fixture server's address, so the
/// deny-private policy stays live for every *other* address.
fn client_trusting(server: &MockServer) -> PolicedClient {
    let addr: IpAddr = server.address().ip();
    let mut config = fast_config();
    config.trusted_addrs.insert(addr);
    PolicedClient::new(config)
}

fn allowed(url: impl Into<String>) -> FetchRequest {
    FetchRequest {
        allow_private_network: true,
        ..FetchRequest::new(url)
    }
}

#[tokio::test]
async fn ssrf_guard_blocks_loopback_by_default() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("nope"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let err = client
        .fetch(&FetchRequest::new(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::PrivateAddress { addr, .. } if addr.is_loopback()),
        "expected PrivateAddress, got {err:?}"
    );
}

#[tokio::test]
async fn w1_allow_private_network_exempts_the_feed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_string("<rss/>"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let response = client
        .fetch(&allowed(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();
    assert!(response.is_success());
    assert_eq!(response.body, b"<rss/>");
    assert!(!response.permanent_redirect);
}

#[tokio::test]
async fn redirects_are_followed_with_relative_locations() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(ResponseTemplate::new(301).insert_header("location", "/middle"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/middle"))
        .respond_with(
            ResponseTemplate::new(308).insert_header("location", format!("{}/end", server.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/end"))
        .respond_with(ResponseTemplate::new(200).set_body_string("arrived"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let response = client
        .fetch(&allowed(format!("{}/start", server.uri())))
        .await
        .unwrap();
    assert!(response.is_success());
    assert_eq!(response.body, b"arrived");
    assert!(response.final_url.ends_with("/end"));
    assert!(
        response.permanent_redirect,
        "301 → 308 chain is permanent end-to-end"
    );
}

#[tokio::test]
async fn temporary_hop_marks_the_chain_non_permanent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "/end"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/end"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let response = client
        .fetch(&allowed(format!("{}/start", server.uri())))
        .await
        .unwrap();
    assert!(!response.permanent_redirect);
}

#[tokio::test]
async fn redirect_cap_is_enforced() {
    let server = MockServer::start().await;
    for hop in 0..7 {
        Mock::given(method("GET"))
            .and(path(format!("/hop{hop}")))
            .respond_with(
                ResponseTemplate::new(302).insert_header("location", format!("/hop{}", hop + 1)),
            )
            .mount(&server)
            .await;
    }

    let client = PolicedClient::new(fast_config());
    let err = client
        .fetch(&allowed(format!("{}/hop0", server.uri())))
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::TooManyRedirects { limit: 5, .. }),
        "expected TooManyRedirects, got {err:?}"
    );
}

#[tokio::test]
async fn redirect_hop_into_a_private_address_is_revalidated_and_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(302).insert_header("location", "http://10.9.9.9/internal"),
        )
        .mount(&server)
        .await;

    // Only the fixture's own loopback address is trusted; the policy is
    // live, so the second hop must be rejected before any connection.
    let client = client_trusting(&server);
    let err = client
        .fetch(&FetchRequest::new(format!("{}/start", server.uri())))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            FetchError::PrivateAddress { addr, .. } if addr == "10.9.9.9".parse::<IpAddr>().unwrap()
        ),
        "expected PrivateAddress for the redirect target, got {err:?}"
    );
}

#[tokio::test]
async fn redirect_hop_to_link_local_metadata_is_blocked() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(
            ResponseTemplate::new(301)
                .insert_header("location", "http://169.254.169.254/latest/meta-data/"),
        )
        .mount(&server)
        .await;

    let client = client_trusting(&server);
    let err = client
        .fetch(&FetchRequest::new(format!("{}/start", server.uri())))
        .await
        .unwrap_err();
    assert!(matches!(err, FetchError::PrivateAddress { .. }));
}

#[tokio::test]
async fn redirect_to_a_forbidden_scheme_is_rejected() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/start"))
        .respond_with(ResponseTemplate::new(302).insert_header("location", "file:///etc/passwd"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let err = client
        .fetch(&allowed(format!("{}/start", server.uri())))
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::UnsupportedScheme { .. }),
        "expected UnsupportedScheme, got {err:?}"
    );
}

#[tokio::test]
async fn body_cap_rejects_oversized_responses() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![b'x'; 64 * 1024]))
        .mount(&server)
        .await;

    let mut config = fast_config();
    config.max_body_bytes = 1024;
    let client = PolicedClient::new(config);
    let err = client
        .fetch(&allowed(format!("{}/big", server.uri())))
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::BodyTooLarge { limit: 1024, .. }),
        "expected BodyTooLarge, got {err:?}"
    );
}

#[tokio::test]
async fn conditional_get_sends_validators_and_maps_304() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("if-none-match", "\"v1\""))
        .respond_with(ResponseTemplate::new(304))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let request = FetchRequest {
        etag: Some("\"v1\"".to_owned()),
        last_modified: Some("Wed, 01 Jul 2026 00:00:00 GMT".to_owned()),
        ..allowed(format!("{}/feed.xml", server.uri()))
    };
    let response = client.fetch(&request).await.unwrap();
    assert!(response.is_not_modified());
    assert!(response.body.is_empty());

    // Both validators rode the request (asserted on the raw request:
    // wiremock's header matcher splits date values on commas).
    let requests = server.received_requests().await.unwrap();
    let sent = &requests[0];
    assert_eq!(
        sent.headers
            .get("if-modified-since")
            .and_then(|v| v.to_str().ok()),
        Some("Wed, 01 Jul 2026 00:00:00 GMT")
    );
}

#[tokio::test]
async fn response_validators_are_captured_for_the_next_fetch() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("etag", "\"v2\"")
                .insert_header("last-modified", "Thu, 02 Jul 2026 00:00:00 GMT")
                .set_body_string("<rss/>"),
        )
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let response = client
        .fetch(&allowed(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();
    assert_eq!(response.etag.as_deref(), Some("\"v2\""));
    assert_eq!(
        response.last_modified.as_deref(),
        Some("Thu, 02 Jul 2026 00:00:00 GMT")
    );
}

#[tokio::test]
async fn gone_feeds_surface_as_410_with_empty_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(410).set_body_string("gone forever"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let response = client
        .fetch(&allowed(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();
    assert!(response.is_gone());
    assert!(response.body.is_empty(), "error bodies are never read");
}

#[tokio::test]
async fn honest_user_agent_rides_every_request() {
    let server = MockServer::start().await;
    let expected = FetchConfig::default().user_agent;
    Mock::given(method("GET"))
        .and(header("user-agent", expected.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    let response = client
        .fetch(&allowed(format!("{}/ua", server.uri())))
        .await
        .unwrap();
    assert!(response.is_success());
}

#[tokio::test]
async fn default_accept_header_rides_every_request() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let client = PolicedClient::new(fast_config());
    client
        .fetch(&allowed(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();

    let requests = server.received_requests().await.unwrap();
    let accept = requests[0]
        .headers
        .get("accept")
        .and_then(|v| v.to_str().ok());
    assert_eq!(
        accept,
        Some(DEFAULT_ACCEPT),
        "the spec-correct Accept header must ride every request"
    );
}

#[tokio::test]
async fn host_override_applies_ua_headers_by_suffix() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(header("user-agent", "OverrideUA/1.0"))
        .and(header("x-curio-test", "yes"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    // The fixture host is 127.0.0.1; override on that suffix so it matches.
    let mut config = fast_config();
    config.trusted_addrs.insert(server.address().ip());
    config.host_overrides = vec![HostOverride {
        host_suffix: "127.0.0.1".to_owned(),
        user_agent: Some("OverrideUA/1.0".to_owned()),
        extra_headers: vec![("x-curio-test".to_owned(), "yes".to_owned())],
        politeness_delay: Some(Duration::ZERO),
        use_native_tls: false,
    }];
    let client = PolicedClient::new(config);
    let response = client
        .fetch(&FetchRequest::new(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();
    assert!(
        response.is_success(),
        "override UA + extra header must ride a matching-host request"
    );
}

#[tokio::test]
async fn host_override_native_tls_builds_and_fetches() {
    // Exercises the `use_native_tls` build branch. The fixture speaks plain
    // HTTP so no TLS handshake happens, but the native-TLS client must still
    // build and complete the request (guards the reddit.com code path).
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let mut config = fast_config();
    config.trusted_addrs.insert(server.address().ip());
    config.host_overrides = vec![HostOverride {
        host_suffix: "127.0.0.1".to_owned(),
        user_agent: None,
        extra_headers: Vec::new(),
        politeness_delay: Some(Duration::ZERO),
        use_native_tls: true,
    }];
    let client = PolicedClient::new(config);
    let response = client
        .fetch(&FetchRequest::new(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();
    assert!(response.is_success());
}

#[tokio::test]
async fn host_override_does_not_leak_to_other_hosts() {
    // An override scoped to reddit.com must not touch a 127.0.0.1 request:
    // the honest default UA still rides. The mock only answers the honest UA,
    // so a leaked override UA would 404 the request and fail this test.
    let server = MockServer::start().await;
    let honest = FetchConfig::default().user_agent;
    Mock::given(method("GET"))
        .and(header("user-agent", honest.as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let mut config = fast_config();
    config.trusted_addrs.insert(server.address().ip());
    config.host_overrides = vec![HostOverride {
        host_suffix: "reddit.com".to_owned(),
        user_agent: Some("ShouldNotAppear/9".to_owned()),
        extra_headers: vec![("x-should-not-appear".to_owned(), "1".to_owned())],
        politeness_delay: None,
        use_native_tls: false,
    }];
    let client = PolicedClient::new(config);
    let response = client
        .fetch(&FetchRequest::new(format!("{}/feed.xml", server.uri())))
        .await
        .unwrap();
    assert!(
        response.is_success(),
        "reddit.com override must not leak onto a 127.0.0.1 request"
    );
}

#[tokio::test]
async fn read_timeout_is_enforced() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(5)))
        .mount(&server)
        .await;

    let mut config = fast_config();
    config.request_timeout = Duration::from_millis(200);
    let client = PolicedClient::new(config);
    let err = client
        .fetch(&allowed(format!("{}/slow", server.uri())))
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::Timeout { .. }),
        "expected Timeout, got {err:?}"
    );
}

#[tokio::test]
async fn politeness_delay_spaces_same_host_requests() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_string("ok"))
        .mount(&server)
        .await;

    let mut config = fast_config();
    config.politeness_delay = Duration::from_millis(120);
    let client = PolicedClient::new(config);
    let url = format!("{}/feed.xml", server.uri());

    let started = Instant::now();
    client.fetch(&allowed(&url)).await.unwrap();
    client.fetch(&allowed(&url)).await.unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(120),
        "second same-host fetch must wait out the politeness delay"
    );
}
