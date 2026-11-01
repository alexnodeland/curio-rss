//! Messy real-world import files (issue #132): bad entities, non-UTF-8
//! encodings, the `feed:` pseudo-scheme and padded URLs. Every usable row
//! imports, every unusable one is counted with its reason — never fatal —
//! and a second import of the same file adds nothing.

#![allow(clippy::unwrap_used)]

mod common;

use std::fmt::Write as _;
use std::time::Duration;

use curio_core::fetch::FetchConfig;
use curio_core::import::read_import_file;
use curio_core::model::{ArticleContent, NewArticle};
use curio_core::storage::{ListArticles, Storage};
use curio_core::{CoreHandle, CoreOptions, ImportKind, ImportSource, RejectReason};

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

/// Reads a fixture the way the heads do: bytes → size cap → decoding.
fn fixture(rel: &str) -> String {
    read_import_file(&common::manifest_dir().join("../../fixtures").join(rel)).unwrap()
}

fn feed_urls(core: &CoreHandle) -> Vec<String> {
    let mut urls: Vec<String> = core
        .list_feeds()
        .unwrap()
        .into_iter()
        .map(|f| f.url)
        .collect();
    urls.sort();
    urls
}

fn title_of(core: &CoreHandle, url: &str) -> Option<String> {
    core.list_feeds()
        .unwrap()
        .into_iter()
        .find(|f| f.url == url)
        .and_then(|f| f.title)
}

#[test]
fn bad_entities_and_broken_outlines_do_not_abort_the_import() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let opml = fixture("opml/messy-entities.opml");

    let outcome = core.import_file(ImportSource::Opml, &opml).unwrap();
    assert_eq!(outcome.feeds_added, 5, "{outcome:?}");
    assert_eq!(outcome.feeds_skipped, 0);
    // The unquoted-attribute outline is unreadable; the webcal: one is not
    // a feed. Both are counted, with reasons, and neither is fatal.
    assert_eq!(outcome.feeds_invalid, 2, "{outcome:?}");
    let reasons: Vec<RejectReason> = outcome.rejected.iter().map(|r| r.reason).collect();
    assert_eq!(
        reasons,
        vec![RejectReason::Malformed, RejectReason::UnsupportedScheme]
    );
    assert!(outcome.rejected.iter().all(|r| r.kind == ImportKind::Feed));
    assert!(
        outcome.rejected[0].detail.contains("unquoted.example"),
        "the reason quotes the broken outline: {}",
        outcome.rejected[0].detail
    );
    assert_eq!(
        outcome.rejected[1].detail,
        "webcal://calendar.example/events.ics"
    );

    assert_eq!(
        feed_urls(&core),
        vec![
            "https://bogus.example/feed",
            "https://cafe.example/feed.xml",
            "https://dup.example/feed",
            "https://plain.example/atom.xml",
            "https://rnd.example/rss?a=1&b=2",
        ]
    );
    // HTML entities resolve; a bare `&` and an unknown entity stay raw.
    assert_eq!(
        title_of(&core, "https://cafe.example/feed.xml").as_deref(),
        Some("Café Society")
    );
    assert_eq!(
        title_of(&core, "https://rnd.example/rss?a=1&b=2").as_deref(),
        Some("R&D Weekly")
    );
    assert_eq!(
        title_of(&core, "https://bogus.example/feed").as_deref(),
        Some("&bogus; Entity")
    );
    // A duplicated attribute is tolerated: the last one wins.
    assert_eq!(
        title_of(&core, "https://dup.example/feed").as_deref(),
        Some("Second")
    );
    // The folder name's `&nbsp;` decodes too (and is trimmed as whitespace
    // only at the ends, so the inner no-break spaces survive).
    let cafe = core
        .list_feeds()
        .unwrap()
        .into_iter()
        .find(|f| f.url == "https://cafe.example/feed.xml")
        .unwrap();
    assert_eq!(cafe.tags, vec!["News\u{A0}&\u{A0}Politics".to_owned()]);

    let again = core.import_file(ImportSource::Opml, &opml).unwrap();
    assert_eq!(again.feeds_added, 0, "a second import adds nothing");
    assert_eq!(again.feeds_skipped, 5);
    assert_eq!(again.feeds_invalid, 2);
}

