//! The article list and search queries: the [`ListArticles`] and
//! [`SearchArticles`] compilers (both projections — whole articles and
//! list rows) over one shared [`ArticleFilter`] clause builder, the
//! unread-count read, and the list-row parser.

use super::query::fts_match_expr;
use super::rows::{parse_curio_id, parse_opt_ts, parse_ts};
use super::{ARTICLE_COLS, ArticleFilter, ListArticles, SearchArticles};
use crate::model::{ArticleId, ArticleState, FeedId, ListedArticle, SNIPPET_SOURCE_CHARS, snippet};
use crate::storage::StorageError;
use rusqlite::Row;
use rusqlite::types::ToSql;

/// Which columns a list or search query selects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Projection {
    /// Every article column, body included — [`RawArticle`].
    Full,
    /// The list-row columns — [`RawListed`]: no body, a bounded text
    /// prefix, the state flags and the tags.
    Listed,
}

/// The state flags a membership view pages through on its partial index
/// (migration 0012), in the order one is picked when a filter requires
/// several. These sets are sparse: walking the index newest-first reads
/// only member rows, where walking `articles` would probe every article's
/// state until a page fills.
const MEMBERSHIP_INDEXES: [(&str, &str); 3] = [
    ("is_starred", "idx_article_state_starred"),
    ("is_read_later", "idx_article_state_read_later"),
    ("is_archived", "idx_article_state_archived"),
];

/// The [`Projection::Listed`] select list, over `articles a` and an
/// `article_state s` join. The text prefix is one character longer than
/// the snippet reads, so the parser can tell that the text runs on; the
/// tags come back as one JSON array per row.
fn listed_cols() -> String {
    format!(
        "a.id, a.curio_id, a.feed_id, a.title, a.source_url, a.author, a.published_at, a.lang, \
         a.word_count, a.saved_at, a.lead_image, substr(a.content_text, 1, {}), \
         COALESCE(s.is_read, 0), COALESCE(s.is_starred, 0), COALESCE(s.is_read_later, 0), \
         COALESCE(s.is_archived, 0), \
         (SELECT json_group_array(t.name) FROM article_tags at JOIN tags t ON t.id = at.tag_id \
          WHERE at.article_id = a.id)",
        SNIPPET_SOURCE_CHARS + 1
    )
}

/// Appends the [`ArticleFilter`] clauses (static fragments, every value a
/// bind) over the `a` (articles) and `s` (`article_state`) aliases; the
/// caller provides the state join whenever a state flag is filtered. A
/// missing state row defaults every flag to 0 via `COALESCE`. `driving`
/// names the state column the query already pages through (a membership
/// index), which is not re-tested.
fn push_filter_clauses(
    filter: &ArticleFilter,
    driving: Option<&str>,
    clauses: &mut Vec<String>,
    binds: &mut Vec<Box<dyn ToSql>>,
) {
    if let Some(feed) = filter.feed_id {
        clauses.push("a.feed_id = ?".to_owned());
        binds.push(Box::new(feed.0));
    }
    for (column, want) in filter.state_flags() {
        if driving == Some(column) {
            continue;
        }
        if let Some(want) = want {
            clauses.push(format!("COALESCE(s.{column}, 0) = ?"));
            binds.push(Box::new(want));
        }
    }
    if let Some(tag) = &filter.tag {
        clauses.push(
            "EXISTS (SELECT 1 FROM article_tags at JOIN tags t ON t.id = at.tag_id \
             WHERE at.article_id = a.id AND t.name = ?)"
                .to_owned(),
        );
        binds.push(Box::new(tag.trim().to_owned()));
    }
    if let Some(feed_tag) = &filter.feed_tag {
        // Folder scope: the feed carries a path tag equal to `feed_tag` or
        // nested beneath it (`Tech` → `Tech`, `Tech/Databases`, …). The
        // subtree test is an exact, case-sensitive prefix compare — never
        // `LIKE`, whose `%`/`_` wildcards would let folder `a_b` match
        // `axb/…` and whose ASCII case folding the `=` branch lacks.
        let feed_tag = feed_tag.trim().to_owned();
        clauses.push(
            "a.feed_id IN (SELECT f.id FROM feeds f, json_each(f.tags) je \
             WHERE je.value = ? OR substr(je.value, 1, length(?) + 1) = ? || '/')"
                .to_owned(),
        );
        binds.push(Box::new(feed_tag.clone()));
        binds.push(Box::new(feed_tag.clone()));
        binds.push(Box::new(feed_tag));
    }
}

