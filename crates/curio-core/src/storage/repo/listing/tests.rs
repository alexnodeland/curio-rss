//! List and search query compiler tests: the SQL each filter, order and
//! cursor compiles to, and the query plans that keep a page an index walk.

#![allow(clippy::unwrap_used)]

use super::*;
use crate::model::FeedId;
use rusqlite::Connection;

#[test]
fn article_cols_prefixed_prefixes_every_column() {
    let cols = article_cols_prefixed("a");
    assert!(cols.starts_with("a.id, a.curio_id"));
    assert!(cols.ends_with("a.lead_image"));
    assert!(!cols.contains(" ,"));
}

#[test]
fn unfiltered_list_query_stays_join_free_with_keyset_order() {
    let (sql, binds) = list_articles_query(&ListArticles::default(), Projection::Full);
    assert!(!sql.contains("JOIN"), "no filters → no joins: {sql}");
    assert!(sql.ends_with("ORDER BY a.id DESC LIMIT ?"));
    assert_eq!(binds.len(), 2, "before + limit");
}

#[test]
fn state_filters_join_and_coalesce_missing_rows() {
    let (sql, binds) = list_articles_query(
        &ListArticles {
            feed_id: Some(FeedId(7)),
            read: Some(false),
            archived: Some(false),
            ..ListArticles::default()
        },
        Projection::Full,
    );
    assert!(sql.contains("FROM articles a LEFT JOIN article_state s ON s.article_id = a.id"));
    assert!(sql.contains("COALESCE(s.is_read, 0) = ?"));
    assert!(sql.contains("COALESCE(s.is_archived, 0) = ?"));
    assert!(sql.contains("a.feed_id = ?"));
    assert!(!sql.contains("is_starred"), "unset filters stay out");
    assert!(!sql.contains("INDEXED BY"), "no set membership required");
    assert!(sql.ends_with("ORDER BY a.id DESC LIMIT ?"), "order fixed");
    assert_eq!(binds.len(), 5, "before + feed + 2 flags + limit");
}

#[test]
fn a_required_membership_pages_through_its_partial_index() {
    let (sql, binds) = list_articles_query(
        &ListArticles {
            feed_id: Some(FeedId(7)),
            read: Some(false),
            archived: Some(true),
            ..ListArticles::default()
        },
        Projection::Full,
    );
    assert!(
        sql.contains(
            "FROM article_state s INDEXED BY idx_article_state_archived \
             CROSS JOIN articles a ON a.id = s.article_id"
        ),
        "{sql}"
    );
    assert!(sql.contains("s.article_id < ?"), "keyset on the index");
    assert!(
        sql.contains("s.is_archived = 1"),
        "literal, so the index applies"
    );
    assert!(
        !sql.contains("COALESCE(s.is_archived"),
        "the driver is not re-tested"
    );
    assert!(sql.contains("COALESCE(s.is_read, 0) = ?"));
    assert!(sql.ends_with("ORDER BY s.article_id DESC LIMIT ?"));
    assert_eq!(binds.len(), 4, "before + feed + read + limit");
}

#[test]
fn the_first_required_membership_in_index_order_drives() {
    let (sql, _) = list_articles_query(
        &ListArticles {
            starred: Some(true),
            read_later: Some(true),
            archived: Some(true),
            ..ListArticles::default()
        },
        Projection::Listed,
    );
    assert!(
        sql.contains("INDEXED BY idx_article_state_starred"),
        "{sql}"
    );
    assert!(sql.contains("COALESCE(s.is_read_later, 0) = ?"));
    assert!(sql.contains("COALESCE(s.is_archived, 0) = ?"));
    // Excluding a set (`= false`) never drives: most articles qualify.
    let (sql, _) = list_articles_query(
        &ListArticles {
            starred: Some(false),
            ..ListArticles::default()
        },
        Projection::Listed,
    );
    assert!(!sql.contains("INDEXED BY"), "{sql}");
}