#[test]
fn a_latin1_opml_imports_with_its_accents_intact() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let raw =
        std::fs::read(common::manifest_dir().join("../../fixtures/opml/latin1.opml")).unwrap();
    assert!(
        std::str::from_utf8(&raw).is_err(),
        "the fixture must really be non-UTF-8"
    );
    let opml = fixture("opml/latin1.opml");

    let outcome = core.import_opml(&opml).unwrap();
    assert_eq!(outcome.added, 3);
    assert_eq!(outcome.invalid, 0);
    assert_eq!(
        title_of(&core, "https://cafe.example.fr/rss").as_deref(),
        Some("Le Café du Commerce")
    );
    assert_eq!(
        title_of(&core, "https://muenchen.example.de/feed").as_deref(),
        Some("Grüße aus München")
    );
    assert_eq!(
        title_of(&core, "https://senor.example.es/feed.xml").as_deref(),
        Some("Señor ¿Qué?")
    );
    let cafe = core
        .list_feeds()
        .unwrap()
        .into_iter()
        .find(|f| f.url == "https://cafe.example.fr/rss")
        .unwrap();
    assert_eq!(cafe.tags, vec!["Français".to_owned()]);

    assert_eq!(core.import_opml(&opml).unwrap().added, 0);
}

#[test]
fn a_utf16le_excel_csv_imports_and_counts_its_bad_rows() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let csv = fixture("import/pocket-utf16le.csv");

    let outcome = core.import_file(ImportSource::PocketCsv, &csv).unwrap();
    assert_eq!(outcome.articles_added, 3, "{outcome:?}");
    assert_eq!(outcome.articles_invalid, 2);
    assert_eq!(outcome.feeds_invalid, 0);
    let rejected: Vec<(RejectReason, &str)> = outcome
        .rejected
        .iter()
        .map(|r| (r.reason, r.detail.as_str()))
        .collect();
    assert_eq!(
        rejected,
        vec![
            (RejectReason::UnsupportedScheme, "javascript:alert(1)"),
            (RejectReason::InvalidUrl, "not a url at all"),
        ]
    );

    let articles = core.list_articles(ListArticles::default()).unwrap();
    let cafe = articles
        .iter()
        .find(|a| a.source_url == "https://example.com/cafe")
        .unwrap();
    assert_eq!(cafe.title, "Café, revisited");
    let mut tags = core.article_tags(cafe.id).unwrap();
    tags.sort();
    assert_eq!(tags, vec!["food", "été"]);
    assert!(articles.iter().any(|a| a.title == "日本語の記事"));
    assert!(
        articles
            .iter()
            .any(|a| a.source_url == "https://example.com/padded"),
        "the padded URL is trimmed"
    );

    let again = core.import_file(ImportSource::PocketCsv, &csv).unwrap();
    assert_eq!(again.articles_added, 0, "a second import adds nothing");
    assert_eq!(again.articles_skipped, 3);
}

#[test]
fn feed_scheme_and_padded_urls_normalize_and_dedupe() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let opml = fixture("opml/feed-scheme.opml");

    let outcome = core.import_file(ImportSource::Opml, &opml).unwrap();
    assert_eq!(outcome.feeds_added, 4, "{outcome:?}");
    // Two more spellings of https://same.example/ collapse onto the first.
    assert_eq!(outcome.feeds_skipped, 2);
    assert_eq!(outcome.feeds_invalid, 2);
    let reasons: Vec<RejectReason> = outcome.rejected.iter().map(|r| r.reason).collect();
    assert_eq!(
        reasons,
        vec![RejectReason::UnsupportedScheme, RejectReason::InvalidUrl]
    );
    assert_eq!(
        feed_urls(&core),
        vec![
            "https://padded.example/feed.xml",
            "https://same.example/",
            "https://scheme.example/rss.xml",
            "https://wrapped.example/atom",
        ]
    );

    let again = core.import_opml(&opml).unwrap();
    assert_eq!(again.added, 0, "a second import adds nothing");
    assert_eq!(again.skipped, 6);
    assert_eq!(again.invalid, 2);
}

