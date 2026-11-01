//! Integration tests for the storage layer: migrations, repo round trips,
//! FTS trigger/rowid regression, and writer/reader concurrency.

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use curio_core::model::{
    ArticleContent, ArticleId, ContentOrigin, FeedId, FeedStatus, FetchErrorKind, FetchRecord,
    FetchStatus, NewArticle, NewFeed,
};
use curio_core::storage::{
    FETCH_LOG_KEEP, FeedSchedule, ListArticles, OpenError, Storage, StorageError,
};
use curio_types::{EventPayload, Timestamp};

fn temp_storage() -> (tempfile::TempDir, Storage) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path().join("curio.db")).unwrap();
    (dir, storage)
}

fn new_article(key: &str, title: &str, text: &str) -> NewArticle {
    NewArticle {
        feed_id: None,
        dedupe_key: key.to_owned(),
        title: title.to_owned(),
        source_url: format!("https://example.com/{key}"),
        author: None,
        published_at: None,
        content: ArticleContent {
            html: format!("<p>{text}</p>"),
            text: text.to_owned(),
        },
        lang: None,
        word_count: None,
        source_updated_at: None,
        lead_image: None,
    }
}

/// Inserts one article and returns its row id.
fn insert_one(storage: &Storage, key: &str, title: &str, text: &str) -> ArticleId {
    let outcome = storage
        .upsert_articles(vec![new_article(key, title, text)])
        .unwrap();
    assert_eq!(outcome.inserted, 1);
    storage
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .find(|a| a.dedupe_key == format!("m:{key}"))
        .unwrap()
        .id
}

// ------------------------------------------------------------- migrations

#[test]
fn fresh_database_reaches_the_current_schema_version() {
    let (_dir, storage) = temp_storage();
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version()
    );
}

#[test]
fn integrity_check_passes_on_a_healthy_database() {
    let (_dir, storage) = temp_storage();
    insert_one(&storage, "guid:1", "Title", "body text");
    storage.integrity_check().unwrap();
}

#[test]
fn migrations_are_idempotent_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    let first_dump = {
        let storage = Storage::open(&path).unwrap();
        storage.set_setting("probe", "survives").unwrap();
        storage.schema_dump().unwrap()
    };
    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.schema_dump().unwrap(), first_dump);
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version()
    );
    assert_eq!(
        storage.get_setting("probe").unwrap().as_deref(),
        Some("survives")
    );
}

#[test]
fn refuses_a_database_from_a_newer_build() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    drop(Storage::open(&path).unwrap());
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.pragma_update(None, "user_version", 9999).unwrap();
    }
    let err = Storage::open(&path).unwrap_err();
    assert!(matches!(err, OpenError::TooNew { found: 9999, .. }));
}

/// "Refused untouched" holds byte for byte, even for a rollback-journal
/// file the WAL switch would otherwise rewrite: the version check runs
/// before any pragma that writes.
#[test]
fn a_too_new_database_is_refused_byte_for_byte_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE from_the_future (x); INSERT INTO from_the_future VALUES (1);",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", Storage::schema_version() + 1)
            .unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let err = Storage::open(&path).unwrap_err();
    assert!(matches!(err, OpenError::TooNew { .. }), "{err:?}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["curio.db"], "no -wal/-shm sidecars, no backup");
}

/// The two "cannot use this file" cases a head must tell apart: a file
/// that is not a database at all, and a database from a newer build.
#[test]
fn garbage_and_too_new_databases_are_distinct_typed_errors() {
    let dir = tempfile::tempdir().unwrap();

    let garbage = dir.path().join("garbage.db");
    std::fs::write(
        &garbage,
        "this is a text file, not a SQLite database\n".repeat(64),
    )
    .unwrap();
    let err = Storage::open(&garbage).unwrap_err();
    assert!(matches!(err, OpenError::Corrupt { .. }), "{err:?}");
    assert!(err.to_string().contains("garbage.db"), "{err}");
    // Refused, not "repaired": the file is byte-for-byte untouched.
    assert!(
        std::fs::read_to_string(&garbage)
            .unwrap()
            .starts_with("this is a text file")
    );

    let newer = dir.path().join("newer.db");
    drop(Storage::open(&newer).unwrap());
    {
        let conn = rusqlite::Connection::open(&newer).unwrap();
        conn.pragma_update(None, "user_version", Storage::schema_version() + 1)
            .unwrap();
    }
    let err = Storage::open(&newer).unwrap_err();
    assert!(
        matches!(err, OpenError::TooNew { found, supported }
            if found == Storage::schema_version() + 1 && supported == Storage::schema_version()),
        "{err:?}"
    );
    // A refused open writes no backup.
    assert!(
        std::fs::read_dir(dir.path()).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".bak"))
    );
}

#[test]
fn an_unopenable_path_is_a_typed_io_error() {
    let dir = tempfile::tempdir().unwrap();
    // The parent directory does not exist: SQLite cannot create the file.
    let err = Storage::open(dir.path().join("missing").join("curio.db")).unwrap_err();
    assert!(matches!(err, OpenError::Io { .. }), "{err:?}");
}

#[test]
fn a_database_locked_by_another_process_is_a_typed_locked_error() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    // Another process (here: connection) in rollback-journal mode holding
    // an exclusive lock: switching to WAL needs that lock, so the open
    // waits out the busy timeout, then reports `Locked`.
    let holder = rusqlite::Connection::open(&path).unwrap();
    holder
        .execute_batch(
            "CREATE TABLE squatter (x); BEGIN EXCLUSIVE; INSERT INTO squatter VALUES (1);",
        )
        .unwrap();
    let err = Storage::open(&path).unwrap_err();
    assert!(matches!(err, OpenError::Locked { .. }), "{err:?}");
    assert!(err.to_string().contains("locked"), "{err}");
    holder.execute_batch("ROLLBACK").unwrap();
}

#[test]
fn fresh_database_schema_matches_the_committed_golden() {
    let (_dir, storage) = temp_storage();
    let dump = storage.schema_dump().unwrap();
    let golden_path = common::manifest_dir().join("tests/golden/schema_v1.txt");
    if std::env::var_os("REGEN_GOLDEN").is_some() {
        std::fs::write(&golden_path, &dump).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(&golden_path)
        .expect("golden schema missing — run with REGEN_GOLDEN=1 to create it");
    assert_eq!(
        dump, golden,
        "schema drifted from tests/golden/schema_v1.txt — if intentional, add a migration \
         and regenerate with REGEN_GOLDEN=1"
    );
}

// ------------------------------------------------------------------ feeds

#[test]
fn feed_lifecycle_round_trips_and_stages_events() {
    let (_dir, storage) = temp_storage();
    let (feed, added) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: Some("Example".to_owned()),
            tags: vec!["rust".to_owned()],
        })
        .unwrap();
    assert_eq!(feed.url, "https://example.com/feed.xml");
    assert_eq!(feed.status, FeedStatus::Active);
    assert_eq!(
        feed.tags,
        vec!["rust".to_owned()],
        "tags persist on the row, not only in the one-time event"
    );
    assert!(matches!(
        &added.event,
        EventPayload::FeedAdded { feed, feed_title, tags }
            if feed == "https://example.com/feed.xml"
                && feed_title.as_deref() == Some("Example")
                && tags == &["rust".to_owned()]
    ));

    // Subscribing to the same URL again is the typed duplicate, not a
    // generic SQLite failure — and stages no second feed.added.
    let err = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap_err();
    assert!(
        matches!(err, StorageError::Duplicate { entity: "feed", .. }),
        "{err:?}"
    );
    assert_eq!(err.to_string(), "feed already exists");
    assert_eq!(storage.pending_intents().unwrap().len(), 1);

    storage
        .set_feed_status(feed.id, FeedStatus::Paused)
        .unwrap();
    let fetched_at = Timestamp::now();
    storage
        .update_feed_fetch_state(
            feed.id,
            Some("etag-1".to_owned()),
            Some("Thu, 02 Jul 2026 00:00:00 GMT".to_owned()),
            fetched_at,
        )
        .unwrap();
    let reread = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(reread.status, FeedStatus::Paused);
    assert_eq!(reread.etag.as_deref(), Some("etag-1"));
    assert_eq!(reread.last_fetched_at, Some(fetched_at));
    assert_eq!(
        storage
            .get_feed_by_url("https://example.com/feed.xml")
            .unwrap()
            .unwrap()
            .id,
        feed.id
    );

    let removed = storage.remove_feed(feed.id).unwrap();
    assert!(matches!(
        &removed.event,
        EventPayload::FeedRemoved { feed } if feed == "https://example.com/feed.xml"
    ));
    assert_eq!(
        storage.list_feeds().unwrap(),
        [] as [curio_core::model::Feed; 0]
    );
    assert!(matches!(
        storage.remove_feed(feed.id),
        Err(StorageError::NotFound { entity: "feed" })
    ));
}

