//! List rows and the read paths under them (#136): `list_article_rows` /
//! `search_article_rows` carry state, tags and a bounded preview in one
//! read and agree with the full-article listing under every filter; the
//! trigger-kept unread counters stay equal to the scan they replaced
//! across any sequence of writes; and migration 0012 backfills them.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use curio_core::model::{
    ArticleContent, ArticleFlag, ArticleId, ArticleState, FeedId, ListedArticle, NewArticle,
    NewFeed, ReadProgress, SNIPPET_MAX_CHARS,
};
use curio_core::storage::{ListArticles, ListCursor, ListOrder, SearchArticles, Storage};
use curio_types::Timestamp;
use proptest::prelude::*;

fn temp_storage() -> (tempfile::TempDir, Storage) {
    let dir = tempfile::tempdir().unwrap();
    let storage = Storage::open(dir.path().join("curio.db")).unwrap();
    (dir, storage)
}

fn side_conn(dir: &tempfile::TempDir) -> rusqlite::Connection {
    rusqlite::Connection::open(dir.path().join("curio.db")).unwrap()
}

fn article(feed_id: Option<FeedId>, key: &str, text: &str) -> NewArticle {
    NewArticle {
        feed_id,
        dedupe_key: key.to_owned(),
        title: format!("Title {key}"),
        source_url: format!("https://example.com/{key}"),
        author: Some("A. Writer".to_owned()),
        published_at: None,
        content: ArticleContent {
            html: format!("<p>{text}</p>"),
            text: text.to_owned(),
        },
        lang: Some("en".to_owned()),
        word_count: Some(3),
        source_updated_at: None,
        lead_image: Some("https://example.com/lead.png".to_owned()),
    }
}

fn add_feed(storage: &Storage, url: &str) -> FeedId {
    storage
        .add_feed(NewFeed {
            url: url.to_owned(),
            title: None,
            tags: vec![],
        })
        .unwrap()
        .0
        .id
}

/// Every stored article id, oldest first.
fn all_ids(storage: &Storage) -> Vec<ArticleId> {
    let mut ids: Vec<ArticleId> = storage
        .list_articles(ListArticles {
            limit: u32::MAX,
            ..ListArticles::default()
        })
        .unwrap()
        .into_iter()
        .map(|a| a.id)
        .collect();
    ids.reverse();
    ids
}

/// The unread counts by the scan the counters replaced — the reference.
fn scanned_unread(conn: &rusqlite::Connection) -> BTreeMap<Option<FeedId>, u64> {
    let mut stmt = conn
        .prepare(
            "SELECT a.feed_id, COUNT(*) FROM articles a \
             LEFT JOIN article_state s ON s.article_id = a.id \
             WHERE COALESCE(s.is_read, 0) = 0 AND COALESCE(s.is_archived, 0) = 0 \
             GROUP BY a.feed_id",
        )
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, Option<i64>>(0)?.map(FeedId),
            u64::try_from(row.get::<_, i64>(1)?).unwrap(),
        ))
    })
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

// ------------------------------------------------------------- list rows

#[test]
fn a_list_row_carries_state_tags_and_a_preview_but_no_body() {
    let (_dir, storage) = temp_storage();
    let feed = add_feed(&storage, "https://feed.example/rss");
    storage
        .upsert_articles(vec![
            article(Some(feed), "a", "  first   body\n\ntext "),
            article(None, "b", ""),
        ])
        .unwrap();
    let ids = all_ids(&storage);
    let (a, b) = (ids[0], ids[1]);
    storage.mark_read(a, true).unwrap();
    storage.star_article(a).unwrap();
    storage.tag_article(a, "zeta").unwrap();
    storage.tag_article(a, "alpha").unwrap();
    let halfway = ReadProgress::from_permille(420).unwrap();
    storage.set_read_progress(a, halfway).unwrap();

    let rows = storage.list_article_rows(ListArticles::default()).unwrap();
    assert_eq!(rows.len(), 2);
    let (row_b, row_a) = (&rows[0], &rows[1]);
    assert_eq!(row_b.id, b, "newest first");
    assert_eq!(
        row_b.state,
        ArticleState::default(),
        "untouched → all false"
    );
    assert_eq!(row_b.tags, Vec::<String>::new());
    assert_eq!(row_b.snippet, None, "no text → no preview");
    assert_eq!(row_b.feed_id, None);

    let full = storage.get_article(a).unwrap().unwrap();
    assert_eq!(row_a.curio_id, full.curio_id);
    assert_eq!(row_a.feed_id, Some(feed));
    assert_eq!(row_a.title, full.title);
    assert_eq!(row_a.source_url, full.source_url);
    assert_eq!(row_a.author, full.author);
    assert_eq!(row_a.published_at, full.published_at);
    assert_eq!(row_a.lang, full.lang);
    assert_eq!(row_a.word_count, full.word_count);
    assert_eq!(row_a.saved_at, full.saved_at);
    assert_eq!(row_a.lead_image, full.lead_image);
    assert_eq!(row_a.snippet.as_deref(), Some("first body text"));
    assert_eq!(
        row_a.state,
        ArticleState {
            read: true,
            starred: true,
            read_later: false,
            archived: false,
            progress: halfway,
        },
        "the reading position rides along with the flags"
    );
    assert_eq!(row_a.state, storage.article_state(a).unwrap());
    assert_eq!(row_a.tags, ["alpha", "zeta"], "sorted, as article_tags");
    assert_eq!(row_a.tags, storage.article_tags(a).unwrap());
}