/// Compiles [`ListArticles`] into SQL + positional binds. Static clause
/// fragments only — no user input ever lands in the SQL string (the tag
/// value is a bind). A missing state row defaults every flag to 0 via
/// `COALESCE`. The query takes one of three shapes:
///
/// - a filter that requires a [`MEMBERSHIP_INDEXES`] flag pages through
///   that flag's partial index (`INDEXED BY`, so the plan cannot drift
///   back to a scan) and joins each member to its article — `CROSS JOIN`
///   pins that loop order;
/// - otherwise, when a state filter or the [`Projection::Listed`] columns
///   need the flags, `articles` is walked newest-first with its state row
///   `LEFT JOIN`ed;
/// - and the unfiltered [`Projection::Full`] path stays join-free.
///
/// Every shape orders by article id, newest first, keyset-paginated by
/// `before` — filters never change the order.
pub(super) fn list_articles_query(
    params: &ListArticles,
    projection: Projection,
) -> (String, Vec<Box<dyn ToSql>>) {
    let cols = match projection {
        Projection::Full => article_cols_prefixed("a"),
        Projection::Listed => listed_cols(),
    };
    let Filtered {
        from,
        key,
        filter,
        mut binds,
    } = filtered_articles(params, projection == Projection::Listed);
    let sql = format!("SELECT {cols} FROM {from} WHERE {filter} ORDER BY {key} DESC LIMIT ?");
    binds.push(Box::new(params.limit));
    (sql, binds)
}

/// Compiles [`ListArticles`] into a `COUNT(*)` of every article its
/// filters (and `before` cursor) keep across every page — `limit` plays no
/// part. The same `FROM … WHERE` as [`list_articles_query`]'s whole
/// articles, so the count is exactly what paging through it walks.
pub(super) fn count_articles_query(params: &ListArticles) -> (String, Vec<Box<dyn ToSql>>) {
    let Filtered {
        from,
        filter,
        binds,
        ..
    } = filtered_articles(params, false);
    (format!("SELECT COUNT(*) FROM {from} WHERE {filter}"), binds)
}

/// The `FROM` and `WHERE` a [`ListArticles`] compiles to, its binds, and
/// the keyset column the list orders and pages by.
struct Filtered {
    from: String,
    key: &'static str,
    filter: String,
    binds: Vec<Box<dyn ToSql>>,
}

/// The shared half of [`list_articles_query`] and
/// [`count_articles_query`]: picks the query shape (see the former) and
/// builds the filter clauses. `listed` says the select list needs the
/// state flags, so `article_state` is joined even with no state filter.
fn filtered_articles(params: &ListArticles, listed: bool) -> Filtered {
    let filter = params.filter();
    let state_filters = filter.state_flags();
    let membership = MEMBERSHIP_INDEXES.iter().find(|(column, _)| {
        state_filters
            .iter()
            .any(|(filtered, want)| filtered == column && *want == Some(true))
    });
    let needs_state = listed || state_filters.iter().any(|(_, want)| want.is_some());
    let (from, key) = match membership {
        Some((_, index)) => (
            format!(
                "article_state s INDEXED BY {index} CROSS JOIN articles a ON a.id = s.article_id"
            ),
            "s.article_id",
        ),
        None if needs_state => (
            "articles a LEFT JOIN article_state s ON s.article_id = a.id".to_owned(),
            "a.id",
        ),
        None => ("articles a".to_owned(), "a.id"),
    };
    let mut clauses = vec![format!("{key} < ?")];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(params.before.map_or(i64::MAX, |id| id.0))];
    if let Some((column, _)) = membership {
        // A literal, not a bind: a partial index only serves a query whose
        // WHERE visibly implies the index's own `= 1`.
        clauses.push(format!("s.{column} = 1"));
    }
    push_filter_clauses(
        &filter,
        membership.map(|(column, _)| *column),
        &mut clauses,
        &mut binds,
    );
    Filtered {
        from,
        key,
        filter: clauses.join(" AND "),
        binds,
    }
}

/// The per-feed unread counts: one `feed_unread` row per feed (key 0 =
/// no feed), kept by migration 0012's triggers — never a scan of the
/// articles.
pub(super) const UNREAD_COUNTS: &str = "SELECT feed_key, unread FROM feed_unread WHERE unread > 0";

/// BM25 column weights for `articles_fts(title, content_text)`: a title
/// hit outranks a body hit.
const SEARCH_RANK: &str = "bm25(articles_fts, 10.0, 1.0)";

