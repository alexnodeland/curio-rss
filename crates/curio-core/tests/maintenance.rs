//! Retention and routine maintenance (#137): the article-retention reap
//! and what it must never touch, tombstones, the orphan purge, unused
//! tags, the bounded fetch log, the search-index rebuild and the
//! maintenance pass's own housekeeping.

#![allow(clippy::unwrap_used)]

use std::collections::BTreeSet;
use std::num::NonZeroU32;
use std::path::Path;
use std::time::Duration;

use curio_core::fetch::FetchConfig;
use curio_core::model::{
    ArticleContent, ArticleId, FeedId, FetchRecord, FetchStatus, NewArticle, NewFeed,
    RetentionPolicy,
};
use curio_core::storage::{FETCH_LOG_KEEP, ListArticles, MaintenancePlan, Storage};
use curio_core::{CoreError, CoreHandle, CoreOptions, MaintenanceOptions};
use curio_types::{DestinationName, Timestamp};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn open_core(profile: &Path) -> CoreHandle {
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

fn add_feed(core: &CoreHandle, url: &str) -> FeedId {
    core.add_feed(NewFeed {
        url: url.to_owned(),
        title: None,
        tags: vec![],
    })
    .unwrap()
    .id
}

fn entry(feed: Option<FeedId>, key: &str) -> NewArticle {
    NewArticle {
        feed_id: feed,
        dedupe_key: key.to_owned(),
        title: format!("title {key}"),
        source_url: format!("https://example.com/{key}"),
        author: None,
        published_at: None,
        content: ArticleContent {
            html: format!("<p>body of {key}</p>"),
            text: format!("body of {key}"),
        },
        lang: None,
        word_count: None,
        source_updated_at: None,
        lead_image: None,
    }
}

/// Stores `keys` as one feed fetch would (in order: the last is newest)
/// and returns their ids in the same order.
fn ingest(storage: &Storage, feed: FeedId, keys: &[&str]) -> Vec<ArticleId> {
    storage
        .upsert_articles(keys.iter().map(|key| entry(Some(feed), key)).collect())
        .unwrap();
    keys.iter()
        .map(|key| {
            storage
                .article_id_by_dedupe_key(Some(feed), key)
                .unwrap()
                .unwrap()
        })
        .collect()
}

fn titles(core: &CoreHandle) -> BTreeSet<String> {
    core.list_articles(ListArticles {
        limit: 1_000,
        ..ListArticles::default()
    })
    .unwrap()
    .into_iter()
    .map(|article| article.title)
    .collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|key| format!("title {key}")).collect()
}

fn raw(storage: &Storage) -> rusqlite::Connection {
    let conn = rusqlite::Connection::open(storage.path()).unwrap();
    conn.busy_timeout(Duration::from_secs(5)).unwrap();
    conn
}

/// Moves every article's first-seen time a year back.
fn backdate_all(storage: &Storage) {
    raw(storage)
        .execute(
            "UPDATE articles SET saved_at = '2020-01-01T00:00:00.000Z'",
            [],
        )
        .unwrap();
}

fn tombstones(storage: &Storage) -> i64 {
    raw(storage)
        .query_row("SELECT COUNT(*) FROM article_tombstones", [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn days(n: u32) -> Option<NonZeroU32> {
    NonZeroU32::new(n)
}

// ------------------------------------------------------------- retention

/// Acceptance: retention never deletes a protected article — starred,
/// read-later, tagged, exported, or unread while kept — and the FTS
/// index still matches its content table afterwards.
#[test]
fn retention_never_reaps_protected_articles_and_fts_stays_in_sync() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let keys = [
        "starred", "later", "tagged", "exported", "unread", "read", "archived",
    ];
    let ids = ingest(core.storage(), feed, &keys);
    let [starred, later, tagged, exported, _unread, read, archived] = ids[..] else {
        unreachable!()
    };
    // Every protected article is read too, so only its protection keeps it.
    for id in [starred, later, tagged, exported, read] {
        core.mark_read(id, true).unwrap();
    }
    core.set_starred(starred, true).unwrap();
    core.set_read_later(later, true).unwrap();
    core.tag_article(tagged, "keep").unwrap();
    core.set_archived(archived, true).unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();
    core.save_to_destination(exported, &dest).unwrap();
    backdate_all(core.storage());

    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(30),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 2, "{report:?}");
    assert_eq!(report.reap_skipped, None);
    assert_eq!(
        titles(&core),
        set(&["starred", "later", "tagged", "exported", "unread"])
    );
    core.fts_integrity_check().unwrap();
    assert_eq!(core.search_articles("read", 10).unwrap().len(), 0);
    assert_eq!(core.search_articles("starred", 10).unwrap().len(), 1);

    // keep_unread off: the unread article goes too, the protected stay.
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(30),
        keep_unread: false,
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 1, "{report:?}");
    assert_eq!(
        titles(&core),
        set(&["starred", "later", "tagged", "exported"])
    );
    core.fts_integrity_check().unwrap();
    core.integrity_check().unwrap();
}

