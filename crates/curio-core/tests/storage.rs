//! Integration tests for the storage layer: migrations, repo round trips,
//! FTS trigger/rowid regression, and writer/reader concurrency.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use curio_core::model::{
    ArticleContent, ArticleId, FeedId, FeedStatus, FetchRecord, FetchStatus, NewArticle, NewFeed,
};
use curio_core::storage::{ListArticles, Storage, StorageError};
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
    assert!(matches!(
        err,
        StorageError::SchemaTooNew { found: 9999, .. }
    ));
}

#[test]
fn fresh_database_schema_matches_the_committed_golden() {
    let (_dir, storage) = temp_storage();
    let dump = storage.schema_dump().unwrap();
    let golden_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/schema_v1.txt");
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

    // Duplicate URL is a constraint violation.
    assert!(
        storage
            .add_feed(NewFeed {
                url: "https://example.com/feed.xml".to_owned(),
                title: None,
                tags: vec![],
            })
            .is_err()
    );

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
    assert!(storage.list_feeds().unwrap().is_empty());
    assert!(matches!(
        storage.remove_feed(feed.id),
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
    assert!(storage.pending_intents().unwrap().is_empty());

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

    // Reading moves the count; the other flags never do.
    let a1_id = id_by_key(&storage, &format!("f{}:a1", feed_a.id.0));
    let b1_id = id_by_key(&storage, &format!("f{}:b1", feed_b.id.0));
    storage.mark_read(a1_id, true).unwrap();
    storage.star_article(b1_id).unwrap();
    storage.archive_article(b1_id).unwrap();
    let counts = storage.unread_counts().unwrap();
    assert_eq!(counts.get(&Some(feed_a.id)), Some(&1));
    assert_eq!(
        counts.get(&Some(feed_b.id)),
        Some(&1),
        "starring/archiving must not affect unreadness"
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
    assert_eq!(counts.values().sum::<u64>(), 2);
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
    assert!(storage.pending_intents().unwrap().is_empty());
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
                articles_new: 0,
                duration_ms: Some(42),
            })
            .unwrap();
    }
    let recent = storage.recent_fetches(feed.id, 2).unwrap();
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].status, FetchStatus::Error);
    assert_eq!(recent[0].error.as_deref(), Some("timeout"));
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
                articles_new: 0,
                duration_ms: None,
            })
            .is_err()
    );
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
    assert!(storage.search_articles("absent", 10).unwrap().is_empty());
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
    assert!(storage.search_articles("beta", 10).unwrap().is_empty());

    // Updates through the sync trigger keep the index fresh.
    storage
        .upsert_articles(vec![new_article("k3", "Third", "delta words now")])
        .unwrap();
    assert!(storage.search_articles("gamma", 10).unwrap().is_empty());
    let hits = storage.search_articles("delta", 10).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id, a3);

    // Deletes clean the index via the delete trigger.
    storage.delete_article(a1).unwrap();
    assert!(storage.search_articles("alpha", 10).unwrap().is_empty());

    storage.vacuum().unwrap();
    storage.fts_integrity_check().unwrap();
}

// ------------------------------------------------------------- concurrency

#[test]
fn storage_handle_is_send_and_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Storage>();
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