/// Compiles [`SearchArticles`] into SQL + positional binds: the escaped
/// FTS expression (see `query`) is the first bind, then the
/// [`ArticleFilter`] clauses exactly as [`list_articles_query`] applies
/// them, ordered by the title-weighted BM25 rank with `id DESC` breaking
/// ties. The `article_state` join appears only when the projection or a
/// state filter needs it. `None` when the query has nothing to match.
pub(super) fn search_articles_query(
    params: &SearchArticles,
    projection: Projection,
) -> Option<(String, Vec<Box<dyn ToSql>>)> {
    let expr = fts_match_expr(&params.query)?;
    let cols = match projection {
        Projection::Full => article_cols_prefixed("a"),
        Projection::Listed => listed_cols(),
    };
    let needs_state = projection == Projection::Listed
        || params
            .filter
            .state_flags()
            .iter()
            .any(|(_, want)| want.is_some());
    let join = if needs_state {
        " LEFT JOIN article_state s ON s.article_id = a.id"
    } else {
        ""
    };
    let mut clauses = vec!["articles_fts MATCH ?".to_owned()];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(expr)];
    push_filter_clauses(&params.filter, None, &mut clauses, &mut binds);
    let sql = format!(
        "SELECT {cols} FROM articles_fts JOIN articles a ON a.id = articles_fts.rowid{join} \
         WHERE {} ORDER BY {SEARCH_RANK}, a.id DESC LIMIT ?",
        clauses.join(" AND ")
    );
    binds.push(Box::new(params.limit));
    Some((sql, binds))
}

pub(super) fn article_cols_prefixed(alias: &str) -> String {
    ARTICLE_COLS
        .split(',')
        .map(|col| format!("{alias}.{}", col.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Raw row image of a [`Projection::Listed`] row: primitives out of
/// rusqlite, parsed into the domain type in a second step so parse
/// failures surface as [`StorageError::Corrupt`], not panics.
pub(super) struct RawListed {
    id: i64,
    curio_id: String,
    feed_id: Option<i64>,
    title: String,
    source_url: String,
    author: Option<String>,
    published_at: Option<String>,
    lang: Option<String>,
    word_count: Option<u32>,
    saved_at: String,
    lead_image: Option<String>,
    text_prefix: String,
    state: ArticleState,
    tags: String,
}

pub(super) fn raw_listed(row: &Row<'_>) -> rusqlite::Result<RawListed> {
    Ok(RawListed {
        id: row.get(0)?,
        curio_id: row.get(1)?,
        feed_id: row.get(2)?,
        title: row.get(3)?,
        source_url: row.get(4)?,
        author: row.get(5)?,
        published_at: row.get(6)?,
        lang: row.get(7)?,
        word_count: row.get(8)?,
        saved_at: row.get(9)?,
        lead_image: row.get(10)?,
        text_prefix: row.get(11)?,
        state: ArticleState {
            read: row.get(12)?,
            starred: row.get(13)?,
            read_later: row.get(14)?,
            archived: row.get(15)?,
        },
        tags: row.get(16)?,
    })
}

impl RawListed {
    pub(super) fn into_listed(self) -> Result<ListedArticle, StorageError> {
        let mut tags: Vec<String> =
            serde_json::from_str(&self.tags).map_err(|err| StorageError::Corrupt {
                column: "tags.name",
                message: err.to_string(),
            })?;
        tags.sort_unstable();
        // The query reads one character past the snippet's source window;
        // that character's presence says the text runs on.
        let mut prefix = self.text_prefix;
        let runs_on = match prefix.char_indices().nth(SNIPPET_SOURCE_CHARS) {
            Some((at, _)) => {
                prefix.truncate(at);
                true
            }
            None => false,
        };
        Ok(ListedArticle {
            id: ArticleId(self.id),
            curio_id: parse_curio_id(&self.curio_id)?,
            feed_id: self.feed_id.map(FeedId),
            title: self.title,
            source_url: self.source_url,
            author: self.author,
            published_at: parse_opt_ts("articles.published_at", self.published_at)?,
            lang: self.lang,
            word_count: self.word_count,
            saved_at: parse_ts("articles.saved_at", &self.saved_at)?,
            lead_image: self.lead_image,
            snippet: snippet(&prefix, runs_on),
            state: self.state,
            tags,
        })
    }
}

#[cfg(test)]
mod tests {
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
}
