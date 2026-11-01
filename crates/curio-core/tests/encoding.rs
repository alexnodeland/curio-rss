//! Character encodings, compression and Content-Type through the facade
//! (#125): legacy-charset pages and feeds come out as the right text,
//! compressed feeds ingest, and a PDF URL is clipped link-only.
//!
//! The byte-exact fixtures live in `fixtures/encoding/`; every server is a
//! wiremock on `127.0.0.1`.

#![allow(clippy::unwrap_used)]

use std::io::Write as _;
use std::time::Duration;

use curio_core::fetch::FetchConfig;
use curio_core::model::{ArticleContent, FetchStatus, NewArticle, NewFeed};
use curio_core::storage::ListArticles;
use curio_core::{CoreHandle, CoreOptions};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const WINDOWS_1251_PAGE: &[u8] = include_bytes!("../../../fixtures/encoding/windows-1251.html");
const SHIFT_JIS_FEED: &[u8] = include_bytes!("../../../fixtures/encoding/shift_jis-feed.xml");
const BOM_FEED: &[u8] = include_bytes!("../../../fixtures/encoding/utf8-bom-feed.xml");
const LEADING_WS_FEED: &[u8] =
    include_bytes!("../../../fixtures/encoding/leading-whitespace-feed.xml");
const RSS: &str = include_str!("../../../fixtures/feeds/rss2.xml");

const RUSSIAN_TITLE: &str = "Съешь же ещё этих мягких французских булок";

/// A core whose SSRF guard trusts `127.0.0.1`, so feedless paths
/// (`save_url`) reach the hermetic fixture server.
fn open_core(profile: &std::path::Path) -> CoreHandle {
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

async fn serve(server: &MockServer, at: &str, response: ResponseTemplate) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(response)
        .mount(server)
        .await;
}

/// Subscribes to `url`, refreshes it once and returns the stored articles.
async fn refresh_once(core: &CoreHandle, url: String) -> Vec<curio_core::model::Article> {
    let feed = core
        .add_feed(NewFeed {
            url,
            title: None,
            tags: Vec::new(),
        })
        .unwrap();
    let outcome = core.refresh_feed(feed.id).await.unwrap();
    assert_eq!(outcome.status, FetchStatus::Ok, "{:?}", outcome.error);
    core.list_articles(ListArticles::default()).unwrap()
}

fn feed_title(core: &CoreHandle) -> Option<String> {
    core.list_feeds().unwrap().remove(0).title
}

#[tokio::test]
async fn a_windows_1251_page_clips_with_its_cyrillic_intact() {
    let server = MockServer::start().await;
    // No charset in the header: only the page's <meta charset> says it.
    serve(
        &server,
        "/ru",
        ResponseTemplate::new(200).set_body_raw(WINDOWS_1251_PAGE, "text/html"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let saved = core
        .save_url(&format!("{}/ru", server.uri()), Vec::new())
        .await
        .unwrap();

    assert!(saved.hydrated);
    assert_eq!(saved.article.title, RUSSIAN_TITLE);
    assert!(
        saved
            .article
            .content
            .text
            .contains("закодирована в windows-1251"),
        "{}",
        saved.article.content.text
    );
    assert!(!saved.article.content.text.contains('\u{fffd}'));
}

#[tokio::test]
async fn a_windows_1251_page_hydrates_by_its_http_charset() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ru",
        ResponseTemplate::new(200)
            .set_body_raw(WINDOWS_1251_PAGE, "text/html; charset=windows-1251"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let id = stored_article(&core, format!("{}/ru", server.uri()));
    let hydrated = core.hydrate_article(id).await.unwrap();
    assert!(
        hydrated.content.text.contains("Второй абзац"),
        "{}",
        hydrated.content.text
    );
}

#[tokio::test]
async fn a_shift_jis_feed_with_an_http_only_charset_ingests_as_japanese() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/feed.xml",
        ResponseTemplate::new(200)
            .set_body_raw(SHIFT_JIS_FEED, "application/rss+xml; charset=Shift_JIS"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let articles = refresh_once(&core, format!("{}/feed.xml", server.uri())).await;
    assert_eq!(feed_title(&core).as_deref(), Some("日本語のフィード"));
    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].title, "こんにちは世界");
    assert!(
        articles[0]
            .content
            .text
            .contains("シフトJISで書かれた本文です。")
    );
}

#[tokio::test]
async fn a_utf8_bom_feed_ingests() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/atom.xml",
        ResponseTemplate::new(200).set_body_raw(BOM_FEED, "application/atom+xml"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let articles = refresh_once(&core, format!("{}/atom.xml", server.uri())).await;
    assert_eq!(
        feed_title(&core).as_deref(),
        Some("Café Feed — BOM edition")
    );
    assert_eq!(articles[0].title, "Naïve façade");
}