#[test]
fn a_long_body_is_previewed_from_its_start_only() {
    let (_dir, storage) = temp_storage();
    // Past the snippet window by a long way, with a marker at the very end
    // that a whole-body read would be the only way to see.
    let text = format!("{} END-MARKER", "lorem ipsum ".repeat(10_000));
    storage
        .upsert_articles(vec![article(None, "long", &text)])
        .unwrap();
    let row = storage
        .list_article_rows(ListArticles::default())
        .unwrap()
        .pop()
        .unwrap();
    let snippet = row.snippet.unwrap();
    assert_eq!(snippet.chars().count(), SNIPPET_MAX_CHARS + 1);
    assert!(snippet.starts_with("lorem ipsum lorem"));
    assert!(snippet.ends_with('…'));

    // Mostly whitespace up front: the preview is short, but the text runs
    // on past the window, so it still says so.
    let sparse = format!("tip{}tail", " ".repeat(5_000));
    storage
        .upsert_articles(vec![article(None, "sparse", &sparse)])
        .unwrap();
    let row = storage
        .list_article_rows(ListArticles {
            limit: 1,
            ..ListArticles::default()
        })
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(row.snippet.as_deref(), Some("tip…"));

    // Multi-byte text is cut on characters, never inside one.
    let wide = "é".repeat(SNIPPET_MAX_CHARS * 10);
    storage
        .upsert_articles(vec![article(None, "wide", &wide)])
        .unwrap();
    let row = storage
        .list_article_rows(ListArticles {
            limit: 1,
            ..ListArticles::default()
        })
        .unwrap()
        .pop()
        .unwrap();
    assert_eq!(
        row.snippet.unwrap(),
        format!("{}…", "é".repeat(SNIPPET_MAX_CHARS))
    );
}

/// The state every article in `ids` gets, cycled so every combination of
/// the four flags occurs.
fn state_for(index: usize) -> ArticleState {
    ArticleState {
        read: index & 1 != 0,
        starred: index & 2 != 0,
        read_later: index & 4 != 0,
        archived: index & 8 != 0,
        progress: ReadProgress::START,
    }
}

fn apply_state(storage: &Storage, id: ArticleId, state: ArticleState) {
    storage.mark_read(id, state.read).unwrap();
    if state.starred {
        storage.star_article(id).unwrap();
    }
    if state.read_later {
        storage.add_read_later(id).unwrap();
    }
    if state.archived {
        storage.archive_article(id).unwrap();
    }
}

fn keeps(want: Option<bool>, have: bool) -> bool {
    want.is_none_or(|want| want == have)
}

/// Pages a listing through its `after` cursor in pages of `limit`, as
/// heads do — the cursor built from each page's last row.
fn paged<T>(
    params: &ListArticles,
    limit: u32,
    mut page: impl FnMut(ListArticles) -> Vec<T>,
    id: impl Fn(&T) -> ArticleId,
) -> Vec<ArticleId>
where
    for<'a> ListCursor: From<&'a T>,
{
    let mut out = Vec::new();
    let mut after = None;
    loop {
        let rows = page(ListArticles {
            after,
            limit,
            ..params.clone()
        });
        out.extend(rows.iter().map(&id));
        after = rows.last().map(ListCursor::from);
        if rows.len() < limit as usize {
            return out;
        }
    }
}

