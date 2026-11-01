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
    TimeoutPhase, TransportKind,
};
use curio_core::model::FetchErrorKind;
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
    assert_eq!(response.redirects, 2, "two hops followed");
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
async fn a_per_request_redirect_cap_tightens_but_never_loosens_the_config() {
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
    let start = format!("{}/hop0", server.uri());

    let err = client
        .fetch(&FetchRequest {
            max_redirects: Some(1),
            ..allowed(start.clone())
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::TooManyRedirects { limit: 1, .. }),
        "{err:?}"
    );
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        2,
        "the first request plus the one allowed hop"
    );

    let err = client
        .fetch(&FetchRequest {
            max_redirects: Some(50),
            ..allowed(start)
        })
        .await
        .unwrap_err();
    assert!(
        matches!(err, FetchError::TooManyRedirects { limit: 5, .. }),
        "the config cap still applies: {err:?}"
    );
}

#[tokio::test]
async fn same_origin_redirects_refuses_an_off_origin_hop_without_requesting_it() {
    let server = MockServer::start().await;
    let elsewhere = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/target"))
        .respond_with(ResponseTemplate::new(200).set_body_string("off origin"))
        .mount(&elsewhere)
        .await;
    Mock::given(method("GET"))
        .and(path("/away"))
        .respond_with(
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/target", elsewhere.uri())),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/near"))
        .respond_with(ResponseTemplate::new(301).insert_header("location", "/end"))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/end"))
        .respond_with(ResponseTemplate::new(200).set_body_string("same origin"))
        .mount(&server)
        .await;
    let client = PolicedClient::new(fast_config());
    let same_origin = |at: &str| FetchRequest {
        same_origin_redirects: true,
        ..allowed(format!("{}{at}", server.uri()))
    };

    let err = client.fetch(&same_origin("/away")).await.unwrap_err();
    assert!(
        matches!(&err, FetchError::CrossOriginRedirect { location, hops: 0, .. }
            if location.starts_with(&elsewhere.uri())),
        "{err:?}"
    );
    assert!(
        elsewhere.received_requests().await.unwrap().is_empty(),
        "the off-origin target is never requested"
    );

    // A same-origin hop is still followed.
    let response = client.fetch(&same_origin("/near")).await.unwrap();
    assert_eq!(response.body, b"same origin");
    assert_eq!(response.redirects, 1);

    // Without the flag, the off-origin hop is followed as before.
    let response = client
        .fetch(&allowed(format!("{}/away", server.uri())))
        .await
        .unwrap();
    assert_eq!(response.body, b"off origin");
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
    assert_eq!(response.body, [] as [u8; 0]);

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
        matches!(
            err,
            FetchError::Timeout {
                phase: TimeoutPhase::Read,
                ..
            }
        ),
        "expected a read Timeout, got {err:?}"
    );
    assert_eq!(err.kind(), FetchErrorKind::ReadTimeout);
}

/// A loopback port with nothing listening on it.
fn closed_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
    // dropped: the port is closed again
}

#[tokio::test]
async fn a_closed_port_is_a_refused_connection_naming_the_cause_once() {
    let url = format!("http://127.0.0.1:{}/feed.xml", closed_port());
    let err = PolicedClient::new(fast_config())
        .fetch(&allowed(&url))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            FetchError::Transport {
                kind: TransportKind::Refused,
                ..
            }
        ),
        "expected a refused connection, got {err:?}"
    );
    assert_eq!(err.kind(), FetchErrorKind::Refused);
    let message = err.to_string();
    assert_eq!(message.matches(&url).count(), 1, "{message}");
    assert!(!message.contains("error sending request"), "{message}");
    assert!(message.to_lowercase().contains("refused"), "{message}");
}

/// A TLS server on `127.0.0.1` presenting a freshly minted self-signed
/// certificate; returns its port. Each connection runs the handshake and
/// is dropped — the client is expected to abort it.
async fn self_signed_tls_server() -> u16 {
    use tokio_rustls::rustls;
    let cert = rcgen::generate_simple_self_signed(vec!["127.0.0.1".to_owned()]).unwrap();
    let key = rustls::pki_types::PrivateKeyDer::Pkcs8(cert.key_pair.serialize_der().into());
    let config = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert.cert.der().clone()], key)
    .unwrap();
    let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(config));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let _ = acceptor.accept(stream).await;
            });
        }
    });
    port
}

#[tokio::test]
async fn a_self_signed_certificate_is_a_tls_certificate_error() {
    let url = format!(
        "https://127.0.0.1:{}/feed.xml",
        self_signed_tls_server().await
    );
    let err = PolicedClient::new(fast_config())
        .fetch(&allowed(&url))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            FetchError::Transport {
                kind: TransportKind::TlsCertificate,
                ..
            }
        ),
        "expected a TLS certificate error, got {err:?}"
    );
    assert_eq!(err.kind(), FetchErrorKind::TlsCertificate);
    let message = err.to_string();
    assert_eq!(message.matches(&url).count(), 1, "{message}");
    // The cause the TLS stack reported, not reqwest's summary.
    assert!(message.contains("certificate"), "{message}");
    assert!(!message.contains("error sending request"), "{message}");
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