/// The query plan `SQLite` picks for `sql` on a migrated database, one
/// detail line per step.
fn plan(sql: &str, binds: &[Box<dyn ToSql>]) -> Vec<String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    drop(crate::storage::Storage::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}")).unwrap();
    stmt.query_map(
        rusqlite::params_from_iter(binds.iter().map(Box::as_ref)),
        |row| row.get::<_, String>(3),
    )
    .unwrap()
    .collect::<Result<_, _>>()
    .unwrap()
}

#[test]
fn a_membership_view_never_scans_the_articles() {
    for (params, index) in [
        (
            ListArticles {
                starred: Some(true),
                archived: Some(false),
                ..ListArticles::default()
            },
            "idx_article_state_starred",
        ),
        (
            ListArticles {
                read_later: Some(true),
                archived: Some(false),
                ..ListArticles::default()
            },
            "idx_article_state_read_later",
        ),
        (
            ListArticles {
                archived: Some(true),
                ..ListArticles::default()
            },
            "idx_article_state_archived",
        ),
    ] {
        let (sql, binds) = list_articles_query(&params, Projection::Listed);
        let steps = plan(&sql, &binds);
        assert!(
            steps
                .iter()
                .any(|step| step.starts_with("SEARCH s USING") && step.contains(index)),
            "{params:?}: {steps:?}"
        );
        assert!(
            steps
                .iter()
                .any(|step| step.starts_with("SEARCH a USING INTEGER PRIMARY KEY")),
            "each member is one article lookup: {steps:?}"
        );
        assert!(
            !steps.iter().any(|step| step.starts_with("SCAN")),
            "no full scan: {steps:?}"
        );
        assert!(
            !steps.iter().any(|step| step.contains("TEMP B-TREE")),
            "the index order is the list order — no sort: {steps:?}"
        );
    }
}

#[test]
fn the_default_list_walks_articles_newest_first_without_a_sort() {
    let params = ListArticles {
        archived: Some(false),
        ..ListArticles::default()
    };
    let (sql, binds) = list_articles_query(&params, Projection::Listed);
    let steps = plan(&sql, &binds);
    assert!(
        steps
            .iter()
            .any(|step| step.starts_with("SEARCH a USING INTEGER PRIMARY KEY (rowid<?)")),
        "{steps:?}"
    );
    assert!(
        !steps.iter().any(|step| step.contains("TEMP B-TREE")),
        "{steps:?}"
    );
}

fn dated_cursor() -> ListCursor {
    ListCursor {
        id: ArticleId(42),
        dated: Some("2026-01-02T03:04:05Z".parse().unwrap()),
    }
}

#[test]
fn each_order_compiles_its_keyset() {
    let compile = |order, after| {
        list_articles_query(
            &ListArticles {
                order,
                after,
                ..ListArticles::default()
            },
            Projection::Full,
        )
    };
    let (sql, binds) = compile(ListOrder::Oldest, Some(ListCursor::id(ArticleId(9))));
    assert!(sql.contains("WHERE a.id > ?"), "{sql}");
    assert!(sql.ends_with("ORDER BY a.id ASC LIMIT ?"), "{sql}");
    assert_eq!(binds.len(), 2, "after + limit");

    let (sql, binds) = compile(ListOrder::Published, None);
    assert!(!sql.contains("WHERE"), "no cursor, no filter: {sql}");
    assert!(
        sql.ends_with("ORDER BY COALESCE(a.published_at, a.saved_at) DESC, a.id DESC LIMIT ?"),
        "{sql}"
    );
    assert_eq!(binds.len(), 1, "limit");

    let (sql, binds) = compile(ListOrder::Published, Some(dated_cursor()));
    assert!(
        sql.contains(
            "WHERE COALESCE(a.published_at, a.saved_at) <= ? \
             AND (COALESCE(a.published_at, a.saved_at) < ? OR a.id < ?)"
        ),
        "{sql}"
    );
    assert_eq!(binds.len(), 4, "date ×2 + id + limit");

    // The id orders ignore a cursor's date.
    let (sql, _) = compile(ListOrder::Newest, Some(dated_cursor()));
    assert!(!sql.contains("COALESCE"), "{sql}");
}