#[tokio::test]
async fn a_feed_with_whitespace_before_its_xml_declaration_ingests() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/feed.xml",
        ResponseTemplate::new(200).set_body_raw(LEADING_WS_FEED, "application/rss+xml"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let articles = refresh_once(&core, format!("{}/feed.xml", server.uri())).await;
    assert_eq!(
        feed_title(&core).as_deref(),
        Some("Leading Whitespace Feed")
    );
    assert_eq!(articles[0].title, "Still parses — résumé");
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(bytes).unwrap();
    encoder.finish().unwrap()
}

fn brotli(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    brotli::BrotliCompress(
        &mut std::io::Cursor::new(bytes),
        &mut out,
        &brotli::enc::BrotliEncoderParams::default(),
    )
    .unwrap();
    out
}

#[tokio::test]
async fn gzip_and_brotli_feeds_ingest() {
    for (coding, body) in [
        ("gzip", gzip(RSS.as_bytes())),
        ("br", brotli(RSS.as_bytes())),
    ] {
        let server = MockServer::start().await;
        serve(
            &server,
            "/feed.xml",
            ResponseTemplate::new(200)
                .insert_header("content-encoding", coding)
                .set_body_raw(body, "application/rss+xml"),
        )
        .await;

        let profile = tempfile::tempdir().unwrap();
        let core = open_core(profile.path());
        let articles = refresh_once(&core, format!("{}/feed.xml", server.uri())).await;
        assert_eq!(articles.len(), 3, "{coding}");
        assert_eq!(
            feed_title(&core).as_deref(),
            Some("Example Blog"),
            "{coding}"
        );
    }
}

#[tokio::test]
async fn a_gzip_compressed_legacy_page_clips_correctly() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/ru",
        ResponseTemplate::new(200)
            .insert_header("content-encoding", "gzip")
            .set_body_raw(gzip(WINDOWS_1251_PAGE), "text/html"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let saved = core
        .save_url(&format!("{}/ru", server.uri()), Vec::new())
        .await
        .unwrap();
    assert_eq!(saved.article.title, RUSSIAN_TITLE);
}

/// A body that *would* extract to a titled article if readability ran on
/// it — so a link-only result proves extraction was skipped.
const PDF_LOOKALIKE: &str = "%PDF-1.7\n<html><head><title>Extracted anyway</title></head>\
    <body><article><p>Readability must never see this text, however long and \
    article-like it looks to a scorer hunting for paragraphs.</p></article></body></html>";

#[tokio::test]
async fn clipping_a_pdf_url_saves_a_link_only_item_without_extraction() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/paper.pdf",
        ResponseTemplate::new(200).set_body_raw(PDF_LOOKALIKE, "application/pdf"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let url = format!("{}/paper.pdf", server.uri());
    let saved = core
        .save_url(&url, vec!["papers".to_owned()])
        .await
        .unwrap();

    assert!(saved.created);
    assert!(!saved.hydrated, "a PDF is not extracted");
    assert_eq!(saved.article.title, url, "the URL stands in for a title");
    assert_eq!(saved.article.content.text, "");
    assert_eq!(saved.article.word_count, None);
    assert!(core.article_state(saved.article.id).unwrap().read_later);
    assert_eq!(
        core.article_tags(saved.article.id).unwrap(),
        vec!["papers".to_owned()]
    );
}

#[tokio::test]
async fn an_untyped_pdf_body_is_sniffed_and_clipped_link_only() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/download",
        ResponseTemplate::new(200).set_body_bytes(PDF_LOOKALIKE.as_bytes()),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let saved = core
        .save_url(&format!("{}/download", server.uri()), Vec::new())
        .await
        .unwrap();
    assert!(!saved.hydrated);
    assert_eq!(saved.article.content.text, "");
}

#[tokio::test]
async fn hydrating_an_image_url_leaves_the_article_unchanged() {
    let server = MockServer::start().await;
    serve(
        &server,
        "/photo",
        ResponseTemplate::new(200).set_body_raw(PDF_LOOKALIKE, "image/jpeg"),
    )
    .await;

    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let id = stored_article(&core, format!("{}/photo", server.uri()));
    let before = core.get_article(id).unwrap().unwrap();
    let after = core.hydrate_article(id).await.unwrap();
    assert_eq!(after.content, before.content);
    assert!(!after.content.text.contains("Readability"));
}

/// Stores a feedless article whose source page is `url`.
fn stored_article(core: &CoreHandle, url: String) -> curio_core::model::ArticleId {
    core.storage()
        .upsert_articles(vec![NewArticle {
            feed_id: None,
            dedupe_key: "k".to_owned(),
            title: "stub".to_owned(),
            source_url: url,
            author: None,
            published_at: None,
            content: ArticleContent {
                html: "<p>x</p>".to_owned(),
                text: "x".to_owned(),
            },
            lang: None,
            word_count: None,
            source_updated_at: None,
            lead_image: None,
        }])
        .unwrap();
    core.list_articles(ListArticles::default())
        .unwrap()
        .remove(0)
        .id
}