/// A recent article is younger than the age limit and stays.
#[test]
fn the_age_limit_spares_recent_articles() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["fresh"]);
    core.mark_read(ids[0], true).unwrap();
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 0);
    assert_eq!(titles(&core), set(&["fresh"]));
}

/// The per-feed limit keeps the newest N stored per feed; a protected
/// article counts towards N but is never the one reaped; another feed's
/// articles are counted separately.
#[test]
fn the_per_feed_limit_keeps_the_newest_and_every_protected_article() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let busy = add_feed(&core, "https://busy.example/feed.xml");
    let quiet = add_feed(&core, "https://quiet.example/feed.xml");
    let ids = ingest(core.storage(), busy, &["b1", "b2", "b3", "b4", "b5"]);
    let quiet_ids = ingest(core.storage(), quiet, &["q1", "q2"]);
    core.mark_read_many(&ids, true).unwrap();
    core.mark_read_many(&quiet_ids, true).unwrap();
    core.set_starred(ids[0], true).unwrap();

    core.set_retention_policy(RetentionPolicy {
        max_per_feed: days(2),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 2, "{report:?}");
    assert_eq!(titles(&core), set(&["b1", "b4", "b5", "q1", "q2"]));
    core.fts_integrity_check().unwrap();
}

/// The default policy reaps nothing, however old the articles.
#[test]
fn retention_is_off_until_a_limit_is_set() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a", "b"]);
    core.mark_read_many(&ids, true).unwrap();
    backdate_all(core.storage());
    assert_eq!(core.retention_policy().unwrap(), RetentionPolicy::default());
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 0);
    assert_eq!(titles(&core).len(), 2);
}

/// Feedless saves (clips, imports) are never reaped by policy.
#[test]
fn retention_never_reaps_feedless_saves() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    core.storage()
        .upsert_articles(vec![entry(None, "clip")])
        .unwrap();
    let id = core
        .storage()
        .article_id_by_dedupe_key(None, "clip")
        .unwrap()
        .unwrap();
    core.mark_read(id, true).unwrap();
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        max_per_feed: days(1),
        keep_unread: false,
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 0);
    assert_eq!(titles(&core), set(&["clip"]));
}

/// An exported destination that cannot be read (its `.curio/` marker is
/// gone — an unmounted vault) makes the pass reap nothing at all.
#[test]
fn an_unreadable_exported_set_reaps_nothing() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["exported", "read"]);
    core.mark_read_many(&ids, true).unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();
    core.save_to_destination(ids[0], &dest).unwrap();
    std::fs::remove_dir_all(vault.path().join(".curio")).unwrap();
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();

    let report = core
        .maintenance(MaintenanceOptions {
            purge_orphans: true,
        })
        .unwrap();
    assert_eq!(report.articles_reaped, 0);
    assert!(report.reap_skipped.is_some(), "{report:?}");
    assert_eq!(titles(&core).len(), 2);
}

/// A manifest that does not parse is no exported set either.
#[test]
fn a_corrupt_manifest_reaps_nothing() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a"]);
    core.mark_read_many(&ids, true).unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest, vault.path().to_path_buf())
        .unwrap();
    std::fs::create_dir_all(vault.path().join(".curio")).unwrap();
    std::fs::write(vault.path().join(".curio/manifest.json"), "{ not json").unwrap();
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 0);
    assert!(report.reap_skipped.is_some(), "{report:?}");
}

/// A conflict copy's entries are exports too: they protect.
#[test]
fn a_manifest_conflict_copy_protects_its_exports() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["synced", "read"]);
    core.mark_read_many(&ids, true).unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();
    core.save_to_destination(ids[0], &dest).unwrap();
    // Another device's manifest won; ours is parked as a conflict copy.
    let curio = vault.path().join(".curio");
    std::fs::rename(
        curio.join("manifest.json"),
        curio.join("manifest (conflicted copy).json"),
    )
    .unwrap();
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 1, "{report:?}");
    assert_eq!(titles(&core), set(&["synced"]));
}