/// `ids` (each with its publication-order date) sorted into `order`.
fn in_order(mut ids: Vec<(ArticleId, Timestamp)>, order: ListOrder) -> Vec<ArticleId> {
    match order {
        ListOrder::Newest => ids.sort_by_key(|a| std::cmp::Reverse(a.0)),
        ListOrder::Oldest => ids.sort_by_key(|a| a.0),
        ListOrder::Published => ids.sort_by_key(|a| std::cmp::Reverse((a.1, a.0))),
    }
    ids.into_iter().map(|(id, _)| id).collect()
}

/// Every combination of the four state filters (each unset / true /
/// false), across feeds and within one, in every order, paged through the
/// keyset: the list rows, the full listing and a model of the filter and
/// order semantics agree — including on the membership views that page
/// through a partial index.
#[test]
fn list_rows_match_the_filter_semantics_under_every_state_filter() {
    let (_dir, storage) = temp_storage();
    let feeds = [
        add_feed(&storage, "https://one.example/rss"),
        add_feed(&storage, "https://two.example/rss"),
    ];
    let batch: Vec<NewArticle> = (0..48)
        .map(|i| {
            let feed = if i % 3 == 0 { None } else { Some(feeds[i % 2]) };
            let mut article = article(feed, &format!("k{i}"), "body");
            // Publication dates out of insertion order, with ties (and
            // every fifth undated, standing at its save time).
            if i % 5 != 0 {
                let day = (i * 7) % 11 + 1;
                article.published_at = Some(format!("2024-03-{day:02}T08:00:00Z").parse().unwrap());
            }
            article
        })
        .collect();
    storage.upsert_articles(batch).unwrap();
    let ids = all_ids(&storage);
    let mut model = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        // The first few stay without any state row at all.
        let state = if index < 4 {
            ArticleState::default()
        } else {
            state_for(index)
        };
        if index >= 4 {
            apply_state(&storage, *id, state);
        }
        let stored = storage.get_article(*id).unwrap().unwrap();
        let dated = stored.published_at.unwrap_or(stored.saved_at);
        model.push((*id, stored.feed_id, state, dated));
    }

    let choices = [None, Some(true), Some(false)];
    let orders = [ListOrder::Newest, ListOrder::Oldest, ListOrder::Published];
    for order in orders {
        for read in choices {
            for starred in choices {
                for read_later in choices {
                    for archived in choices {
                        for feed_id in [None, Some(feeds[0])] {
                            let params = ListArticles {
                                feed_id,
                                order,
                                read,
                                starred,
                                read_later,
                                archived,
                                ..ListArticles::default()
                            };
                            let kept = model
                                .iter()
                                .filter(|(_, feed, state, _)| {
                                    feed_id.is_none_or(|want| *feed == Some(want))
                                        && keeps(read, state.read)
                                        && keeps(starred, state.starred)
                                        && keeps(read_later, state.read_later)
                                        && keeps(archived, state.archived)
                                })
                                .map(|(id, _, _, dated)| (*id, *dated))
                                .collect();
                            let expected = in_order(kept, order);
                            let rows = paged(
                                &params,
                                3,
                                |p| storage.list_article_rows(p).unwrap(),
                                |row| row.id,
                            );
                            let full =
                                paged(&params, 3, |p| storage.list_articles(p).unwrap(), |a| a.id);
                            assert_eq!(rows, expected, "rows under {params:?}");
                            assert_eq!(full, expected, "full under {params:?}");
                            assert_eq!(
                                storage.count_listed_articles(params.clone()).unwrap(),
                                expected.len() as u64,
                                "count under {params:?}"
                            );
                        }
                    }
                }
            }
        }
    }

    // And each row's flags are the article's own.
    for row in storage
        .list_article_rows(ListArticles {
            limit: 1_000,
            ..ListArticles::default()
        })
        .unwrap()
    {
        assert_eq!(row.state, storage.article_state(row.id).unwrap());
    }
}