/// `set_feed_metadata` is the edit-feed modal's user-overwrite path — it
/// unconditionally replaces site URL / description (empty clears to NULL),
/// unlike the fetch-fill `update_feed_metadata`, which only COALESCE-fills a
/// NULL so a refresh never clobbers a human edit.
#[test]
fn set_feed_metadata_overwrites_where_update_only_fills() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: Some("Example".to_owned()),
            tags: vec![],
        })
        .unwrap();

    // A fresh subscription has no site URL / description.
    let reread = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(reread.site_url, None);
    assert_eq!(reread.description, None);

    // The user sets both through the edit-feed path.
    storage
        .set_feed_metadata(
            feed.id,
            Some("https://example.com".to_owned()),
            Some("A hand-written note".to_owned()),
        )
        .unwrap();
    let reread = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(reread.site_url.as_deref(), Some("https://example.com"));
    assert_eq!(reread.description.as_deref(), Some("A hand-written note"));

    // A later fetch fills (COALESCE) — it must NOT clobber the human edits.
    storage
        .update_feed_metadata(
            feed.id,
            None,
            Some("https://feed-provided.example".to_owned()),
            Some("Feed-provided blurb".to_owned()),
        )
        .unwrap();
    let reread = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(
        reread.site_url.as_deref(),
        Some("https://example.com"),
        "fetch-fill must not overwrite a user-set site URL"
    );
    assert_eq!(reread.description.as_deref(), Some("A hand-written note"));

    // But the user CAN overwrite again — and clear with an empty value.
    storage
        .set_feed_metadata(
            feed.id,
            Some("https://moved.example".to_owned()),
            Some(String::new()),
        )
        .unwrap();
    let reread = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(reread.site_url.as_deref(), Some("https://moved.example"));
    assert_eq!(reread.description, None, "an empty value clears the field");

    assert!(matches!(
        storage.set_feed_metadata(FeedId(9999), None, None),
        Err(StorageError::NotFound { entity: "feed" })
    ));
}

/// The feed URL is the identity key of feed.added/feed.removed: adopting
/// a permanent redirect must stage the negation pair, or a consumer's
/// fold keeps a phantom subscription under the original URL forever
/// (the eventual feed.removed would carry the new URL and negate
/// nothing).
#[test]
fn adopting_a_redirect_stages_the_removed_added_pair() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://old.example/feed.xml".to_owned(),
            title: Some("Example".to_owned()),
            tags: vec!["rust".to_owned()],
        })
        .unwrap();
    storage
        .mark_intents_emitted(
            storage
                .pending_intents()
                .unwrap()
                .iter()
                .map(|p| p.intent_id)
                .collect(),
        )
        .unwrap();

    // Same URL: no-op, no events.
    storage
        .update_feed_url(feed.id, "https://old.example/feed.xml".to_owned())
        .unwrap();
    assert_eq!(
        storage.pending_intents().unwrap(),
        [] as [curio_core::storage::PendingIntent; 0]
    );

    // Real adoption: removed{old} then added{new} with title + tags.
    storage
        .update_feed_url(feed.id, "https://new.example/feed.xml".to_owned())
        .unwrap();
    let pending = storage.pending_intents().unwrap();
    assert_eq!(pending.len(), 2);
    assert!(matches!(
        &pending[0].envelope.event,
        EventPayload::FeedRemoved { feed } if feed == "https://old.example/feed.xml"
    ));
    assert!(matches!(
        &pending[1].envelope.event,
        EventPayload::FeedAdded { feed, feed_title, tags }
            if feed == "https://new.example/feed.xml"
                && feed_title.as_deref() == Some("Example")
                && tags == &["rust".to_owned()]
    ));
    assert_eq!(
        storage.get_feed(feed.id).unwrap().unwrap().url,
        "https://new.example/feed.xml"
    );

    // Conflict with another subscription: OR IGNORE keeps the URL and
    // stages nothing (the folded membership stays correct).
    storage
        .add_feed(NewFeed {
            url: "https://taken.example/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let before = storage.pending_intents().unwrap().len();
    storage
        .update_feed_url(feed.id, "https://taken.example/feed.xml".to_owned())
        .unwrap();
    assert_eq!(storage.pending_intents().unwrap().len(), before);
    assert_eq!(
        storage.get_feed(feed.id).unwrap().unwrap().url,
        "https://new.example/feed.xml"
    );
}

#[test]
fn removing_a_feed_orphans_articles_instead_of_deleting_them() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let mut article = new_article("a1", "Kept", "body");
    article.feed_id = Some(feed.id);
    storage.upsert_articles(vec![article]).unwrap();
    storage.remove_feed(feed.id).unwrap();
    let articles = storage.list_articles(ListArticles::default()).unwrap();
    assert_eq!(articles.len(), 1);
    assert_eq!(articles[0].feed_id, None);
}

// --------------------------------------------------------------- articles

#[test]
fn upsert_inserts_then_updates_by_dedupe_key() {
    let (_dir, storage) = temp_storage();
    let first = storage
        .upsert_articles(vec![new_article("k1", "Original", "old words")])
        .unwrap();
    assert_eq!((first.inserted, first.updated), (1, 0));
    let original = &storage.list_articles(ListArticles::default()).unwrap()[0];

    let second = storage
        .upsert_articles(vec![new_article("k1", "Revised", "new words")])
        .unwrap();
    assert_eq!((second.inserted, second.updated), (0, 1));
    let revised = storage.get_article(original.id).unwrap().unwrap();
    // Identity and saved_at survive the update; content refreshes.
    assert_eq!(revised.curio_id, original.curio_id);
    assert_eq!(revised.saved_at, original.saved_at);
    assert_eq!(revised.title, "Revised");
    assert_eq!(revised.content.text, "new words");
    assert_eq!(storage.count_articles().unwrap(), 1);
    assert_eq!(
        storage
            .get_article_by_curio_id(original.curio_id)
            .unwrap()
            .unwrap()
            .id,
        original.id
    );
}