/// The policy round-trips through settings, and its key is facade-owned.
#[test]
fn the_retention_policy_persists_and_its_key_is_reserved() {
    let profile = tempfile::tempdir().unwrap();
    let policy = RetentionPolicy {
        max_age_days: days(90),
        max_per_feed: days(500),
        keep_unread: false,
    };
    {
        let core = open_core(profile.path());
        core.set_retention_policy(policy).unwrap();
    }
    let core = open_core(profile.path());
    assert_eq!(core.retention_policy().unwrap(), policy);
    assert!(matches!(
        core.get_setting("retention.policy"),
        Err(CoreError::ReservedSettingKey { .. })
    ));
    assert!(matches!(
        core.set_setting("retention.policy", "{}"),
        Err(CoreError::ReservedSettingKey { .. })
    ));
}

// ------------------------------------------------------------ tombstones

/// A feed that keeps listing a reaped item does not bring it back; once a
/// later successful fetch no longer lists it, its tombstone is forgotten
/// (and an item that really returns later is stored again).
#[test]
fn a_reaped_item_stays_gone_while_its_feed_lists_it() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let storage = core.storage();
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(storage, feed, &["old", "new"]);
    core.mark_read_many(&ids, true).unwrap();
    let now = Timestamp::now();
    let policy = RetentionPolicy {
        max_per_feed: days(1),
        ..RetentionPolicy::default()
    };
    let plan = |retention, now| MaintenancePlan {
        retention,
        purge_orphans: false,
        exported: BTreeSet::new(),
        now,
    };
    let counts = storage.run_maintenance(plan(policy, now)).unwrap();
    assert_eq!(counts.articles_reaped, 1);
    assert_eq!(tombstones(storage), 1);

    // The feed still lists both: nothing is (re)inserted.
    let outcome = storage
        .upsert_articles(vec![entry(Some(feed), "old"), entry(Some(feed), "new")])
        .unwrap();
    assert_eq!(outcome.inserted, 0);
    assert_eq!(titles(&core), set(&["new"]));

    // A fetch a day later lists it no more → the tombstone is pruned
    // once that fetch is more than the slack past the last sighting.
    let soon = Timestamp::from_unix_millis(now.unix_millis() + 86_400_000).unwrap();
    let later = Timestamp::from_unix_millis(now.unix_millis() + 3 * 86_400_000).unwrap();
    let fetched = |at| FetchRecord {
        feed_id: feed,
        fetched_at: at,
        status: FetchStatus::Ok,
        http_status: Some(200),
        error: None,
        error_kind: None,
        articles_new: 0,
        duration_ms: None,
    };
    storage.record_fetch(fetched(soon)).unwrap();
    let counts = storage
        .run_maintenance(plan(RetentionPolicy::default(), now))
        .unwrap();
    assert_eq!(counts.tombstones_pruned, 0, "within the slack");
    storage.record_fetch(fetched(later)).unwrap();
    let counts = storage
        .run_maintenance(plan(RetentionPolicy::default(), now))
        .unwrap();
    assert_eq!(counts.tombstones_pruned, 1);
    let outcome = storage
        .upsert_articles(vec![entry(Some(feed), "old")])
        .unwrap();
    assert_eq!(outcome.inserted, 1, "a genuinely returning item is stored");
}

/// A sighting re-stamps the tombstone, so a feed that keeps listing the
/// item keeps it tombstoned however long that goes on.
#[test]
fn a_sighting_keeps_the_tombstone_alive() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let storage = core.storage();
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(storage, feed, &["old", "new"]);
    core.mark_read_many(&ids, true).unwrap();
    // Reaped (and stamped) ten days ago.
    let past =
        Timestamp::from_unix_millis(Timestamp::now().unix_millis() - 10 * 86_400_000).unwrap();
    let mut plan = MaintenancePlan {
        retention: RetentionPolicy {
            max_per_feed: days(1),
            ..RetentionPolicy::default()
        },
        purge_orphans: false,
        exported: BTreeSet::new(),
        now: past,
    };
    storage.run_maintenance(plan.clone()).unwrap();
    // Seen again now (re-stamped), then a successful fetch is logged.
    storage
        .upsert_articles(vec![entry(Some(feed), "old")])
        .unwrap();
    storage
        .record_fetch(FetchRecord {
            feed_id: feed,
            fetched_at: Timestamp::now(),
            status: FetchStatus::Ok,
            http_status: Some(200),
            error: None,
            error_kind: None,
            articles_new: 0,
            duration_ms: None,
        })
        .unwrap();
    plan.retention = RetentionPolicy::default();
    plan.now = Timestamp::now();
    let counts = storage.run_maintenance(plan).unwrap();
    assert_eq!(counts.tombstones_pruned, 0);
    assert_eq!(tombstones(storage), 1);
}