#[test]
fn tag_and_folder_filters_compose_with_a_membership_view() {
    let (_dir, storage) = temp_storage();
    let (tech, _) = storage
        .add_feed(NewFeed {
            url: "https://tech.example/rss".to_owned(),
            title: None,
            tags: vec!["Tech/Rust".to_owned()],
        })
        .unwrap();
    let other = add_feed(&storage, "https://other.example/rss");
    storage
        .upsert_articles(vec![
            article(Some(tech.id), "t1", "x"),
            article(Some(tech.id), "t2", "x"),
            article(Some(other), "o1", "x"),
        ])
        .unwrap();
    let ids = all_ids(&storage);
    for id in &ids {
        storage.star_article(*id).unwrap();
    }
    storage.tag_article(ids[1], "keep").unwrap();
    storage.tag_article(ids[2], "keep").unwrap();

    let starred_in_folder = storage
        .list_article_rows(ListArticles {
            starred: Some(true),
            feed_tag: Some("Tech".to_owned()),
            ..ListArticles::default()
        })
        .unwrap();
    let got: Vec<ArticleId> = starred_in_folder.iter().map(|r| r.id).collect();
    assert_eq!(got, [ids[1], ids[0]]);

    let starred_tagged = storage
        .list_article_rows(ListArticles {
            starred: Some(true),
            tag: Some(" keep ".to_owned()),
            ..ListArticles::default()
        })
        .unwrap();
    let got: Vec<ArticleId> = starred_tagged.iter().map(|r| r.id).collect();
    assert_eq!(got, [ids[2], ids[1]]);
    assert!(starred_tagged.iter().all(|r| r.tags == ["keep"]));
}

/// Stores one article published at `published` (`None` = undated) and
/// returns its id.
fn insert_dated(storage: &Storage, key: &str, published: Option<&str>) -> ArticleId {
    let mut new = article(None, key, "body");
    new.published_at = published.map(|at| at.parse().unwrap());
    storage.upsert_articles(vec![new]).unwrap();
    *all_ids(storage).last().unwrap()
}

fn published_page(storage: &Storage, after: Option<ListCursor>) -> Vec<ListedArticle> {
    storage
        .list_article_rows(ListArticles {
            order: ListOrder::Published,
            after,
            limit: 2,
            ..ListArticles::default()
        })
        .unwrap()
}

/// The published order puts a backfill at its posts' own dates, not on
/// top, and pages stably: rows written, deleted or re-dated between pages
/// never make a page repeat or skip a row that kept its place.
#[test]
fn published_order_pages_stably_while_the_library_changes() {
    let (_dir, storage) = temp_storage();
    let today = insert_dated(&storage, "today", Some("2025-06-02T09:00:00Z"));
    let undated = insert_dated(&storage, "undated", None);
    let tie_a = insert_dated(&storage, "tie-a", Some("2025-05-01T12:00:00Z"));
    let tie_b = insert_dated(&storage, "tie-b", Some("2025-05-01T12:00:00Z"));
    let mid = insert_dated(&storage, "mid", Some("2025-04-15T00:00:00Z"));
    // A backfill — an OPML import's first refresh — arrives last.
    let old = insert_dated(&storage, "old", Some("2019-05-05T05:05:05Z"));
    let older = insert_dated(&storage, "older", Some("2018-01-01T00:00:00Z"));

    let newest: Vec<ArticleId> = storage
        .list_article_rows(ListArticles::default())
        .unwrap()
        .iter()
        .map(|row| row.id)
        .collect();
    assert_eq!(newest[..2], [older, old], "insertion order puts it on top");

    // Undated stands at its save time (now, the latest); a date tie falls
    // back to the id, newest first; the backfill sinks to its own dates.
    let expected = [undated, today, tie_b, tie_a, mid, old, older];
    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let page = published_page(&storage, after);
        walked.extend(page.iter().map(|row| row.id));
        after = page.last().map(ListCursor::from);
        if page.len() < 2 {
            break;
        }
    }
    assert_eq!(walked, expected);

    // A bare-id cursor resumes at the same place while its row exists.
    let first = published_page(&storage, None);
    let carried = published_page(&storage, Some(ListCursor::from(&first[1])));
    let by_id = published_page(&storage, Some(ListCursor::id(first[1].id)));
    assert_eq!(carried, by_id);

    // Now walk again, changing the library between pages.
    let page1 = published_page(&storage, None);
    assert_eq!(ids_of(&page1), [undated, today]);
    let cursor = ListCursor::from(&page1[1]);
    // A row newer than the cursor sits above the walk; a row older than
    // it lands ahead of the walk, once; the cursor's own row goes away.
    let fresh = insert_dated(&storage, "fresh", Some("2025-06-02T10:00:00Z"));
    let late_backfill = insert_dated(&storage, "late", Some("2025-04-20T00:00:00Z"));
    storage.delete_article(today).unwrap();
    let page2 = published_page(&storage, Some(cursor));
    assert_eq!(
        ids_of(&page2),
        [tie_b, tie_a],
        "the cursor's date carries it"
    );
    let page3 = published_page(&storage, page2.last().map(ListCursor::from));
    assert_eq!(ids_of(&page3), [late_backfill, mid]);
    let page4 = published_page(&storage, page3.last().map(ListCursor::from));
    assert_eq!(ids_of(&page4), [old, older]);
    assert_eq!(
        published_page(&storage, page4.last().map(ListCursor::from)),
        []
    );
    assert!(!ids_of(&page2).contains(&fresh));

    // The count of what remains after a cursor matches the walk.
    assert_eq!(
        storage
            .count_listed_articles(ListArticles {
                order: ListOrder::Published,
                after: Some(cursor),
                ..ListArticles::default()
            })
            .unwrap(),
        6
    );
}