/// Two subscriptions carrying the same guid (sequential-integer guids are
/// common in broken feeds, and guids are attacker-choosable) must stay two
/// distinct articles: a refresh of one feed can never overwrite the
/// other's content, identity, or FTS entry.
#[test]
fn identical_guids_from_different_feeds_stay_distinct_articles() {
    let (_dir, storage) = temp_storage();
    let (feed_a, _) = storage
        .add_feed(NewFeed {
            url: "https://a.example/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let (feed_b, _) = storage
        .add_feed(NewFeed {
            url: "https://b.example/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();

    let mut from_a = new_article("guid:1", "A's story", "alpha content");
    from_a.feed_id = Some(feed_a.id);
    let mut from_b = new_article("guid:1", "B's story", "beta content");
    from_b.feed_id = Some(feed_b.id);

    let first = storage.upsert_articles(vec![from_a.clone()]).unwrap();
    let second = storage.upsert_articles(vec![from_b.clone()]).unwrap();
    assert_eq!((first.inserted, second.inserted), (1, 1));
    assert_eq!(storage.count_articles().unwrap(), 2);

    // Refreshing B updates only B's row; A is untouched.
    from_b.content.text = "beta content revised".to_owned();
    let refresh = storage.upsert_articles(vec![from_b]).unwrap();
    assert_eq!((refresh.inserted, refresh.updated), (0, 1));
    let articles = storage.list_articles(ListArticles::default()).unwrap();
    let a_row = articles
        .iter()
        .find(|a| a.feed_id == Some(feed_a.id))
        .unwrap();
    let b_row = articles
        .iter()
        .find(|a| a.feed_id == Some(feed_b.id))
        .unwrap();
    assert_eq!(a_row.title, "A's story");
    assert_eq!(a_row.content.text, "alpha content");
    assert_eq!(b_row.content.text, "beta content revised");
    assert_ne!(a_row.curio_id, b_row.curio_id);

    // The same collision on the manual-save namespace: a manual save never
    // matches a feed article's key either.
    let manual = new_article("guid:1", "Manual", "gamma");
    storage.upsert_articles(vec![manual]).unwrap();
    assert_eq!(storage.count_articles().unwrap(), 3);
}

/// Migration 0002 rewrites pre-existing rows with the same provenance
/// scoping the repo now applies on insert, so a v1 database refreshes
/// without duplicating its whole article set.
#[test]
fn migration_0002_scopes_legacy_dedupe_keys() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        // Build a v1-shaped database by hand: schema 0001, legacy rows,
        // user_version = 1.
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(include_str!("../src/storage/migrations/0001_init.sql"))
            .unwrap();
        conn.execute_batch(
            "INSERT INTO feeds (id, url, added_at, modified_at)
             VALUES (7, 'https://a.example/feed.xml', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z');
             INSERT INTO articles (curio_id, feed_id, dedupe_key, title, source_url,
                                   saved_at, modified_at)
             VALUES ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d', 7, 'guid:1', 'From feed',
                     'https://a.example/1', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z'),
                    ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6e', NULL, 'guid:2', 'Manual',
                     'https://m.example/2', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 1).unwrap();
    }

    let storage = Storage::open(&path).unwrap();
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version()
    );
    let mut keys: Vec<String> = storage
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .map(|a| a.dedupe_key)
        .collect();
    keys.sort();
    assert_eq!(keys, vec!["f7:guid:1".to_owned(), "m:guid:2".to_owned()]);

    // And the migrated feed article still matches its own refresh.
    let mut refresh = new_article("guid:1", "From feed (updated)", "text");
    refresh.feed_id = Some(FeedId(7));
    let outcome = storage.upsert_articles(vec![refresh]).unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (0, 1));
}

#[test]
fn list_articles_paginates_by_keyset() {
    let (_dir, storage) = temp_storage();
    let batch: Vec<NewArticle> = (0..10)
        .map(|i| new_article(&format!("k{i}"), &format!("T{i}"), "text"))
        .collect();
    storage.upsert_articles(batch).unwrap();
    let page1 = storage
        .list_articles(ListArticles {
            limit: 4,
            ..ListArticles::default()
        })
        .unwrap();
    assert_eq!(page1.len(), 4);
    let page2 = storage
        .list_articles(ListArticles {
            limit: 4,
            before: Some(page1.last().unwrap().id),
            ..ListArticles::default()
        })
        .unwrap();
    assert_eq!(page2.len(), 4);
    assert!(page1.last().unwrap().id > page2.first().unwrap().id);
    let all: Vec<_> = storage.list_articles(ListArticles::default()).unwrap();
    assert_eq!(all.len(), 10);
}

/// Adds a feed subscription (test shorthand).
fn add_feed(storage: &Storage, url: &str) -> curio_core::model::Feed {
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: url.to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    feed
}

/// Finds an article's row id by its (scoped) dedupe key.
fn id_by_key(storage: &Storage, scoped_key: &str) -> ArticleId {
    storage
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .find(|a| a.dedupe_key == scoped_key)
        .unwrap()
        .id
}

#[test]
fn list_articles_filters_by_read_state_flags() {
    let (_dir, storage) = temp_storage();
    let batch: Vec<NewArticle> = (0..4)
        .map(|i| new_article(&format!("k{i}"), &format!("T{i}"), "text"))
        .collect();
    storage.upsert_articles(batch).unwrap();
    let k0 = id_by_key(&storage, "m:k0");
    let k1 = id_by_key(&storage, "m:k1");
    storage.mark_read(k0, true).unwrap();
    storage.star_article(k0).unwrap();
    storage.add_read_later(k1).unwrap();
    storage.archive_article(k1).unwrap();

    let list = |params: ListArticles| -> Vec<String> {
        storage
            .list_articles(params)
            .unwrap()
            .into_iter()
            .map(|a| a.dedupe_key)
            .collect()
    };

    // Articles with no article_state row count as unread/unstarred/….
    let unread = list(ListArticles {
        read: Some(false),
        ..ListArticles::default()
    });
    assert_eq!(unread.len(), 3);
    assert!(!unread.contains(&"m:k0".to_owned()));
    assert_eq!(
        list(ListArticles {
            read: Some(true),
            ..ListArticles::default()
        }),
        vec!["m:k0".to_owned()]
    );
    assert_eq!(
        list(ListArticles {
            starred: Some(true),
            ..ListArticles::default()
        }),
        vec!["m:k0".to_owned()]
    );
    assert_eq!(
        list(ListArticles {
            read_later: Some(true),
            ..ListArticles::default()
        }),
        vec!["m:k1".to_owned()]
    );
    assert_eq!(
        list(ListArticles {
            archived: Some(false),
            ..ListArticles::default()
        })
        .len(),
        3
    );

    // Filters compose (AND): starred AND read.
    assert_eq!(
        list(ListArticles {
            read: Some(true),
            starred: Some(true),
            ..ListArticles::default()
        }),
        vec!["m:k0".to_owned()]
    );
    assert_eq!(
        list(ListArticles {
            read: Some(false),
            starred: Some(true),
            ..ListArticles::default()
        }),
        [] as [std::string::String; 0]
    );
}

#[test]
fn list_articles_filters_by_tag_without_reordering_the_keyset() {
    let (_dir, storage) = temp_storage();
    let feed = add_feed(&storage, "https://a.example/feed.xml");
    let batch: Vec<NewArticle> = (0..6)
        .map(|i| {
            let mut article = new_article(&format!("k{i}"), &format!("T{i}"), "text");
            article.feed_id = Some(feed.id);
            article
        })
        .collect();
    storage.upsert_articles(batch).unwrap();
    for i in [0, 2, 4] {
        let id = id_by_key(&storage, &format!("f{}:k{i}", feed.id.0));
        storage.tag_article(id, "rust").unwrap();
    }

    // Tag filter (trimmed like the producer side), keyset order intact.
    let tagged = storage
        .list_articles(ListArticles {
            tag: Some("  rust  ".to_owned()),
            ..ListArticles::default()
        })
        .unwrap();
    let keys: Vec<&str> = tagged.iter().map(|a| a.dedupe_key.as_str()).collect();
    let feed_id = feed.id.0;
    assert_eq!(
        keys,
        vec![
            format!("f{feed_id}:k4"),
            format!("f{feed_id}:k2"),
            format!("f{feed_id}:k0")
        ],
        "id DESC, filter does not reorder"
    );

    // Keyset pagination composes with the filter.
    let page1 = storage
        .list_articles(ListArticles {
            tag: Some("rust".to_owned()),
            feed_id: Some(feed.id),
            limit: 2,
            ..ListArticles::default()
        })
        .unwrap();
    assert_eq!(page1.len(), 2);
    let page2 = storage
        .list_articles(ListArticles {
            tag: Some("rust".to_owned()),
            feed_id: Some(feed.id),
            limit: 2,
            before: Some(page1.last().unwrap().id),
            ..ListArticles::default()
        })
        .unwrap();
    assert_eq!(page2.len(), 1);
    assert!(page1.last().unwrap().id > page2[0].id);

    // Unknown tag → empty, not an error.
    assert_eq!(
        storage
            .list_articles(ListArticles {
                tag: Some("nope".to_owned()),
                ..ListArticles::default()
            })
            .unwrap(),
        [] as [curio_core::model::Article; 0]
    );
}

#[test]
fn unread_counts_group_by_feed_and_follow_read_flips() {
    let (_dir, storage) = temp_storage();
    assert!(
        storage.unread_counts().unwrap().is_empty(),
        "an empty database has no counts"
    );

    let feed_a = add_feed(&storage, "https://a.example/feed.xml");
    let feed_b = add_feed(&storage, "https://b.example/feed.xml");
    let mut a1 = new_article("a1", "A1", "alpha one");
    a1.feed_id = Some(feed_a.id);
    let mut a2 = new_article("a2", "A2", "alpha two");
    a2.feed_id = Some(feed_a.id);
    let mut b1 = new_article("b1", "B1", "beta one");
    b1.feed_id = Some(feed_b.id);
    let manual = new_article("m1", "Manual", "gamma");
    storage.upsert_articles(vec![a1, a2, b1, manual]).unwrap();

    // Never-touched articles (no article_state row) are unread.
    let counts = storage.unread_counts().unwrap();
    assert_eq!(counts.get(&Some(feed_a.id)), Some(&2));
    assert_eq!(counts.get(&Some(feed_b.id)), Some(&1));
    assert_eq!(counts.get(&None), Some(&1), "feedless articles count too");
    assert_eq!(counts.values().sum::<u64>(), 4, "total = sum of the map");

    // Reading moves the count; starring does not.
    let a1_id = id_by_key(&storage, &format!("f{}:a1", feed_a.id.0));
    let b1_id = id_by_key(&storage, &format!("f{}:b1", feed_b.id.0));
    storage.mark_read(a1_id, true).unwrap();
    storage.star_article(b1_id).unwrap();
    let counts = storage.unread_counts().unwrap();
    assert_eq!(counts.get(&Some(feed_a.id)), Some(&1));
    assert_eq!(
        counts.get(&Some(feed_b.id)),
        Some(&1),
        "starring must not affect unreadness"
    );

    // Archiving DOES take an article out of the unread count (out of the flow).
    storage.archive_article(b1_id).unwrap();
    assert_eq!(
        storage.unread_counts().unwrap().get(&Some(feed_b.id)),
        None,
        "an archived (still-unread) article leaves the unread count"
    );

    // Unreading restores; a fully-read feed drops out of the map.
    storage.mark_read(a1_id, false).unwrap();
    assert_eq!(
        storage.unread_counts().unwrap().get(&Some(feed_a.id)),
        Some(&2)
    );
    storage.mark_read(a1_id, true).unwrap();
    let a2_id = id_by_key(&storage, &format!("f{}:a2", feed_a.id.0));
    storage.mark_read(a2_id, true).unwrap();
    let counts = storage.unread_counts().unwrap();
    assert_eq!(counts.get(&Some(feed_a.id)), None, "no unread → no key");
    assert_eq!(
        counts.values().sum::<u64>(),
        1,
        "feed_a fully read, feed_b's only article archived — just the manual one remains"
    );
}

#[test]
fn mark_all_read_scopes_to_one_feed() {
    let (_dir, storage) = temp_storage();
    let feed_a = add_feed(&storage, "https://a.example/feed.xml");
    let feed_b = add_feed(&storage, "https://b.example/feed.xml");
    let mut a1 = new_article("a1", "A1", "alpha one");
    a1.feed_id = Some(feed_a.id);
    let mut a2 = new_article("a2", "A2", "alpha two");
    a2.feed_id = Some(feed_a.id);
    let mut b1 = new_article("b1", "B1", "beta one");
    b1.feed_id = Some(feed_b.id);
    storage.upsert_articles(vec![a1, a2, b1]).unwrap();

    // Marking feed A read flips both of its articles; feed B is untouched.
    assert_eq!(storage.mark_all_read(Some(feed_a.id)).unwrap().len(), 2);
    let counts = storage.unread_counts().unwrap();
    assert_eq!(counts.get(&Some(feed_a.id)), None, "feed A fully read");
    assert_eq!(counts.get(&Some(feed_b.id)), Some(&1), "feed B untouched");

    // A second sweep of A changes nothing.
    assert_eq!(storage.mark_all_read(Some(feed_a.id)).unwrap(), Vec::new());
}

#[test]
fn mark_all_read_returns_exactly_the_flipped_ids_and_undo_restores_them() {
    let (_dir, storage) = temp_storage();
    let feed = add_feed(&storage, "https://undo.example/feed.xml");
    let mut rows = Vec::new();
    for key in ["u1", "u2", "u3"] {
        let mut article = new_article(key, key, "body");
        article.feed_id = Some(feed.id);
        rows.push(article);
    }
    // A feedless (manually saved) row is only in the library-wide sweep.
    rows.push(new_article("manual", "Manual", "saved by hand"));
    storage.upsert_articles(rows).unwrap();
    let all = storage.list_articles(ListArticles::default()).unwrap();
    let id_of = |key: &str| {
        all.iter()
            .find(|article| article.title == key || article.title == "Manual" && key == "manual")
            .unwrap()
            .id
    };
    let (u1, u2, u3, manual) = (id_of("u1"), id_of("u2"), id_of("u3"), id_of("manual"));
    // u2 is already read: it is not part of the sweep, so undo must leave it read.
    assert!(storage.mark_read(u2, true).unwrap());

    let mut expected = vec![u1, u3, manual];
    expected.sort_unstable_by_key(|id| id.0);
    let flipped = storage.mark_all_read(None).unwrap();
    assert_eq!(
        flipped, expected,
        "only the previously-unread ids, ascending"
    );

    // Undo: exactly the snapshot goes back to unread; u2 stays read.
    assert_eq!(storage.mark_read_many(&flipped, false).unwrap(), 3);
    for id in [u1, u3, manual] {
        assert!(!storage.article_state(id).unwrap().read, "{id:?} restored");
    }
    assert!(
        storage.article_state(u2).unwrap().read,
        "u2 was read before"
    );

    // Idempotent: a second undo changes nothing; unknown ids are skipped.
    assert_eq!(storage.mark_read_many(&flipped, false).unwrap(), 0);
    assert_eq!(
        storage
            .mark_read_many(&[ArticleId(9_999_999), u1], true)
            .unwrap(),
        1,
        "a purged id is skipped, the live one still flips"
    );
    assert!(storage.article_state(u1).unwrap().read);
    assert_eq!(storage.mark_read_many(&[], true).unwrap(), 0);
}

#[test]
fn feed_tag_filter_scopes_articles_to_the_folder_subtree() {
    let (_dir, storage) = temp_storage();
    let feed = |url: &str, tag: &str| {
        storage
            .add_feed(NewFeed {
                url: url.to_owned(),
                title: None,
                tags: vec![tag.to_owned()],
            })
            .unwrap()
            .0
    };
    let tech = feed("https://tech.example/f", "Tech");
    let db = feed("https://db.example/f", "Tech/Databases");
    let cook = feed("https://cook.example/f", "Cooking");
    let article = |key: &str, feed_id| {
        let mut a = new_article(key, key, "body");
        a.feed_id = Some(feed_id);
        a
    };
    storage
        .upsert_articles(vec![
            article("tech", tech.id),
            article("db", db.id),
            article("cook", cook.id),
        ])
        .unwrap();

    let titles = |feed_tag: &str| {
        let mut t = storage
            .list_articles(ListArticles {
                feed_tag: Some(feed_tag.to_owned()),
                ..Default::default()
            })
            .unwrap()
            .into_iter()
            .map(|a| a.title)
            .collect::<Vec<_>>();
        t.sort();
        t
    };

    // A top-level folder covers its own feed AND nested subfolders.
    assert_eq!(titles("Tech"), vec!["db".to_owned(), "tech".to_owned()]);
    // A nested folder is narrower; a sibling folder is disjoint.
    assert_eq!(titles("Tech/Databases"), vec!["db".to_owned()]);
    assert_eq!(titles("Cooking"), vec!["cook".to_owned()]);
    // `Tec` must NOT match `Tech` (prefix guarded by the `/` boundary).
    assert_eq!(titles("Tec"), [] as [std::string::String; 0]);
}

// ------------------------------------------------------- state and events

#[test]
fn state_flips_are_idempotent_and_stage_intents_in_order() {
    let (_dir, storage) = temp_storage();
    let id = insert_one(&storage, "k1", "Title", "body");

    let starred = storage.star_article(id).unwrap().unwrap();
    let article = storage.get_article(id).unwrap().unwrap();
    assert!(matches!(
        &starred.event,
        EventPayload::ArticleStarred { curio_id, tags }
            if *curio_id == article.curio_id && tags.is_empty()
    ));
    // Idempotent: already starred → no event.
    assert!(storage.star_article(id).unwrap().is_none());
    assert!(storage.article_state(id).unwrap().starred);

    let unstarred = storage.unstar_article(id).unwrap().unwrap();
    assert!(matches!(
        &unstarred.event,
        EventPayload::ArticleUnstarred { .. }
    ));
    assert!(storage.unstar_article(id).unwrap().is_none());
    assert!(!storage.article_state(id).unwrap().starred);

    assert!(storage.add_read_later(id).unwrap().is_some());
    assert!(storage.add_read_later(id).unwrap().is_none());
    assert!(storage.remove_read_later(id).unwrap().is_some());
    assert!(storage.archive_article(id).unwrap().is_some());
    assert!(storage.unarchive_article(id).unwrap().is_some());

    // mark_read is DB-local: no event staged.
    assert!(storage.mark_read(id, true).unwrap());
    assert!(!storage.mark_read(id, true).unwrap());
    assert!(storage.article_state(id).unwrap().read);

    // opened always stages.
    let opened = storage.record_opened(id, Some(1500)).unwrap();
    assert!(matches!(
        &opened.event,
        EventPayload::ArticleOpened {
            dwell_ms: Some(1500),
            ..
        }
    ));

    let pending = storage.pending_intents().unwrap();
    let types: Vec<&str> = pending
        .iter()
        .map(|p| p.envelope.event.event_type())
        .collect();
    assert_eq!(
        types,
        [
            "article.starred",
            "article.unstarred",
            "article.read_later.added",
            "article.read_later.removed",
            "article.archived",
            "article.unarchived",
            "article.opened",
        ]
    );

    storage
        .mark_intents_emitted(pending.iter().map(|p| p.intent_id).collect())
        .unwrap();
    assert_eq!(
        storage.pending_intents().unwrap(),
        [] as [curio_core::storage::PendingIntent; 0]
    );
}

#[test]
fn state_events_carry_tags_in_payload() {
    let (_dir, storage) = temp_storage();
    let id = insert_one(&storage, "k1", "Title", "body");
    assert!(storage.tag_article(id, "rust").unwrap().is_some());
    assert!(storage.tag_article(id, "db").unwrap().is_some());
    // Duplicate tag: no event.
    assert!(storage.tag_article(id, "rust").unwrap().is_none());

    let starred = storage.star_article(id).unwrap().unwrap();
    assert!(matches!(
        &starred.event,
        EventPayload::ArticleStarred { tags, .. }
            if tags == &["db".to_owned(), "rust".to_owned()]
    ));

    let untagged = storage.untag_article(id, "db").unwrap().unwrap();
    assert!(matches!(
        &untagged.event,
        EventPayload::ArticleUntagged { tag, .. } if tag == "db"
    ));
    assert!(storage.untag_article(id, "db").unwrap().is_none());
    assert_eq!(storage.article_tags(id).unwrap(), vec!["rust".to_owned()]);
    assert_eq!(
        storage.list_tags().unwrap(),
        vec!["db".to_owned(), "rust".to_owned()]
    );
}

/// The published schemas require tags to be non-empty (`minLength: 1`);
/// the producer must refuse what its own consumers would reject.
#[test]
fn empty_and_whitespace_tags_are_refused_and_inputs_are_trimmed() {
    let (_dir, storage) = temp_storage();
    let id = insert_one(&storage, "k1", "Title", "body");

    for bad in ["", " ", "\t", "\n  "] {
        assert!(matches!(
            storage.tag_article(id, bad),
            Err(StorageError::InvalidTag)
        ));
        assert!(matches!(
            storage.untag_article(id, bad),
            Err(StorageError::InvalidTag)
        ));
    }
    assert!(
        storage.pending_intents().unwrap().is_empty(),
        "no event staged"
    );

    // Accidental surrounding whitespace normalizes to the same tag.
    assert!(storage.tag_article(id, " rust ").unwrap().is_some());
    assert!(storage.tag_article(id, "rust").unwrap().is_none());
    assert_eq!(storage.article_tags(id).unwrap(), vec!["rust".to_owned()]);
    assert!(storage.untag_article(id, " rust").unwrap().is_some());

    // Feed tags normalize (trim, drop empties, dedupe) instead of erroring:
    // OPML categories are third-party input.
    let (_feed, added) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![
                String::new(),
                "  ".to_owned(),
                " rust ".to_owned(),
                "rust".to_owned(),
                "db".to_owned(),
            ],
        })
        .unwrap();
    assert!(matches!(
        &added.event,
        EventPayload::FeedAdded { tags, .. }
            if tags == &["rust".to_owned(), "db".to_owned()]
    ));
}