/// Removing a feed drops its tombstones with it.
#[test]
fn removing_a_feed_drops_its_tombstones() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a", "b"]);
    core.mark_read_many(&ids, true).unwrap();
    core.set_retention_policy(RetentionPolicy {
        max_per_feed: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(tombstones(core.storage()), 1);
    core.remove_feed(feed).unwrap();
    assert_eq!(tombstones(core.storage()), 0);
}

// --------------------------------------------------------------- orphans

/// A removed feed's articles are purged on request only — and never a
/// protected one or a feedless save.
#[test]
fn orphans_are_purged_on_request_keeping_protected_ones() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["unread", "read", "starred"]);
    core.mark_read(ids[1], true).unwrap();
    core.set_starred(ids[2], true).unwrap();
    core.storage()
        .upsert_articles(vec![entry(None, "clip")])
        .unwrap();
    core.remove_feed(feed).unwrap();
    assert_eq!(core.unread_counts().unwrap().get(&None), Some(&3));

    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.orphans_purged, 0, "not without asking");
    assert_eq!(titles(&core).len(), 4);

    let report = core
        .maintenance(MaintenanceOptions {
            purge_orphans: true,
        })
        .unwrap();
    assert_eq!(report.orphans_purged, 2, "{report:?}");
    assert_eq!(titles(&core), set(&["starred", "clip"]));
    assert_eq!(core.unread_counts().unwrap().get(&None), Some(&2));
    core.fts_integrity_check().unwrap();
}

// ------------------------------------------------------------------ tags

/// Acceptance: a tag disappears after its last untag.
#[test]
fn an_unused_tag_disappears_after_its_last_untag() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a", "b"]);
    core.tag_article(ids[0], "rust").unwrap();
    core.tag_article(ids[1], "rust").unwrap();
    core.tag_article(ids[1], "sqlite").unwrap();

    core.untag_article(ids[0], "rust").unwrap();
    assert_eq!(core.list_tags().unwrap(), ["rust", "sqlite"]);
    core.untag_article(ids[1], "rust").unwrap();
    assert_eq!(core.list_tags().unwrap(), ["sqlite"]);
    // Re-tagging brings it back as an ordinary tag.
    core.tag_article(ids[0], "rust").unwrap();
    assert_eq!(core.list_tags().unwrap(), ["rust", "sqlite"]);
}

/// Ghost tags an older build left (or a deleted article's) are dropped by
/// the maintenance pass.
#[test]
fn maintenance_drops_ghost_tags() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a", "b"]);
    core.tag_article(ids[0], "kept").unwrap();
    core.tag_article(ids[1], "orphaned").unwrap();
    core.delete_article(ids[1]).unwrap();
    raw(core.storage())
        .execute("INSERT INTO tags (name) VALUES ('ghost')", [])
        .unwrap();
    assert_eq!(core.list_tags().unwrap(), ["ghost", "kept", "orphaned"]);

    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.tags_dropped, 2);
    assert_eq!(core.list_tags().unwrap(), ["kept"]);
}

// ------------------------------------------------------------- fetch log