fn ids_of(rows: &[ListedArticle]) -> Vec<ArticleId> {
    rows.iter().map(|row| row.id).collect()
}

/// The oldest-first order is the newest-first walk reversed, cursor and
/// all — the order a read-later queue is read in.
#[test]
fn oldest_order_pages_front_to_back() {
    let (_dir, storage) = temp_storage();
    let ids: Vec<ArticleId> = (0..5)
        .map(|i| insert_dated(&storage, &format!("q{i}"), None))
        .collect();
    let params = ListArticles {
        order: ListOrder::Oldest,
        ..ListArticles::default()
    };
    let walked = paged(
        &params,
        2,
        |p| storage.list_article_rows(p).unwrap(),
        |r| r.id,
    );
    assert_eq!(walked, ids);
    let rest = storage
        .list_articles(ListArticles {
            after: Some(ListCursor::id(ids[2])),
            ..params
        })
        .unwrap();
    assert_eq!(rest.iter().map(|a| a.id).collect::<Vec<_>>(), ids[3..]);
}

#[test]
fn search_rows_carry_state_and_tags_in_rank_order() {
    let (_dir, storage) = temp_storage();
    storage
        .upsert_articles(vec![
            article(None, "one", "borrowck borrowck borrowck"),
            article(None, "two", "borrowck once"),
            article(None, "three", "unrelated"),
        ])
        .unwrap();
    let ids = all_ids(&storage);
    storage.archive_article(ids[1]).unwrap();
    storage.tag_article(ids[1], "rust").unwrap();

    let rows = storage
        .search_article_rows(SearchArticles::new("borrowck", 10))
        .unwrap();
    let full = storage.search_articles("borrowck", 10).unwrap();
    let row_ids: Vec<ArticleId> = rows.iter().map(|r| r.id).collect();
    let full_ids: Vec<ArticleId> = full.iter().map(|a| a.id).collect();
    assert_eq!(row_ids, full_ids, "same hits, same rank order");
    assert_eq!(rows.len(), 2);
    let two = rows.iter().find(|r| r.id == ids[1]).unwrap();
    assert!(two.state.archived);
    assert_eq!(two.tags, ["rust"]);
    assert_eq!(two.snippet.as_deref(), Some("borrowck once"));
    assert_eq!(
        storage
            .search_article_rows(SearchArticles::new("   ", 10))
            .unwrap(),
        []
    );
    assert_eq!(
        storage
            .search_article_rows(SearchArticles::new("\"NEAR(", 10))
            .unwrap(),
        []
    );
}

// --------------------------------------------------------- unread counters