#[test]
fn a_membership_view_keeps_its_index_in_every_order() {
    for (order, index) in [
        (ListOrder::Newest, "idx_article_state_starred "),
        (ListOrder::Oldest, "idx_article_state_starred "),
        (ListOrder::Published, "idx_article_state_starred_dated "),
    ] {
        let (sql, _) = list_articles_query(
            &ListArticles {
                starred: Some(true),
                order,
                after: Some(dated_cursor()),
                ..ListArticles::default()
            },
            Projection::Listed,
        );
        assert!(sql.contains(&format!("INDEXED BY {index}")), "{sql}");
    }
    let (sql, binds) = list_articles_query(
        &ListArticles {
            read_later: Some(true),
            order: ListOrder::Published,
            after: Some(dated_cursor()),
            ..ListArticles::default()
        },
        Projection::Listed,
    );
    assert!(
        sql.contains("WHERE s.dated <= ? AND (s.dated < ? OR s.article_id < ?)"),
        "the view's copy of the date, and its id: {sql}"
    );
    assert!(
        sql.ends_with("ORDER BY s.dated DESC, s.article_id DESC LIMIT ?"),
        "{sql}"
    );
    assert_eq!(binds.len(), 4, "date ×2 + id + limit");
    let (sql, _) = list_articles_query(
        &ListArticles {
            starred: Some(true),
            order: ListOrder::Oldest,
            ..ListArticles::default()
        },
        Projection::Listed,
    );
    assert!(sql.contains("s.article_id > ?"), "{sql}");
    assert!(sql.ends_with("ORDER BY s.article_id ASC LIMIT ?"), "{sql}");
}

/// Asserts that a plan never sorts: a published-order page walks an index
/// already in list order.
fn assert_walks_without_sorting(steps: &[String], what: &str) {
    assert!(
        !steps.iter().any(|step| step.contains("TEMP B-TREE")),
        "{what}: the index order is the list order — no sort: {steps:?}"
    );
}

#[test]
fn a_membership_view_pages_the_published_order_through_its_date_index() {
    for (params, index) in [
        (
            ListArticles {
                starred: Some(true),
                archived: Some(false),
                ..ListArticles::default()
            },
            "idx_article_state_starred_dated",
        ),
        (
            ListArticles {
                read_later: Some(true),
                archived: Some(false),
                ..ListArticles::default()
            },
            "idx_article_state_read_later_dated",
        ),
        (
            ListArticles {
                archived: Some(true),
                ..ListArticles::default()
            },
            "idx_article_state_archived_dated",
        ),
    ] {
        for after in [None, Some(dated_cursor())] {
            let params = ListArticles {
                order: ListOrder::Published,
                after,
                ..params.clone()
            };
            let (sql, binds) = list_articles_query(&params, Projection::Listed);
            let steps = plan(&sql, &binds);
            let walk = if after.is_some() { "SEARCH" } else { "SCAN" };
            assert!(
                steps
                    .iter()
                    .any(|step| step.starts_with(&format!("{walk} s USING INDEX {index}"))),
                "{params:?}: {steps:?}"
            );
            assert!(
                steps
                    .iter()
                    .any(|step| step.starts_with("SEARCH a USING INTEGER PRIMARY KEY")),
                "each member is one article lookup: {steps:?}"
            );
            assert!(
                !steps.iter().any(|step| step.starts_with("SCAN a")),
                "no walk of the articles: {steps:?}"
            );
            assert_walks_without_sorting(&steps, &format!("{params:?}"));
        }
    }
}