#[test]
fn state_ops_on_a_missing_article_report_not_found() {
    let (_dir, storage) = temp_storage();
    let ghost = ArticleId(999);
    assert!(matches!(
        storage.star_article(ghost),
        Err(StorageError::NotFound { entity: "article" })
    ));
    assert!(matches!(
        storage.tag_article(ghost, "x"),
        Err(StorageError::NotFound { entity: "article" })
    ));
    assert!(matches!(
        storage.record_opened(ghost, None),
        Err(StorageError::NotFound { entity: "article" })
    ));
}

// ------------------------------------------------- fetch log and settings

#[test]
fn fetch_log_round_trips_newest_first() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    for (status, http) in [
        (FetchStatus::Ok, Some(200)),
        (FetchStatus::NotModified, Some(304)),
        (FetchStatus::Error, None),
    ] {
        storage
            .record_fetch(FetchRecord {
                feed_id: feed.id,
                fetched_at: Timestamp::now(),
                status,
                http_status: http,
                error: (status == FetchStatus::Error).then(|| "timeout".to_owned()),
                error_kind: (status == FetchStatus::Error).then_some(FetchErrorKind::ReadTimeout),
                articles_new: 0,
                duration_ms: Some(42),
            })
            .unwrap();
    }
    let recent = storage.recent_fetches(feed.id, 2).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].status, FetchStatus::Error);
    assert_eq!(recent[0].error.as_deref(), Some("timeout"));
    assert_eq!(recent[0].error_kind, Some(FetchErrorKind::ReadTimeout));
    assert_eq!(recent[1].error_kind, None);
    assert_eq!(recent[1].status, FetchStatus::NotModified);
    // Unknown feed violates the foreign key.
    assert!(
        storage
            .record_fetch(FetchRecord {
                feed_id: FeedId(999),
                fetched_at: Timestamp::now(),
                status: FetchStatus::Ok,
                http_status: Some(200),
                error: None,
                error_kind: None,
                articles_new: 0,
                duration_ms: None,
            })
            .is_err()
    );
}