/// Acceptance: the fetch log stays bounded across 1000 refreshes, and
/// the maintenance pass trims a log grown past the cap some other way.
#[test]
fn the_fetch_log_stays_bounded_across_1000_refreshes() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let attempt = || FetchRecord {
        feed_id: feed,
        fetched_at: Timestamp::now(),
        status: FetchStatus::NotModified,
        http_status: Some(304),
        error: None,
        error_kind: None,
        articles_new: 0,
        duration_ms: Some(5),
    };
    for _ in 0..1_000 {
        core.storage().record_fetch(attempt()).unwrap();
    }
    let keep = usize::try_from(FETCH_LOG_KEEP).unwrap();
    assert_eq!(core.recent_fetches(feed, 10_000).unwrap().len(), keep);

    // Rows written around the per-insert trim (an older build) …
    let conn = raw(core.storage());
    for _ in 0..30 {
        conn.execute(
            "INSERT INTO fetch_log (feed_id, fetched_at, status) VALUES (?1, ?2, 'ok')",
            (feed.0, Timestamp::now().to_string()),
        )
        .unwrap();
    }
    // … are trimmed by the pass.
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.fetch_log_trimmed, 30);
    assert_eq!(core.recent_fetches(feed, 10_000).unwrap().len(), keep);
}

// ------------------------------------------------------------ FTS repair

/// Acceptance: a rebuild repairs a deliberately desynced search index.
#[test]
fn rebuilding_the_search_index_repairs_a_desynced_one() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    ingest(core.storage(), feed, &["real"]);
    // An index entry with no article behind it, and an article the index
    // never saw (the sync triggers bypassed).
    let conn = raw(core.storage());
    conn.execute(
        "INSERT INTO articles_fts (rowid, title, content_text) VALUES (9999, 'ghost', 'ghost')",
        [],
    )
    .unwrap();
    conn.execute_batch(
        "DROP TRIGGER articles_fts_ai;
         INSERT INTO articles (curio_id, feed_id, dedupe_key, title, source_url, saved_at,
                               modified_at)
         VALUES ('0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d', NULL, 'm:unindexed', 'unindexed',
                 'https://example.com/u', '2026-01-01T00:00:00.000Z',
                 '2026-01-01T00:00:00.000Z');
         CREATE TRIGGER articles_fts_ai AFTER INSERT ON articles BEGIN
             INSERT INTO articles_fts(rowid, title, content_text)
             VALUES (new.id, new.title, new.content_text);
         END;",
    )
    .unwrap();
    assert!(core.fts_integrity_check().is_err(), "the index is desynced");

    core.rebuild_search_index().unwrap();
    core.fts_integrity_check().unwrap();
    assert_eq!(core.search_articles("ghost", 10).unwrap().len(), 0);
    assert_eq!(core.search_articles("unindexed", 10).unwrap().len(), 1);
    assert_eq!(core.search_articles("real", 10).unwrap().len(), 1);
}

/// A reader that cached the index structure before a merge (the pass's
/// `optimize`, a rebuild) must not make the integrity check report the
/// healthy index as corrupt.
#[test]
fn the_integrity_check_survives_an_index_merge_by_another_connection() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a", "b", "c", "d"]);
    core.mark_read_many(&ids, true).unwrap();
    // Warm every pooled reader's cached index structure.
    for _ in 0..8 {
        core.integrity_check().unwrap();
        core.search_articles("body", 10).unwrap();
    }
    core.set_retention_policy(RetentionPolicy {
        max_per_feed: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 3);
    core.integrity_check().unwrap();
    core.rebuild_search_index().unwrap();
    core.integrity_check().unwrap();
    core.fts_integrity_check().unwrap();
}

// ------------------------------------------------------------ the pass

/// The pass also sweeps expired event-log files.
#[test]
fn maintenance_sweeps_expired_event_logs() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    add_feed(&core, "https://example.com/feed.xml");
    let expired = profile.path().join(".curio/events/events-20200101.jsonl");
    std::fs::write(&expired, "").unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.event_files_removed, vec![expired.clone()]);
    assert_eq!(report.event_sweep_skipped, None);
    assert!(!expired.exists());
}

/// The pass ends with the WAL checkpointed and truncated.
#[test]
fn maintenance_truncates_the_wal() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let keys: Vec<String> = (0..200).map(|n| format!("k{n}")).collect();
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    ingest(core.storage(), feed, &keys);
    let mut wal = core.db_path().as_os_str().to_owned();
    wal.push("-wal");
    assert!(std::fs::metadata(&wal).unwrap().len() > 0);
    core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(std::fs::metadata(&wal).unwrap().len(), 0);
}

/// The pass reports nothing skipped when its upkeep ran.
#[test]
fn a_pass_with_deletions_runs_its_upkeep() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["a", "b", "c"]);
    core.mark_read_many(&ids, true).unwrap();
    core.set_retention_policy(RetentionPolicy {
        max_per_feed: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 2);
    assert_eq!(report.upkeep_skipped, None);
    core.fts_integrity_check().unwrap();
    core.integrity_check().unwrap();
}