#[test]
fn an_article_imported_before_normalization_is_not_duplicated() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    // What an older Curio stored for a Pocket row `https://Example.com`:
    // a feedless save keyed on the URL as the file spelled it.
    {
        let storage = Storage::open(profile.path().join("curio.db")).unwrap();
        storage
            .upsert_articles(vec![NewArticle {
                feed_id: None,
                dedupe_key: "link:https://Example.com".to_owned(),
                title: "Old save".to_owned(),
                source_url: "https://Example.com".to_owned(),
                author: None,
                published_at: None,
                content: ArticleContent::default(),
                lang: None,
                word_count: None,
                source_updated_at: None,
                lead_image: None,
            }])
            .unwrap();
    }
    let before = core.list_articles(ListArticles::default()).unwrap().len();

    let csv = "title,url,time_added,tags,status\nOld save, https://Example.com ,,,unread\n";
    let outcome = core.import_file(ImportSource::PocketCsv, csv).unwrap();
    assert_eq!(outcome.articles_added, 0, "{outcome:?}");
    assert_eq!(outcome.articles_skipped, 1);
    assert_eq!(
        core.list_articles(ListArticles::default()).unwrap().len(),
        before,
        "the legacy row is matched, not duplicated"
    );
}

#[test]
fn an_existing_subscription_dedupes_in_normalized_form() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    // Subscribed before normalization existed, in a non-canonical spelling.
    core.add_feed(curio_core::model::NewFeed {
        url: "HTTPS://Same.Example".to_owned(),
        title: None,
        tags: Vec::new(),
    })
    .unwrap();
    let outcome = core
        .import_opml(
            r#"<opml version="2.0"><body>
                <outline xmlUrl="https://same.example/"/>
                <outline xmlUrl="feed://same.example"/>
            </body></opml>"#,
        )
        .unwrap();
    assert_eq!(outcome.added, 0);
    assert_eq!(outcome.skipped, 2);
    assert_eq!(core.list_feeds().unwrap().len(), 1);
}

#[test]
fn rejection_details_are_capped_but_counts_stay_exact() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let mut csv = String::from("url,title\n");
    let bad_rows = curio_core::MAX_REJECTED_DETAILS + 25;
    for i in 0..bad_rows {
        writeln!(csv, "mailto:{i}@example.com,Bad {i}").unwrap();
    }
    csv.push_str("https://example.com/good,Good\n");

    let outcome = core.import_file(ImportSource::PocketCsv, &csv).unwrap();
    assert_eq!(outcome.articles_added, 1);
    assert_eq!(outcome.articles_invalid, bad_rows);
    assert_eq!(outcome.rejected.len(), curio_core::MAX_REJECTED_DETAILS);
    assert_eq!(outcome.rejected[0].detail, "mailto:0@example.com");
}

#[test]
fn an_opml_that_breaks_off_keeps_the_feeds_before_the_damage() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let truncated = r#"<?xml version="1.0"?>
<opml version="2.0"><body>
  <outline type="rss" text="Kept" xmlUrl="https://kept.example/feed"/>
  <outline type="rss" text="Cut off" xmlUrl="https://lost.example/feed"
"#;
    let outcome = core.import_file(ImportSource::Opml, truncated).unwrap();
    assert_eq!(outcome.feeds_added, 1);
    assert_eq!(outcome.feeds_invalid, 1);
    assert_eq!(outcome.rejected[0].reason, RejectReason::Malformed);
    assert!(
        outcome.rejected[0].detail.contains("unreadable XML"),
        "{}",
        outcome.rejected[0].detail
    );
    assert_eq!(feed_urls(&core), vec!["https://kept.example/feed"]);
}

#[test]
fn a_file_that_is_not_opml_at_all_is_still_an_error() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    assert!(
        core.import_file(ImportSource::Opml, "title,url\nA,https://a.test/\n")
            .is_err()
    );
    assert!(core.import_file(ImportSource::Opml, "<opml").is_err());
}