fn attempt(feed_id: FeedId, status: FetchStatus) -> FetchRecord {
    FetchRecord {
        feed_id,
        fetched_at: Timestamp::now(),
        status,
        http_status: None,
        error: (status == FetchStatus::Error).then(|| "boom".to_owned()),
        error_kind: (status == FetchStatus::Error).then_some(FetchErrorKind::Http),
        articles_new: 0,
        duration_ms: None,
    }
}

/// However often a feed is refreshed, its fetch log keeps only the newest
/// rows — and trimming one feed's log never touches another's.
#[test]
fn the_fetch_log_stays_bounded_per_feed() {
    let (_dir, storage) = temp_storage();
    let add = |url: &str| {
        storage
            .add_feed(NewFeed {
                url: url.to_owned(),
                title: None,
                tags: vec![],
            })
            .unwrap()
            .0
    };
    let busy = add("https://busy.example/feed.xml");
    let quiet = add("https://quiet.example/feed.xml");
    storage
        .record_fetch(attempt(quiet.id, FetchStatus::Ok))
        .unwrap();
    for n in 0..500 {
        let status = if n % 2 == 0 {
            FetchStatus::Ok
        } else {
            FetchStatus::Error
        };
        storage.record_fetch(attempt(busy.id, status)).unwrap();
    }
    let keep = usize::try_from(FETCH_LOG_KEEP).unwrap();
    assert_eq!(storage.recent_fetches(busy.id, 1_000).unwrap().len(), keep);
    assert_eq!(storage.recent_fetches(quiet.id, 1_000).unwrap().len(), 1);
    // The derived health still reads the newest attempt.
    let busy = storage.get_feed(busy.id).unwrap().unwrap();
    assert_eq!(busy.last_error.as_deref(), Some("boom"));
    assert!(busy.last_ok_at.is_some());
}

/// `settle_fetch` writes the log row and the feed's schedule together;
/// switching the feed back to active forgets the failure run.
#[test]
fn settling_a_fetch_stores_the_schedule_and_resuming_clears_it() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let now = Timestamp::now();
    let later = Timestamp::from_unix_millis(now.unix_millis() + 60_000).unwrap();
    let failing = FeedSchedule {
        consecutive_failures: 3,
        next_attempt_at: Some(later),
        failing_since: Some(now),
        status: None,
    };
    storage
        .settle_fetch(attempt(feed.id, FetchStatus::Error), failing)
        .unwrap();
    let stored = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.consecutive_failures, 3);
    assert_eq!(stored.next_attempt_at, Some(later));
    assert_eq!(
        stored.last_error_kind,
        Some(FetchErrorKind::Http),
        "from the log row"
    );
    assert_eq!(stored.failing_since, Some(now));
    assert_eq!(stored.status, FeedStatus::Active);
    assert_eq!(stored.auto_paused_at, None);
    assert!(!stored.is_due(now));
    assert_eq!(storage.recent_fetches(feed.id, 10).unwrap().len(), 1);

    // An auto-pause stamps when it happened.
    let record = attempt(feed.id, FetchStatus::Error);
    let paused_at = record.fetched_at;
    storage
        .settle_fetch(
            record,
            FeedSchedule {
                status: Some(FeedStatus::Paused),
                ..failing
            },
        )
        .unwrap();
    let stored = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.status, FeedStatus::Paused);
    assert_eq!(stored.auto_paused_at, Some(paused_at));

    // Resume: a clean slate, due at once.
    storage
        .set_feed_status(feed.id, FeedStatus::Active)
        .unwrap();
    let stored = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.consecutive_failures, 0);
    assert_eq!(stored.next_attempt_at, None);
    assert_eq!(stored.failing_since, None);
    assert_eq!(stored.auto_paused_at, None);
    assert!(stored.is_due(now));
    // The log is history: the last attempt's error (and its kind) still
    // shows until the next fetch.
    assert_eq!(stored.last_error_kind, Some(FetchErrorKind::Http));

    // A user pause keeps the schedule but is never marked automatic.
    storage
        .settle_fetch(attempt(feed.id, FetchStatus::Error), failing)
        .unwrap();
    storage
        .set_feed_status(feed.id, FeedStatus::Paused)
        .unwrap();
    let stored = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(stored.consecutive_failures, 3);
    assert_eq!(stored.auto_paused_at, None);

    assert!(matches!(
        storage.settle_fetch(attempt(FeedId(999), FetchStatus::Ok), failing),
        Err(StorageError::NotFound { entity: "feed" })
    ));
}

#[test]
fn a_feed_refresh_interval_round_trips() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    assert_eq!(feed.refresh_interval_min, None);
    storage
        .set_feed_refresh_interval(feed.id, Some(90))
        .unwrap();
    assert_eq!(
        storage
            .get_feed(feed.id)
            .unwrap()
            .unwrap()
            .refresh_interval_min,
        Some(90)
    );
    storage.set_feed_refresh_interval(feed.id, None).unwrap();
    assert_eq!(
        storage
            .get_feed(feed.id)
            .unwrap()
            .unwrap()
            .refresh_interval_min,
        None
    );
    assert!(matches!(
        storage.set_feed_refresh_interval(FeedId(999), Some(5)),
        Err(StorageError::NotFound { entity: "feed" })
    ));
}