#[test]
fn the_published_list_walks_the_date_index_without_a_sort() {
    for after in [None, Some(dated_cursor())] {
        let params = ListArticles {
            order: ListOrder::Published,
            after,
            archived: Some(false),
            ..ListArticles::default()
        };
        let (sql, binds) = list_articles_query(&params, Projection::Listed);
        let steps = plan(&sql, &binds);
        assert!(
            steps
                .iter()
                .any(|step| step.contains("a USING INDEX idx_articles_dated")),
            "{after:?}: {steps:?}"
        );
        assert_walks_without_sorting(&steps, &format!("{after:?}"));
        if after.is_some() {
            assert!(
                steps
                    .iter()
                    .any(|step| step.starts_with("SEARCH a USING INDEX")),
                "the cursor seeks the index rather than scanning past: {steps:?}"
            );
        }
    }
}

/// A migrated database holding one article (id 1), dated 2025-01-02 —
/// published, or else saved then — for the cursor resolution tests.
fn one_article_db(published: Option<&str>) -> (tempfile::TempDir, Connection) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("curio.db");
    drop(crate::storage::Storage::open(&path).unwrap());
    let conn = Connection::open(&path).unwrap();
    conn.execute(
        "INSERT INTO articles (id, curio_id, dedupe_key, source_url, published_at, saved_at, \
         modified_at) VALUES (1, '019780f0-0000-7000-8000-000000000001', 'm:k1', \
         'https://x.example/', ?1, ?2, ?2)",
        (
            published,
            if published.is_some() {
                "2026-01-01T00:00:00.000Z"
            } else {
                "2025-01-02T00:00:00.000Z"
            },
        ),
    )
    .unwrap();
    (dir, conn)
}

#[test]
fn a_bare_published_cursor_resolves_to_its_rows_date() {
    for published in [Some("2025-01-02T00:00:00.000Z"), None] {
        let (_dir, conn) = one_article_db(published);
        let params = ListArticles {
            order: ListOrder::Published,
            after: Some(ListCursor::id(ArticleId(1))),
            ..ListArticles::default()
        };
        let resolved = resolve_cursor(&conn, params.clone()).unwrap();
        assert_eq!(
            resolved.after,
            Some(ListCursor {
                id: ArticleId(1),
                dated: Some("2025-01-02T00:00:00Z".parse().unwrap()),
            }),
            "{published:?}"
        );
        // Resolved, it compiles to a bound date — never a NULL.
        let (_, binds) = list_articles_query(&resolved, Projection::Full);
        assert_eq!(binds.len(), 4);

        // A cursor that carries its date, or an id order, is left alone.
        let carried = ListArticles {
            after: Some(dated_cursor()),
            ..params.clone()
        };
        assert_eq!(resolve_cursor(&conn, carried.clone()).unwrap(), carried);
        let newest = ListArticles {
            order: ListOrder::Newest,
            after: Some(ListCursor::id(ArticleId(7))),
            ..ListArticles::default()
        };
        assert_eq!(resolve_cursor(&conn, newest.clone()).unwrap(), newest);
    }
}

#[test]
fn a_bare_published_cursor_of_a_missing_row_is_not_found() {
    let (_dir, conn) = one_article_db(None);
    let params = ListArticles {
        order: ListOrder::Published,
        after: Some(ListCursor::id(ArticleId(2))),
        ..ListArticles::default()
    };
    assert!(matches!(
        resolve_cursor(&conn, params),
        Err(StorageError::NotFound { entity: "article" })
    ));
}

#[test]
fn the_oldest_list_walks_articles_forward_without_a_sort() {
    let params = ListArticles {
        order: ListOrder::Oldest,
        archived: Some(false),
        ..ListArticles::default()
    };
    let (sql, binds) = list_articles_query(&params, Projection::Listed);
    let steps = plan(&sql, &binds);
    assert!(
        steps
            .iter()
            .any(|step| step.starts_with("SEARCH a USING INTEGER PRIMARY KEY (rowid>?)")),
        "{steps:?}"
    );
    assert!(
        !steps.iter().any(|step| step.contains("TEMP B-TREE")),
        "{steps:?}"
    );
}

#[test]
fn unread_counts_read_the_counters_not_the_articles() {
    let steps = plan(UNREAD_COUNTS, &[]);
    assert!(
        steps.iter().all(|step| !step.contains("articles")),
        "{steps:?}"
    );
}