#[test]
fn unread_counts_follow_every_kind_of_write() {
    let (dir, storage) = temp_storage();
    let conn = side_conn(&dir);
    let check = |what: &str| {
        assert_eq!(
            storage.unread_counts().unwrap(),
            scanned_unread(&conn),
            "after {what}"
        );
    };
    let one = add_feed(&storage, "https://one.example/rss");
    let two = add_feed(&storage, "https://two.example/rss");
    storage
        .upsert_articles(vec![
            article(Some(one), "a", "x"),
            article(Some(one), "b", "x"),
            article(Some(two), "c", "x"),
            article(None, "d", "x"),
        ])
        .unwrap();
    check("insert");
    assert_eq!(
        storage.unread_counts().unwrap(),
        BTreeMap::from([(Some(one), 2), (Some(two), 1), (None, 1)])
    );
    let ids = all_ids(&storage);
    storage.mark_read(ids[0], true).unwrap();
    check("mark read");
    storage.archive_article(ids[1]).unwrap();
    check("archive");
    assert!(
        !storage.unread_counts().unwrap().contains_key(&Some(one)),
        "a feed with nothing unread is absent"
    );
    storage.star_article(ids[2]).unwrap();
    check("star (no effect)");
    storage.mark_read(ids[0], false).unwrap();
    check("mark unread");
    storage.unarchive_article(ids[1]).unwrap();
    check("unarchive");
    storage.mark_all_read(Some(one)).unwrap();
    check("mark all read in a feed");
    storage.mark_read_many(&ids, false).unwrap();
    check("mark many unread");
    storage.mark_read(ids[2], true).unwrap();
    storage.delete_article(ids[2]).unwrap();
    check("delete a read article");
    storage.delete_article(ids[3]).unwrap();
    check("delete an unread article");
    storage.archive_article(ids[1]).unwrap();
    storage.remove_feed(one).unwrap();
    check("remove a feed (its articles become feedless)");
    assert_eq!(
        storage.unread_counts().unwrap(),
        BTreeMap::from([(None, 1)])
    );
    storage.mark_all_read(None).unwrap();
    check("mark everything read");
    assert!(storage.unread_counts().unwrap().is_empty());
    // Re-ingesting identical rows writes nothing and counts nothing twice.
    storage
        .upsert_articles(vec![article(Some(two), "e", "x")])
        .unwrap();
    storage
        .upsert_articles(vec![article(Some(two), "e", "x changed")])
        .unwrap();
    check("re-ingest");
    assert_eq!(
        storage.unread_counts().unwrap(),
        BTreeMap::from([(Some(two), 1)])
    );
}