#[test]
fn feed_health_derives_from_the_latest_fetch_outcome() {
    let (_dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    let only = |s: &Storage| s.list_feeds().unwrap().pop().unwrap();

    // Cold: never fetched ⇒ no error, no success time (the healthy default).
    let fresh = only(&storage);
    assert_eq!(fresh.last_error, None);
    assert_eq!(fresh.last_ok_at, None);

    let record = |status: FetchStatus, error: Option<&str>| {
        storage
            .record_fetch(FetchRecord {
                feed_id: feed.id,
                fetched_at: Timestamp::now(),
                status,
                http_status: None,
                error: error.map(str::to_owned),
                error_kind: error.map(|_| FetchErrorKind::Refused),
                articles_new: 0,
                duration_ms: None,
            })
            .unwrap();
    };

    // A success stamps last_ok_at and keeps last_error clear.
    record(FetchStatus::Ok, None);
    let healthy = only(&storage);
    assert_eq!(healthy.last_error, None);
    let ok_at = healthy.last_ok_at;
    assert!(ok_at.is_some());

    // A later error surfaces on last_error; last_ok_at still points at the success.
    record(FetchStatus::Error, Some("connection refused"));
    let errored = only(&storage);
    assert_eq!(errored.last_error.as_deref(), Some("connection refused"));
    assert_eq!(errored.last_error_kind, Some(FetchErrorKind::Refused));
    assert_eq!(errored.last_ok_at, ok_at);

    // Recovery clears last_error again — the dot follows the newest attempt.
    record(FetchStatus::Ok, None);
    let recovered = only(&storage);
    assert_eq!(recovered.last_error, None);
    assert_eq!(recovered.last_error_kind, None);
}

/// Kinds are added without a migration, so a profile a newer build wrote
/// can carry one this build does not know: it reads as no kind — never as
/// a corrupt row — and the human error text still comes through.
#[test]
fn an_unknown_stored_error_kind_reads_as_none() {
    let (dir, storage) = temp_storage();
    let (feed, _) = storage
        .add_feed(NewFeed {
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap();
    storage
        .record_fetch(FetchRecord {
            feed_id: feed.id,
            fetched_at: Timestamp::now(),
            status: FetchStatus::Error,
            http_status: None,
            error: Some("something new went wrong".to_owned()),
            error_kind: Some(FetchErrorKind::Refused),
            articles_new: 0,
            duration_ms: None,
        })
        .unwrap();
    side_conn(&dir)
        .execute(
            "UPDATE fetch_log SET error_kind = 'kind_from_a_newer_build'",
            [],
        )
        .unwrap();

    let listed = storage.list_feeds().unwrap().pop().unwrap();
    assert_eq!(listed.last_error_kind, None);
    assert_eq!(
        listed.last_error.as_deref(),
        Some("something new went wrong")
    );
    let recent = storage.recent_fetches(feed.id, 1).unwrap();
    assert_eq!(recent[0].error_kind, None);
    assert_eq!(recent[0].error.as_deref(), Some("something new went wrong"));
}

#[test]
fn settings_upsert_and_read_back() {
    let (_dir, storage) = temp_storage();
    assert_eq!(storage.get_setting("theme").unwrap(), None);
    storage.set_setting("theme", "dark").unwrap();
    storage.set_setting("theme", "light").unwrap();
    assert_eq!(
        storage.get_setting("theme").unwrap().as_deref(),
        Some("light")
    );
}

// ------------------------------------------------------------ FTS + VACUUM

#[test]
fn search_hits_content_text_not_just_titles() {
    let (_dir, storage) = temp_storage();
    insert_one(&storage, "k1", "Bland title", "the quick brown fox");
    let hits = storage.search_articles("quick fox", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].title, "Bland title");
    assert_eq!(
        storage.search_articles("absent", 10).unwrap(),
        [] as [curio_core::model::Article; 0]
    );
}

#[test]
fn hostile_search_input_never_reaches_raw_match() {
    let (_dir, storage) = temp_storage();
    insert_one(&storage, "k1", "Title", "some body text");
    // Unbalanced quotes, FTS operators, column filters: all must be
    // treated as literal tokens, never as query syntax or errors.
    for query in ["\"unbalanced", "a AND b", "title:x", "NEAR(a b)", "-", "*"] {
        let result = storage.search_articles(query, 10);
        assert!(result.is_ok(), "query {query:?} errored: {result:?}");
    }
    assert_eq!(storage.search_articles("   ", 10).unwrap(), vec![]);
}

/// The sketch's corruption bug: FTS5 external-content keyed to the
/// implicit rowid of a TEXT-PK table was silently corrupted by VACUUM
/// (rowids renumber). With an explicit `INTEGER PRIMARY KEY` + sync
/// triggers, rowids — and the index mapping — survive vacuum, deletes,
/// and updates.
#[test]
fn fts_mapping_survives_vacuum_deletes_and_updates() {
    let (_dir, storage) = temp_storage();
    let a1 = insert_one(&storage, "k1", "First", "alpha words here");
    let a2 = insert_one(&storage, "k2", "Second", "beta words here");
    let a3 = insert_one(&storage, "k3", "Third", "gamma words here");

    // Create a rowid gap, then vacuum (would renumber implicit rowids).
    storage.delete_article(a2).unwrap();
    storage.vacuum().unwrap();
    storage.fts_integrity_check().unwrap();

    // Mapping still correct after vacuum.
    let hits = storage.search_articles("gamma", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, a3);
    assert_eq!(hits[0].title, "Third");
    assert_eq!(
        storage.search_articles("beta", 10).unwrap(),
        [] as [curio_core::model::Article; 0]
    );

    // Updates through the sync trigger keep the index fresh.
    storage
        .upsert_articles(vec![new_article("k3", "Third", "delta words now")])
        .unwrap();
    assert_eq!(
        storage.search_articles("gamma", 10).unwrap(),
        [] as [curio_core::model::Article; 0]
    );
    let hits = storage.search_articles("delta", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, a3);

    // Deletes clean the index via the delete trigger.
    storage.delete_article(a1).unwrap();
    assert_eq!(
        storage.search_articles("alpha", 10).unwrap(),
        [] as [curio_core::model::Article; 0]
    );

    storage.vacuum().unwrap();
    storage.fts_integrity_check().unwrap();
}

// ------------------------------------------------------------- concurrency

#[test]
fn storage_handle_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Storage>();
}

/// Guards the single-writer WAL invariant against regression: every mutation
/// must open its transaction through `begin_write` (which is `IMMEDIATE`), never
/// the default `conn.transaction()` (DEFERRED). A deferred transaction that
/// reads before it writes starts as a reader and, under concurrent readers in
/// WAL mode, can fail to upgrade with `SQLITE_BUSY_SNAPSHOT` — an error
/// `busy_timeout` does not retry, which is what made
/// `parallel_readers_stay_consistent_during_writes` flake in CI.
#[test]
fn writer_mutations_use_immediate_transactions() {
    // Every file of the `storage::repo` module (one per table family), read
    // from disk so a section added later is covered without editing this.
    let dir = common::manifest_dir().join("src/storage/repo");
    let mut scanned = Vec::new();
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).unwrap();
            assert!(
                !source.contains("conn.transaction()"),
                "{}: writer mutations must use begin_write() (IMMEDIATE), not the deferred \
                 conn.transaction()",
                path.display(),
            );
            scanned.push(path.file_name().unwrap().to_string_lossy().into_owned());
        }
    }
    scanned.sort();
    // Guard against a vacuous pass: the scan (which is not recursive) must see
    // every section file, so one moved or renamed out of the directory fails
    // here instead of silently dropping out of the check.
    for expected in [
        "articles.rs",
        "feeds.rs",
        "fetch_log.rs",
        "intents.rs",
        "mod.rs",
        "rows.rs",
        "settings.rs",
        "state.rs",
        "tags.rs",
    ] {
        assert!(
            scanned.iter().any(|name| name == expected),
            "repo module scan missed {expected}; saw {scanned:?}",
        );
    }
}

#[test]
fn parallel_readers_stay_consistent_during_writes() {
    let (_dir, storage) = temp_storage();
    let storage = Arc::new(storage);
    let stop = Arc::new(AtomicBool::new(false));

    let readers: Vec<_> = (0..4)
        .map(|_| {
            let storage = Arc::clone(&storage);
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || -> Result<u64, StorageError> {
                let mut iterations = 0;
                let mut last_count = 0;
                while !stop.load(Ordering::Relaxed) {
                    let count = storage.count_articles()?;
                    // Counts only grow while the writer inserts.
                    assert!(count >= last_count, "count went backwards");
                    last_count = count;
                    storage.search_articles("alpha", 5)?;
                    storage.list_articles(ListArticles::default())?;
                    iterations += 1;
                }
                Ok(iterations)
            })
        })
        .collect();

    for batch in 0..20 {
        let articles: Vec<NewArticle> = (0..25)
            .map(|i| {
                new_article(
                    &format!("b{batch}-a{i}"),
                    &format!("Title {batch}/{i}"),
                    "alpha beta gamma",
                )
            })
            .collect();
        storage.upsert_articles(articles).unwrap();
    }

    stop.store(true, Ordering::Relaxed);
    for reader in readers {
        let iterations = reader.join().unwrap().unwrap();
        assert!(iterations > 0, "reader thread never got a turn");
    }
    assert_eq!(storage.count_articles().unwrap(), 500);
    assert_eq!(storage.search_articles("alpha", 600).unwrap().len(), 500);
}

#[test]
fn parallel_writers_serialize_through_the_writer_thread() {
    let (_dir, storage) = temp_storage();
    let storage = Arc::new(storage);
    let writers: Vec<_> = (0..8)
        .map(|i| {
            let storage = Arc::clone(&storage);
            std::thread::spawn(move || {
                storage
                    .add_feed(NewFeed {
                        url: format!("https://example.com/{i}.xml"),
                        title: None,
                        tags: vec![],
                    })
                    .map(|_| ())
            })
        })
        .collect();
    for writer in writers {
        writer.join().unwrap().unwrap();
    }
    assert_eq!(storage.list_feeds().unwrap().len(), 8);
}

// --------------------------------------------------------- lead image

#[test]
fn lead_image_round_trips_through_insert_and_update() {
    let (_dir, storage) = temp_storage();
    let mut article = new_article("img:1", "Has an image", "body");
    article.lead_image = Some("https://cdn.example.com/lead.jpg".to_owned());
    storage.upsert_articles(vec![article.clone()]).unwrap();

    let stored = storage
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .find(|a| a.dedupe_key == "m:img:1")
        .unwrap();
    assert_eq!(
        stored.lead_image.as_deref(),
        Some("https://cdn.example.com/lead.jpg"),
        "the lead image survives insert and the list read path"
    );
    // The full read path (get_article) carries it too.
    let full = storage.get_article(stored.id).unwrap().unwrap();
    assert_eq!(full.lead_image, stored.lead_image);

    // Re-ingesting the same dedupe key updates the lead image in place.
    article.lead_image = Some("https://cdn.example.com/updated.jpg".to_owned());
    let outcome = storage.upsert_articles(vec![article]).unwrap();
    assert_eq!(outcome.updated, 1);
    let updated = storage.get_article(stored.id).unwrap().unwrap();
    assert_eq!(
        updated.lead_image.as_deref(),
        Some("https://cdn.example.com/updated.jpg")
    );
}

#[test]
fn absent_lead_image_reads_back_as_none() {
    let (_dir, storage) = temp_storage();
    let id = insert_one(&storage, "img:none", "No image", "body");
    let stored = storage.get_article(id).unwrap().unwrap();
    assert_eq!(stored.lead_image, None);
}