#[test]
fn listed_rows_always_join_state_and_never_select_the_body() {
    let (sql, binds) = list_articles_query(&ListArticles::default(), Projection::Listed);
    assert!(sql.contains("LEFT JOIN article_state s ON s.article_id = a.id"));
    assert!(!sql.contains("content_html"), "{sql}");
    assert!(
        sql.contains(&format!(
            "substr(a.content_text, 1, {})",
            SNIPPET_SOURCE_CHARS + 1
        )),
        "only a bounded prefix of the text: {sql}"
    );
    assert!(sql.contains("json_group_array(t.name)"));
    assert_eq!(binds.len(), 2, "before + limit");
    let (search, _) =
        search_articles_query(&SearchArticles::new("rust", 5), Projection::Listed).unwrap();
    assert!(!search.contains("content_html"), "{search}");
    assert!(search.contains("LEFT JOIN article_state s ON s.article_id = a.id"));
}

#[test]
fn search_query_ranks_title_first_and_reuses_the_list_filters() {
    let (sql, binds) = search_articles_query(
        &SearchArticles {
            query: "rust".to_owned(),
            filter: ArticleFilter {
                feed_id: Some(FeedId(3)),
                read: Some(false),
                feed_tag: Some("Tech".to_owned()),
                ..ArticleFilter::default()
            },
            limit: 20,
        },
        Projection::Full,
    )
    .unwrap();
    assert!(sql.contains("articles_fts MATCH ?"));
    assert!(sql.contains("a.feed_id = ?"));
    assert!(sql.contains("LEFT JOIN article_state s"));
    assert!(sql.contains("COALESCE(s.is_read, 0) = ?"));
    assert!(!sql.contains("INDEXED BY"), "the FTS match drives a search");
    assert!(
        sql.ends_with("ORDER BY bm25(articles_fts, 10.0, 1.0), a.id DESC LIMIT ?"),
        "{sql}"
    );
    assert_eq!(binds.len(), 7, "match + feed + read + feed_tag ×3 + limit");
}

#[test]
fn unscoped_full_search_query_stays_join_free() {
    let (sql, binds) =
        search_articles_query(&SearchArticles::new("rust", 5), Projection::Full).unwrap();
    assert!(!sql.contains("article_state"), "{sql}");
    assert_eq!(binds.len(), 2, "match + limit");
    assert!(search_articles_query(&SearchArticles::new(" - ", 5), Projection::Full).is_none());
}

#[test]
fn tag_filter_binds_the_tag_instead_of_splicing_it() {
    let (sql, binds) = list_articles_query(
        &ListArticles {
            tag: Some("rust'; DROP TABLE articles; --".to_owned()),
            ..ListArticles::default()
        },
        Projection::Full,
    );
    assert!(sql.contains("t.name = ?"));
    assert!(!sql.contains("DROP TABLE"), "tag value never enters SQL");
    assert!(
        !sql.contains("article_state"),
        "tag alone needs no state join"
    );
    assert_eq!(binds.len(), 3, "before + tag + limit");
}

#[test]
fn feed_tag_filter_matches_the_folder_subtree_via_bound_params() {
    let (sql, binds) = list_articles_query(
        &ListArticles {
            feed_tag: Some("Tech".to_owned()),
            ..ListArticles::default()
        },
        Projection::Full,
    );
    assert!(sql.contains("json_each(f.tags)"));
    assert!(
        sql.contains("je.value = ? OR substr(je.value, 1, length(?) + 1) = ? || '/'"),
        "matches the folder itself and its subtree: {sql}"
    );
    assert!(!sql.contains("LIKE"), "no wildcard matching: {sql}");
    assert!(
        !sql.contains("article_state"),
        "feed_tag alone needs no state join"
    );
    assert_eq!(
        binds.len(),
        5,
        "before + feed_tag (exact + subtree ×2) + limit"
    );
}