// ------------------------------------------------------- export records

fn exports(storage: &Storage) -> i64 {
    raw(storage)
        .query_row("SELECT COUNT(*) FROM article_exports", [], |row| row.get(0))
        .unwrap()
}

/// An article recorded as exported is never reaped, though the pass's
/// manifest view does not list it: an export that lands between the
/// pass reading the manifests and reaping is honoured inside the reap's
/// own transaction.
#[test]
fn an_export_recorded_mid_pass_is_never_reaped() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let storage = core.storage();
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(storage, feed, &["exporting", "read"]);
    core.mark_read_many(&ids, true).unwrap();
    assert!(
        storage
            .mark_article_exported(ids[0], Timestamp::now())
            .unwrap()
    );
    backdate_all(storage);
    let counts = storage
        .run_maintenance(MaintenancePlan {
            retention: RetentionPolicy {
                max_age_days: days(1),
                ..RetentionPolicy::default()
            },
            purge_orphans: false,
            exported: BTreeSet::new(),
            now: Timestamp::now(),
        })
        .unwrap();
    assert_eq!(counts.articles_reaped, 1, "{counts:?}");
    assert_eq!(titles(&core), set(&["exporting"]));
}

/// Recording is idempotent, and an article already deleted (reaped while
/// its export was starting) is reported gone, so the export writes no
/// note for it.
#[test]
fn recording_an_export_of_a_deleted_article_reports_it_gone() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let storage = core.storage();
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(storage, feed, &["kept", "gone"]);
    let now = Timestamp::now();
    assert!(storage.mark_article_exported(ids[0], now).unwrap());
    assert!(storage.mark_article_exported(ids[0], now).unwrap());
    assert_eq!(exports(storage), 1);
    storage.delete_article(ids[1]).unwrap();
    assert!(!storage.mark_article_exported(ids[1], now).unwrap());
    assert_eq!(exports(storage), 1);
    // Deleting an exported article drops its record with it.
    storage.delete_article(ids[0]).unwrap();
    assert_eq!(exports(storage), 0);
}

/// A bulk export records its whole batch at once: already-recorded
/// articles stay recorded once, and only the deleted ones are reported.
#[test]
fn recording_a_batch_of_exports_reports_only_the_deleted_articles() {
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let storage = core.storage();
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(storage, feed, &["recorded", "fresh", "gone"]);
    let now = Timestamp::now();
    assert!(storage.mark_article_exported(ids[0], now).unwrap());
    storage.delete_article(ids[2]).unwrap();

    let gone = storage.mark_articles_exported(&ids, now).unwrap();
    assert_eq!(gone, BTreeSet::from([ids[2]]));
    assert_eq!(exports(storage), 2);
    // All recorded: nothing to write, nothing gone.
    assert!(
        storage
            .mark_articles_exported(&ids[..2], now)
            .unwrap()
            .is_empty()
    );
    assert_eq!(exports(storage), 2);
}

/// Export protection outlives the destination: removing it, or
/// re-rooting it at a fresh folder, does not expose what was exported.
#[test]
fn an_export_stays_protected_after_its_destination_is_removed_or_moved() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let moved_to = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["removed", "moved", "read"]);
    core.mark_read_many(&ids, true).unwrap();
    let removed: DestinationName = "vault".parse().unwrap();
    let moved: DestinationName = "other".parse().unwrap();
    core.add_destination(removed.clone(), vault.path().to_path_buf())
        .unwrap();
    core.add_destination(moved.clone(), other.path().to_path_buf())
        .unwrap();
    core.save_to_destination(ids[0], &removed).unwrap();
    core.save_to_destination(ids[1], &moved).unwrap();
    core.remove_destination(&removed).unwrap();
    core.add_destination(moved, moved_to.path().to_path_buf())
        .unwrap();
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();

    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.reap_skipped, None);
    assert_eq!(report.articles_reaped, 1, "{report:?}");
    assert_eq!(titles(&core), set(&["removed", "moved"]));
}