#[test]
fn a_future_published_date_is_clamped_to_first_seen_and_stays_put() {
    let (dir, storage) = temp_storage();
    let mut article = new_article("future:1", "From the future", "body");
    article.published_at = Some("2999-01-01T00:00:00.000Z".parse().unwrap());
    let before = Timestamp::now();
    storage.upsert_articles(vec![article.clone()]).unwrap();
    let id = storage
        .article_id_by_dedupe_key(None, "future:1")
        .unwrap()
        .unwrap();
    let stored = storage.get_article(id).unwrap().unwrap();
    assert_eq!(
        stored.published_at,
        Some(stored.saved_at),
        "a future date is clamped to the first-seen time"
    );
    assert!(stored.saved_at >= before);

    // Re-ingesting the same future date clamps to the STORED saved_at, not
    // to a fresh now — so the row is not rewritten on every refresh.
    let conn = side_conn(&dir);
    let stamp = modified_at(&conn, id);
    let outcome = storage.upsert_articles(vec![article.clone()]).unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (0, 0));
    assert_eq!(modified_at(&conn, id), stamp);

    // A past date passes through untouched.
    let past: Timestamp = "2020-05-01T08:00:00.000Z".parse().unwrap();
    article.published_at = Some(past);
    assert_eq!(storage.upsert_articles(vec![article]).unwrap().updated, 1);
    let stored = storage.get_article(id).unwrap().unwrap();
    assert_eq!(stored.published_at, Some(past));
}

#[test]
fn source_urls_by_dedupe_key_reads_only_stored_keys_in_scope() {
    let (_dir, storage) = temp_storage();
    let feed = add_feed(&storage, "https://example.com/feed.xml");
    let mut in_feed = new_article("guid:same", "In feed", "body");
    in_feed.feed_id = Some(feed.id);
    let manual = new_article("guid:other", "Manual", "body");
    storage.upsert_articles(vec![in_feed, manual]).unwrap();

    let keys = || {
        vec![
            "guid:same".to_owned(),
            "guid:other".to_owned(),
            "guid:missing".to_owned(),
        ]
    };
    let in_scope = storage
        .source_urls_by_dedupe_key(Some(feed.id), keys())
        .unwrap();
    assert_eq!(
        in_scope.len(),
        1,
        "the manual row is out of the feed's scope"
    );
    assert_eq!(
        in_scope.get("guid:same").map(String::as_str),
        Some("https://example.com/guid:same")
    );
    let manual_scope = storage.source_urls_by_dedupe_key(None, keys()).unwrap();
    assert_eq!(
        manual_scope.keys().collect::<Vec<_>>(),
        ["guid:other"],
        "keyed by the unscoped key"
    );
}

// ------------------------------- change-aware, provenance-aware upsert (#111)

/// A side connection onto the profile database, for asserting on columns
/// and shadow tables the repo API does not expose.
fn side_conn(dir: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(dir.path().join("curio.db")).unwrap()
}

fn modified_at(conn: &rusqlite::Connection, id: ArticleId) -> String {
    conn.query_row(
        "SELECT modified_at FROM articles WHERE id = ?1",
        [id.0],
        |row| row.get(0),
    )
    .unwrap()
}

fn content_origin(conn: &rusqlite::Connection, id: ArticleId) -> String {
    conn.query_row(
        "SELECT content_origin FROM articles WHERE id = ?1",
        [id.0],
        |row| row.get(0),
    )
    .unwrap()
}

/// Every FTS5 segment block, in order — any delete+reinsert of an index
/// row writes new blocks, so equal snapshots mean zero FTS writes.
fn fts_blocks(conn: &rusqlite::Connection) -> Vec<(i64, Vec<u8>)> {
    let mut stmt = conn
        .prepare("SELECT id, block FROM articles_fts_data ORDER BY id")
        .unwrap();
    stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

/// `PRAGMA data_version` moves whenever another connection commits a
/// change to the database — an unchanged value means nothing was written.
fn data_version(conn: &rusqlite::Connection) -> i64 {
    conn.query_row("PRAGMA data_version", [], |row| row.get(0))
        .unwrap()
}

fn feed_batch(feed_id: FeedId) -> Vec<NewArticle> {
    (0..3)
        .map(|i| {
            let mut article = new_article(&format!("guid:{i}"), &format!("Item {i}"), "excerpt");
            article.feed_id = Some(feed_id);
            article.author = Some("Ann".to_owned());
            article.published_at = Some("2026-07-01T00:00:00.000Z".parse().unwrap());
            article.word_count = Some(1);
            article
        })
        .collect()
}

fn one_feed(storage: &Storage) -> FeedId {
    storage
        .add_feed(NewFeed {
            url: "https://a.example/feed.xml".to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap()
        .0
        .id
}

#[test]
fn re_upserting_identical_entries_writes_nothing() {
    let (dir, storage) = temp_storage();
    let feed = one_feed(&storage);
    assert_eq!(
        storage.upsert_articles(feed_batch(feed)).unwrap().inserted,
        3
    );
    let conn = side_conn(&dir);
    let ids: Vec<ArticleId> = storage
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    let stamps: Vec<String> = ids.iter().map(|id| modified_at(&conn, *id)).collect();
    let blocks = fts_blocks(&conn);
    let version = data_version(&conn);

    let outcome = storage.upsert_articles(feed_batch(feed)).unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (0, 0));
    // Nothing was committed at all: no row, no FTS shadow-table write.
    assert_eq!(data_version(&conn), version, "the batch wrote nothing");
    assert_eq!(fts_blocks(&conn), blocks, "zero FTS writes");
    let after: Vec<String> = ids.iter().map(|id| modified_at(&conn, *id)).collect();
    assert_eq!(after, stamps, "modified_at unchanged");
    storage.fts_integrity_check().unwrap();

    // One genuinely changed entry is the only one written and counted.
    let mut batch = feed_batch(feed);
    batch[1].content.text = "excerpt, revised".to_owned();
    let outcome = storage.upsert_articles(batch).unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (0, 1));
    assert_ne!(data_version(&conn), version);
    storage.fts_integrity_check().unwrap();
}

#[test]
fn a_metadata_only_change_updates_the_row_without_reindexing_fts() {
    let (dir, storage) = temp_storage();
    let feed = one_feed(&storage);
    storage.upsert_articles(feed_batch(feed)).unwrap();
    let conn = side_conn(&dir);
    let blocks = fts_blocks(&conn);

    let mut batch = feed_batch(feed);
    batch[0].author = Some("Bea".to_owned());
    let outcome = storage.upsert_articles(batch).unwrap();
    assert_eq!(outcome.updated, 1);
    assert_eq!(fts_blocks(&conn), blocks, "author is not FTS-indexed");
    let stored = storage
        .list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .find(|a| a.dedupe_key.ends_with(":guid:0"))
        .unwrap();
    assert_eq!(stored.author.as_deref(), Some("Bea"));
    storage.fts_integrity_check().unwrap();
}

#[test]
fn hydrated_and_enriched_content_survive_a_feed_re_upsert() {
    let (dir, storage) = temp_storage();
    let feed = one_feed(&storage);
    storage.upsert_articles(feed_batch(feed)).unwrap();
    let rows = storage.list_articles(ListArticles::default()).unwrap();
    let id_of = |suffix: &str| {
        rows.iter()
            .find(|a| a.dedupe_key.ends_with(suffix))
            .unwrap()
            .id
    };
    let (full_id, enriched_id, plain_id) = (id_of(":guid:0"), id_of(":guid:1"), id_of(":guid:2"));
    let conn = side_conn(&dir);
    assert_eq!(content_origin(&conn, plain_id), "feed");

    let full = ArticleContent {
        html: "<p>the whole hydrated readability body</p>".to_owned(),
        text: "the whole hydrated readability body".to_owned(),
    };
    let enriched = ArticleContent {
        html: "<p>enriched selftext from the provider</p>".to_owned(),
        text: "enriched selftext from the provider".to_owned(),
    };
    storage
        .update_article_content(full_id, &full, Some(5), ContentOrigin::FullText)
        .unwrap();
    storage
        .update_article_content(enriched_id, &enriched, Some(5), ContentOrigin::Enriched)
        .unwrap();
    assert_eq!(content_origin(&conn, full_id), "full_text");
    assert_eq!(content_origin(&conn, enriched_id), "enriched");

    // The same feed XML again: nothing changes, nothing reverts.
    let outcome = storage.upsert_articles(feed_batch(feed)).unwrap();
    assert_eq!((outcome.inserted, outcome.updated), (0, 0));
    for (id, content) in [(full_id, &full), (enriched_id, &enriched)] {
        let stored = storage.get_article(id).unwrap().unwrap();
        assert_eq!(&stored.content, content);
        assert_eq!(stored.word_count, Some(5));
    }

    // A revised feed entry still updates title/metadata — never the body.
    let mut batch = feed_batch(feed);
    batch[0].title = "Item 0, retitled".to_owned();
    batch[0].content.text = "a different excerpt".to_owned();
    batch[2].content.text = "plain excerpt, revised".to_owned();
    let outcome = storage.upsert_articles(batch).unwrap();
    assert_eq!(outcome.updated, 2);
    let retitled = storage.get_article(full_id).unwrap().unwrap();
    assert_eq!(retitled.title, "Item 0, retitled");
    assert_eq!(retitled.content, full, "hydrated body kept");
    assert_eq!(content_origin(&conn, full_id), "full_text");
    assert_eq!(
        storage.get_article(plain_id).unwrap().unwrap().content.text,
        "plain excerpt, revised",
        "feed-origin content still refreshes"
    );
    // The FTS row follows the new title and still indexes the full body.
    let hits = storage.search_articles("retitled readability", 5).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, full_id);
    storage.fts_integrity_check().unwrap();
}

#[test]
fn manual_saves_are_recorded_with_manual_origin() {
    let (dir, storage) = temp_storage();
    let id = insert_one(&storage, "clip:1", "Clip", "clipped body");
    assert_eq!(content_origin(&side_conn(&dir), id), "manual");
}