/// One write the property walk can make.
#[derive(Debug, Clone)]
enum Op {
    Insert {
        feed: usize,
    },
    Read {
        article: usize,
        read: bool,
    },
    Archive {
        article: usize,
        archived: bool,
    },
    Star {
        article: usize,
    },
    Delete {
        article: usize,
    },
    RemoveFeed {
        feed: usize,
    },
    MarkAllRead {
        feed: Option<usize>,
    },
    MarkManyUnread {
        from: usize,
    },
    FlagMany {
        from: usize,
        archive: bool,
        value: bool,
    },
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (0usize..4).prop_map(|feed| Op::Insert { feed }),
        3 => (any::<usize>(), any::<bool>())
            .prop_map(|(article, read)| Op::Read { article, read }),
        2 => (any::<usize>(), any::<bool>())
            .prop_map(|(article, archived)| Op::Archive { article, archived }),
        1 => any::<usize>().prop_map(|article| Op::Star { article }),
        1 => any::<usize>().prop_map(|article| Op::Delete { article }),
        1 => (0usize..3).prop_map(|feed| Op::RemoveFeed { feed }),
        1 => proptest::option::of(0usize..3).prop_map(|feed| Op::MarkAllRead { feed }),
        1 => any::<usize>().prop_map(|from| Op::MarkManyUnread { from }),
        2 => (any::<usize>(), any::<bool>(), any::<bool>())
            .prop_map(|(from, archive, value)| Op::FlagMany { from, archive, value }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Any sequence of writes leaves the counters equal to a fresh scan.
    #[test]
    fn unread_counters_always_equal_the_scan(ops in proptest::collection::vec(op(), 1..40)) {
        let (dir, storage) = temp_storage();
        let conn = side_conn(&dir);
        let mut feeds: Vec<FeedId> = (0..3)
            .map(|i| add_feed(&storage, &format!("https://f{i}.example/rss")))
            .collect();
        let mut keys = 0usize;
        for op in ops {
            let ids = all_ids(&storage);
            let pick = |n: usize| (!ids.is_empty()).then(|| ids[n % ids.len()]);
            match op {
                Op::Insert { feed } => {
                    keys += 1;
                    let feed = feeds.get(feed).copied();
                    storage.upsert_articles(vec![article(feed, &format!("p{keys}"), "x")]).unwrap();
                }
                Op::Read { article, read } => {
                    if let Some(id) = pick(article) {
                        storage.mark_read(id, read).unwrap();
                    }
                }
                Op::Archive { article, archived } => {
                    if let Some(id) = pick(article) {
                        if archived {
                            storage.archive_article(id).unwrap();
                        } else {
                            storage.unarchive_article(id).unwrap();
                        }
                    }
                }
                Op::Star { article } => {
                    if let Some(id) = pick(article) {
                        storage.star_article(id).unwrap();
                    }
                }
                Op::Delete { article } => {
                    if let Some(id) = pick(article) {
                        storage.delete_article(id).unwrap();
                    }
                }
                Op::RemoveFeed { feed } => {
                    if feed < feeds.len() {
                        storage.remove_feed(feeds.remove(feed)).unwrap();
                    }
                }
                Op::MarkAllRead { feed } => {
                    let feed = feed.and_then(|f| feeds.get(f).copied());
                    storage.mark_all_read(feed).unwrap();
                }
                Op::MarkManyUnread { from } => {
                    let tail: Vec<ArticleId> = ids.iter().skip(from % 4).copied().collect();
                    storage.mark_read_many(&tail, false).unwrap();
                }
                Op::FlagMany { from, archive, value } => {
                    let tail: Vec<ArticleId> = ids.iter().skip(from % 4).copied().collect();
                    let flag = if archive { ArticleFlag::Archived } else { ArticleFlag::Read };
                    storage.set_flag_many(&tail, flag, value).unwrap();
                }
            }
            prop_assert_eq!(storage.unread_counts().unwrap(), scanned_unread(&conn));
        }
    }
}

// ------------------------------------------------------------ migration

/// Migration 0012 backfills the counters from what a v11 database holds —
/// untouched, read, archived and feedless articles alike — and the
/// triggers carry on from there.
#[test]
fn migration_0012_backfills_the_unread_counters() {
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
            include_str!("../src/storage/migrations/0010_feed_schedule.sql"),
            include_str!("../src/storage/migrations/0011_screen_entry_links.sql"),
        ] {
            conn.execute_batch(sql).unwrap();
        }
        conn.pragma_update(None, "user_version", 11).unwrap();
        for (id, url) in [(1, "https://a.example/feed"), (2, "https://b.example/feed")] {
            conn.execute(
                "INSERT INTO feeds (id, url, added_at, modified_at) \
                 VALUES (?1, ?2, '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                (id, url),
            )
            .unwrap();
        }
        // Feed 1: three articles, one read; feed 2: two, one archived;
        // two feedless, one read.
        for (id, feed) in [
            (1, Some(1)),
            (2, Some(1)),
            (3, Some(1)),
            (4, Some(2)),
            (5, Some(2)),
            (6, None),
            (7, None),
        ] {
            conn.execute(
                "INSERT INTO articles (id, curio_id, feed_id, dedupe_key, source_url, saved_at, \
                 modified_at) VALUES (?1, ?2, ?3, ?4, 'https://x.example/', \
                 '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
                (
                    id,
                    format!("019780f0-0000-7000-8000-{id:012}"),
                    feed,
                    format!("k{id}"),
                ),
            )
            .unwrap();
        }
        for (id, read, starred, archived) in
            [(1, 1, 0, 0), (2, 0, 1, 0), (4, 0, 0, 1), (6, 1, 0, 0)]
        {
            conn.execute(
                "INSERT INTO article_state (article_id, is_read, is_starred, is_archived, \
                 updated_at) VALUES (?1, ?2, ?3, ?4, '2026-01-01T00:00:00.000Z')",
                (id, read, starred, archived),
            )
            .unwrap();
        }
    }

    let storage = Storage::open(&path).unwrap();
    assert_eq!(
        storage.db_schema_version().unwrap(),
        Storage::schema_version()
    );
    let expected = BTreeMap::from([(Some(FeedId(1)), 2), (Some(FeedId(2)), 1), (None, 1)]);
    assert_eq!(storage.unread_counts().unwrap(), expected);
    assert_eq!(scanned_unread(&side_conn(&dir)), expected);

    // The starred view pages through the new index and finds the migrated
    // member.
    let starred = storage
        .list_article_rows(ListArticles {
            starred: Some(true),
            ..ListArticles::default()
        })
        .unwrap();
    assert_eq!(
        starred.iter().map(|r| r.id).collect::<Vec<_>>(),
        [ArticleId(2)]
    );

    storage.mark_read(ArticleId(3), true).unwrap();
    assert_eq!(
        storage.unread_counts().unwrap(),
        BTreeMap::from([(Some(FeedId(1)), 1), (Some(FeedId(2)), 1), (None, 1)])
    );
}