/// Exports made before Curio recorded them (only the manifest knows) are
/// recorded by the first pass that reads the manifest, so they stay
/// protected once the destination is gone.
#[test]
fn a_pass_records_the_exports_only_a_manifest_knows() {
    let profile = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = add_feed(&core, "https://example.com/feed.xml");
    let ids = ingest(core.storage(), feed, &["old export", "read"]);
    core.mark_read_many(&ids, true).unwrap();
    let dest: DestinationName = "vault".parse().unwrap();
    core.add_destination(dest.clone(), vault.path().to_path_buf())
        .unwrap();
    core.save_to_destination(ids[0], &dest).unwrap();
    // As a profile upgraded from before export records existed.
    raw(core.storage())
        .execute("DELETE FROM article_exports", [])
        .unwrap();
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();

    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 1, "{report:?}");
    assert_eq!(exports(core.storage()), 1);
    core.remove_destination(&dest).unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 0, "{report:?}");
    assert_eq!(titles(&core), set(&["old export"]));
}

// ------------------------------------------- tombstones and reused guids

/// Serves `posts` at `/feed.xml`, every item under the one guid `same`
/// (so two or more posts make the guid shared, one makes it unique).
async fn serve_same_guid(server: &MockServer, posts: &[&str]) {
    server.reset().await;
    let items = posts
        .iter()
        .map(|post| {
            format!(
                "<item><guid>same</guid><title>title {post}</title><link>{}/{post}</link>\
                 <description>stub</description></item>",
                server.uri()
            )
        })
        .collect::<Vec<_>>()
        .concat();
    let xml = format!(
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Reused</title>{items}</channel></rss>"#
    );
    Mock::given(method("GET"))
        .and(path("/feed.xml"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(xml, "application/rss+xml"))
        .mount(server)
        .await;
}

/// Subscribes to the mock server's feed (loopback allowed for it).
fn subscribe_mock(core: &CoreHandle, server: &MockServer) -> FeedId {
    let feed = add_feed(core, &format!("{}/feed.xml", server.uri()));
    core.set_feed_allow_private_network(feed, true).unwrap();
    feed
}

/// Reaps the one read article (unread ones are kept).
fn reap_read(core: &CoreHandle) {
    backdate_all(core.storage());
    core.set_retention_policy(RetentionPolicy {
        max_age_days: days(1),
        ..RetentionPolicy::default()
    })
    .unwrap();
    let report = core.maintenance(MaintenanceOptions::default()).unwrap();
    assert_eq!(report.articles_reaped, 1, "{report:?}");
}

fn title_id(core: &CoreHandle, title: &str) -> ArticleId {
    core.list_articles(ListArticles::default())
        .unwrap()
        .into_iter()
        .find(|article| article.title == title)
        .unwrap()
        .id
}

/// An item reaped under its `guid+link:` key (its guid was shared) stays
/// gone when a later fetch lists it alone, keyed `guid:`.
#[tokio::test]
async fn a_reaped_shared_guid_item_stays_gone_once_its_guid_is_unshared() {
    let server = MockServer::start().await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe_mock(&core, &server);

    serve_same_guid(&server, &["a", "b", "c"]).await;
    assert_eq!(core.refresh_feed(feed).await.unwrap().new_articles, 3);
    core.mark_read(title_id(&core, "title c"), true).unwrap();
    reap_read(&core);
    assert_eq!(titles(&core), set(&["a", "b"]));

    serve_same_guid(&server, &["c"]).await;
    let alone = core.refresh_feed(feed).await.unwrap();
    assert_eq!(alone.new_articles, 0, "{alone:?}");
    assert_eq!(titles(&core), set(&["a", "b"]));
}

/// An item reaped under its plain `guid:` key (its guid was unique) stays
/// gone when a later fetch shares the guid and keys it `guid+link:`; the
/// other items sharing the guid are new and stored.
#[tokio::test]
async fn a_reaped_unique_guid_item_stays_gone_once_its_guid_is_shared() {
    let server = MockServer::start().await;
    let profile = tempfile::tempdir().unwrap();
    let core = open_core(profile.path());
    let feed = subscribe_mock(&core, &server);

    serve_same_guid(&server, &["c"]).await;
    assert_eq!(core.refresh_feed(feed).await.unwrap().new_articles, 1);
    core.mark_read(title_id(&core, "title c"), true).unwrap();
    reap_read(&core);
    assert!(titles(&core).is_empty());

    serve_same_guid(&server, &["a", "b", "c"]).await;
    let shared = core.refresh_feed(feed).await.unwrap();
    assert_eq!(shared.new_articles, 2, "{shared:?}");
    assert_eq!(titles(&core), set(&["a", "b"]));
}