/// Migration 0007 backfills provenance on existing rows and swaps in the
/// change-only FTS update trigger.
#[test]
fn migration_0007_backfills_content_origin_and_guards_the_fts_trigger() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        // A v6-shaped database: migrations 0001–0006 applied by hand.
        let conn = rusqlite::Connection::open(&path).unwrap();
        for sql in [
            include_str!("../src/storage/migrations/0001_init.sql"),
            include_str!("../src/storage/migrations/0002_feed_scoped_dedupe.sql"),
            include_str!("../src/storage/migrations/0003_feed_tags.sql"),
            include_str!("../src/storage/migrations/0004_article_lead_image.sql"),
            include_str!("../src/storage/migrations/0005_feed_sort_order.sql"),
            include_str!("../src/storage/migrations/0006_feed_full_text.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        // Feed 7 is a plain feed; feed 8 is in full-text mode, so its rows
        // may still hold a readability extract and are backfilled
        // `full_text` (a pre-upgrade refresh never re-hydrates them).
        conn.execute_batch(
            "INSERT INTO feeds (id, url, added_at, modified_at)
             VALUES (7, 'https://a.example/feed.xml', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z');
             INSERT INTO feeds (id, url, added_at, modified_at, fetch_full_text)
             VALUES (8, 'https://b.example/feed.xml', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z', 1);
             INSERT INTO articles (curio_id, feed_id, dedupe_key, title, source_url,
                                   saved_at, modified_at)
             VALUES ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d', 7, 'f7:guid:1', 'From feed',
                     'https://a.example/1', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z'),
                    ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6f', 8, 'f8:guid:3', 'Full text',
                     'https://b.example/3', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z'),
                    ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6e', NULL, 'm:guid:2', 'Manual',
                     'https://m.example/2', '2026-07-01T00:00:00.000Z',
                     '2026-07-01T00:00:00.000Z');",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 6).unwrap();
    }

    let storage = Storage::open(&path).unwrap();
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version()
    );
    let conn = rusqlite::Connection::open(&path).unwrap();
    let origins: Vec<(String, String)> = conn
        .prepare("SELECT dedupe_key, content_origin FROM articles ORDER BY dedupe_key")
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        origins,
        vec![
            ("f7:guid:1".to_owned(), "feed".to_owned()),
            ("f8:guid:3".to_owned(), "full_text".to_owned()),
            ("m:guid:2".to_owned(), "manual".to_owned()),
        ]
    );
    let trigger: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'trigger' AND name = 'articles_fts_au'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(
        trigger.contains("WHEN old.title IS NOT new.title"),
        "{trigger}"
    );
    // The CHECK constraint rejects an unknown provenance.
    assert!(
        conn.execute("UPDATE articles SET content_origin = 'bogus'", [])
            .is_err()
    );
    storage.fts_integrity_check().unwrap();
}

/// A profile #206's build already took to v7 still gets the dead-letter
/// table: migration 0008 runs on top of 0007, after a `pre-v8` backup.
#[test]
fn migration_0008_adds_the_dead_letter_table_to_a_v7_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        // A v7-shaped database: migrations 0001–0007 applied by hand.
        let conn = rusqlite::Connection::open(&path).unwrap();
        for sql in [
            include_str!("../src/storage/migrations/0001_init.sql"),
            include_str!("../src/storage/migrations/0002_feed_scoped_dedupe.sql"),
            include_str!("../src/storage/migrations/0003_feed_tags.sql"),
            include_str!("../src/storage/migrations/0004_article_lead_image.sql"),
            include_str!("../src/storage/migrations/0005_feed_sort_order.sql"),
            include_str!("../src/storage/migrations/0006_feed_full_text.sql"),
            include_str!("../src/storage/migrations/0007_article_content_origin.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 7).unwrap();
        let has_dead: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'event_intents_dead'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(has_dead, 0);
    }

    let storage = Storage::open(&path).unwrap();
    let latest = Storage::schema_version();
    assert!(latest >= 8);
    assert_eq!(storage.db_schema_version().unwrap(), latest);
    assert_eq!(storage.quarantined_intent_count().unwrap(), 0);
    assert!(
        dir.path()
            .join(format!("curio.db.pre-v{latest}.bak"))
            .is_file()
    );
}

/// A v8 profile gains `fetch_log.error_kind`: the fetch history it already
/// holds survives with no kind (it was never tracked), and new errored
/// attempts carry theirs — through to the feed's derived health.
#[test]
fn migration_0009_adds_the_fetch_error_kind_to_a_v8_database() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for sql in [
            include_str!("../src/storage/migrations/0001_init.sql"),
            include_str!("../src/storage/migrations/0002_feed_scoped_dedupe.sql"),
            include_str!("../src/storage/migrations/0003_feed_tags.sql"),
            include_str!("../src/storage/migrations/0004_article_lead_image.sql"),
            include_str!("../src/storage/migrations/0005_feed_sort_order.sql"),
            include_str!("../src/storage/migrations/0006_feed_full_text.sql"),
            include_str!("../src/storage/migrations/0007_article_content_origin.sql"),
            include_str!("../src/storage/migrations/0008_event_intents_dead.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 8).unwrap();
        conn.execute_batch(
            "INSERT INTO feeds (id, url, added_at, modified_at) \
               VALUES (1, 'https://example.com/feed.xml', '2026-01-01T00:00:00Z', \
                       '2026-01-01T00:00:00Z'); \
             INSERT INTO fetch_log (feed_id, fetched_at, status, error) \
               VALUES (1, '2026-01-01T00:00:00Z', 'error', 'old failure');",
        )
        .unwrap();
    }

    let storage = Storage::open(&path).unwrap();
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version()
    );
    let feed = storage.list_feeds().unwrap().remove(0);
    assert_eq!(feed.last_error.as_deref(), Some("old failure"));
    assert_eq!(feed.last_error_kind, None);
    let old = storage.recent_fetches(feed.id, 5).unwrap();
    assert_eq!((old.len(), old[0].error_kind), (1, None));

    storage
        .record_fetch(FetchRecord {
            feed_id: feed.id,
            fetched_at: Timestamp::now(),
            status: FetchStatus::Error,
            http_status: None,
            error: Some("connection refused".to_owned()),
            error_kind: Some(FetchErrorKind::Refused),
            articles_new: 0,
            duration_ms: None,
        })
        .unwrap();
    let feed = storage.get_feed(feed.id).unwrap().unwrap();
    assert_eq!(feed.last_error_kind, Some(FetchErrorKind::Refused));
    let newest = storage.recent_fetches(feed.id, 1).unwrap();
    assert_eq!(newest[0].error_kind, Some(FetchErrorKind::Refused));
}

/// Migration 0010 gives existing feeds an empty schedule (due on the next
/// sweep, no failures) and trims each feed's fetch log to its newest 50
/// rows — the only rows the log keeps from now on.
#[test]
fn migration_0010_schedules_existing_feeds_and_trims_their_fetch_log() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        for sql in [
            include_str!("../src/storage/migrations/0001_init.sql"),
            include_str!("../src/storage/migrations/0002_feed_scoped_dedupe.sql"),
            include_str!("../src/storage/migrations/0003_feed_tags.sql"),
            include_str!("../src/storage/migrations/0004_article_lead_image.sql"),
            include_str!("../src/storage/migrations/0005_feed_sort_order.sql"),
            include_str!("../src/storage/migrations/0006_feed_full_text.sql"),
            include_str!("../src/storage/migrations/0007_article_content_origin.sql"),
            include_str!("../src/storage/migrations/0008_event_intents_dead.sql"),
            include_str!("../src/storage/migrations/0009_fetch_error_kind.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 9).unwrap();
        for (id, url) in [(1, "https://a.example/feed"), (2, "https://b.example/feed")] {
            conn.execute(
                "INSERT INTO feeds (id, url, added_at, modified_at) \
                 VALUES (?1, ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                (id, url),
            )
            .unwrap();
        }
        let mut insert = conn
            .prepare(
                "INSERT INTO fetch_log (feed_id, fetched_at, status) \
                 VALUES (?1, '2026-01-01T00:00:00.000Z', ?2)",
            )
            .unwrap();
        for n in 0..120 {
            // Feed 1's newest row is an error, so the health column has
            // something to show after the trim.
            insert
                .execute((1, if n == 119 { "error" } else { "ok" }))
                .unwrap();
        }
        for _ in 0..3 {
            insert.execute((2, "ok")).unwrap();
        }
    }

    let storage = Storage::open(&path).unwrap();
    assert_eq!(storage.db_schema_version().unwrap(), 10);
    assert!(dir.path().join("curio.db.pre-v10.bak").is_file());
    let fetches = |id| storage.recent_fetches(FeedId(id), 1_000).unwrap();
    assert_eq!(fetches(1).len(), 50);
    assert_eq!(
        fetches(1)[0].status,
        FetchStatus::Error,
        "the newest rows stay"
    );
    assert_eq!(fetches(2).len(), 3, "a short log is untouched");
    for feed in storage.list_feeds().unwrap() {
        assert_eq!(feed.consecutive_failures, 0);
        assert_eq!(feed.next_attempt_at, None);
        assert_eq!(feed.last_error_kind, None);
        assert_eq!(feed.refresh_interval_min, None);
        assert_eq!(feed.failing_since, None);
        assert_eq!(feed.auto_paused_at, None);
        assert!(feed.is_due(Timestamp::now()));
    }
}
