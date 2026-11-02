//! Generator for the released-version migration fixtures in this directory.
//!
//! NOT compiled by the workspace. It is the integration test that was
//! dropped into a scratch checkout of each release tag and run there, so
//! every `vX.Y.Z.db` was written by that release's own storage code (its
//! own migrations, its own repo SQL) — never by today's. Regenerating one:
//!
//! ```text
//! git worktree add --detach /tmp/curio-vX.Y.Z vX.Y.Z
//! cp fixtures/db/generate.rs /tmp/curio-vX.Y.Z/crates/curio-core/tests/mkfixture.rs
//! cd /tmp/curio-vX.Y.Z
//! CURIO_FIXTURE_OUT=$PWD/vX.Y.Z.db cargo test -p curio-core --test mkfixture -- --ignored
//! ```
//!
//! The data set is identical across tags, so the upgrade test in
//! `crates/curio-core/tests/migration_fixtures.rs` asserts one set of
//! expected counts against every fixture. Keep [`EXPECTED`] there in sync
//! with what this writes.
//!
//! The storage API it uses was stable from v0.1.0 to v0.4.0; v0.6.0 added
//! `FetchRecord::error_kind` (migration 0009). For a tag before v0.6.0,
//! delete the `error_kind` line (and its `FetchErrorKind` import) in the
//! scratch copy.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use curio_core::model::{
    ArticleContent, FeedStatus, FetchErrorKind, FetchRecord, FetchStatus, NewArticle, NewFeed,
};
use curio_core::storage::{ListArticles, Storage};
use curio_types::Timestamp;

fn article(
    feed: Option<curio_core::model::FeedId>,
    key: &str,
    title: &str,
    text: &str,
) -> NewArticle {
    NewArticle {
        feed_id: feed,
        dedupe_key: key.to_owned(),
        title: title.to_owned(),
        source_url: format!("https://example.com/{key}"),
        author: Some("Fixture Author".to_owned()),
        published_at: Some(Timestamp::now()),
        content: ArticleContent {
            html: format!("<p>{text}</p>"),
            text: text.to_owned(),
        },
        lang: Some("en".to_owned()),
        word_count: Some(u32::try_from(text.split_whitespace().count()).unwrap()),
        source_updated_at: None,
        lead_image: Some(format!("https://example.com/{key}.png")),
    }
}

#[test]
#[ignore = "fixture generator — run by hand at a release tag"]
fn write_fixture_db() {
    let out = std::env::var("CURIO_FIXTURE_OUT").expect("set CURIO_FIXTURE_OUT");
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        let storage = Storage::open(&path).unwrap();

        // Three feeds: tagged, retitled + paused, plain.
        let (rust, _) = storage
            .add_feed(NewFeed {
                url: "https://rust.example/feed.xml".to_owned(),
                title: Some("Rust Weekly".to_owned()),
                tags: vec!["tech".to_owned(), "tech/rust".to_owned()],
            })
            .unwrap();
        let (news, _) = storage
            .add_feed(NewFeed {
                url: "https://news.example/atom".to_owned(),
                title: None,
                tags: vec![],
            })
            .unwrap();
        let (_quiet, _) = storage
            .add_feed(NewFeed {
                url: "https://quiet.example/rss".to_owned(),
                title: Some("Quiet".to_owned()),
                tags: vec!["misc".to_owned()],
            })
            .unwrap();
        storage
            .set_feed_title(news.id, Some("Daily News".to_owned()))
            .unwrap();
        storage
            .set_feed_status(news.id, FeedStatus::Paused)
            .unwrap();
        storage
            .update_feed_fetch_state(
                rust.id,
                Some("\"etag-1\"".to_owned()),
                Some("Thu, 02 Jul 2026 00:00:00 GMT".to_owned()),
                Timestamp::now(),
            )
            .unwrap();
        for (feed, status, http, articles_new) in [
            (rust.id, FetchStatus::Ok, Some(200), 4),
            (rust.id, FetchStatus::NotModified, Some(304), 0),
            (news.id, FetchStatus::Error, Some(503), 0),
        ] {
            storage
                .record_fetch(FetchRecord {
                    feed_id: feed,
                    fetched_at: Timestamp::now(),
                    status,
                    http_status: http,
                    error: (status == FetchStatus::Error).then(|| "upstream 503".to_owned()),
                    // v0.6.0+ only (see the module docs).
                    error_kind: (status == FetchStatus::Error).then_some(FetchErrorKind::Http),
                    articles_new,
                    duration_ms: Some(42),
                })
                .unwrap();
        }

        // Ten articles: four per feed-ish plus two manual saves; distinct
        // searchable words so FTS can be probed after the upgrade.
        let mut batch = Vec::new();
        for i in 0..4 {
            batch.push(article(
                Some(rust.id),
                &format!("guid:rust-{i}"),
                &format!("Ownership part {i}"),
                &format!("borrowck lifetimes ferris article {i}"),
            ));
        }
        for i in 0..4 {
            batch.push(article(
                Some(news.id),
                &format!("guid:news-{i}"),
                &format!("Headline {i}"),
                &format!("parliament weather zebra story {i}"),
            ));
        }
        batch.push(article(
            None,
            "manual-1",
            "Saved essay",
            "quixotic manual essay",
        ));
        batch.push(article(
            None,
            "manual-2",
            "Café naïve",
            "unicode café naïve résumé",
        ));
        let outcome = storage.upsert_articles(batch).unwrap();
        assert_eq!(outcome.inserted, 10);

        let ids: Vec<_> = storage
            .list_articles(ListArticles::default())
            .unwrap()
            .into_iter()
            .map(|a| a.id)
            .collect();
        assert_eq!(ids.len(), 10);
        // A spread of state: read, starred, read-later, archived, tagged.
        storage.mark_read(ids[0], true).unwrap();
        storage.mark_read(ids[1], true).unwrap();
        storage.mark_read(ids[2], true).unwrap();
        storage.star_article(ids[0]).unwrap();
        storage.star_article(ids[5]).unwrap();
        storage.add_read_later(ids[3]).unwrap();
        storage.archive_article(ids[4]).unwrap();
        storage.tag_article(ids[0], "keep").unwrap();
        storage.tag_article(ids[0], "rust").unwrap();
        storage.tag_article(ids[8], "keep").unwrap();

        storage
            .set_setting("fixture.probe", "survives-upgrade")
            .unwrap();

        // Intents are left staged (no emitter ran): a real crash-before-
        // emit state the upgraded build must still be able to replay.
        assert!(!storage.pending_intents().unwrap().is_empty());
        storage.vacuum().unwrap();
    }
    // Fold the WAL into the main file so the fixture is one self-contained
    // file, then copy it out.
    let conn = rusqlite::Connection::open(&path).unwrap();
    let _: String = conn
        .query_row("PRAGMA journal_mode = DELETE", [], |row| row.get(0))
        .unwrap();
    drop(conn);
    std::fs::copy(&path, out).unwrap();
}
