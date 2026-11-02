//! The article list and search queries: the [`ListArticles`] and
//! [`SearchArticles`] compilers (both projections — whole articles and
//! list rows) over one shared [`ArticleFilter`] clause builder, the
//! unread-count read, and the list-row parser.

use super::query::fts_match_expr;
use super::rows::{parse_curio_id, parse_opt_ts, parse_ts};
use super::state::decode_progress;
use super::{ARTICLE_COLS, ArticleFilter, ListArticles, ListCursor, ListOrder, SearchArticles};
use crate::model::{
    ArticleId, ArticleState, FeedId, ListedArticle, ReadProgress, SNIPPET_SOURCE_CHARS, snippet,
};
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
         COALESCE(s.is_archived, 0), COALESCE(s.read_permille, 0), \
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

/// The publication date [`ListOrder::Published`] sorts by, over the `a`
/// (articles) alias: `published_at`, an undated article standing at its
/// `saved_at`. Migration 0015 indexes exactly this expression — `SQLite`
/// uses an expression index only for a query that spells the same
/// expression — and the index's implicit trailing rowid is the id
/// tie-break, so a published-order page walks the index with no sort.
///
/// Both columns hold the one fixed-width form every write produces
/// ([`curio_types::Timestamp`]'s `YYYY-MM-DDTHH:MM:SS.mmmZ`), so text
/// order is time order, and a cursor's date binds as the very text its
/// row stores.
const DATED: &str = "COALESCE(a.published_at, a.saved_at)";

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
///   need the flags, `articles` is walked in list order with its state
///   row `LEFT JOIN`ed;
/// - and the unfiltered [`Projection::Full`] path stays join-free.
///
/// Every shape orders by [`ListArticles::order`], keyset-paginated by
/// [`ListArticles::after`] — filters never change the order. The id
/// orders walk the driving table's key, so a page reads only its rows;
/// [`ListOrder::Published`] walks the [`DATED`] index, except in a
/// membership view, which sorts its (sparse) member set instead.
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
        order_by,
        filter,
        mut binds,
    } = filtered_articles(params, projection == Projection::Listed);
    let sql = format!("SELECT {cols} FROM {from}{filter} ORDER BY {order_by} LIMIT ?");
    binds.push(Box::new(params.limit));
    (sql, binds)
}

/// Compiles [`ListArticles`] into a `COUNT(*)` of every article its
/// filters (and `after` cursor) keep across every page — `limit` plays no
/// part. The same `FROM … WHERE` as [`list_articles_query`]'s whole
/// articles, so the count is exactly what paging through it walks.
pub(super) fn count_articles_query(params: &ListArticles) -> (String, Vec<Box<dyn ToSql>>) {
    let Filtered {
        from,
        filter,
        binds,
        ..
    } = filtered_articles(params, false);
    (format!("SELECT COUNT(*) FROM {from}{filter}"), binds)
}

/// The `FROM` and `WHERE` (keyword included; empty when nothing filters)
/// a [`ListArticles`] compiles to, its binds, and the `ORDER BY` terms
/// the list pages by.
struct Filtered {
    from: String,
    order_by: String,
    filter: String,
    binds: Vec<Box<dyn ToSql>>,
}

/// The keyset half of a list query: pushes the clause (and its binds)
/// that resumes after `cursor` in `order`, and returns the `ORDER BY`
/// terms. `id` is the id column the query shape walks — the membership
/// index's `s.article_id` or `a.id`. The id orders always bind a bound
/// (the far end of the key range when there is no cursor), so each walk
/// is a range search on the key; the date order adds its clause only with
/// a cursor.
fn keyset(
    order: ListOrder,
    cursor: Option<&ListCursor>,
    id: &str,
    clauses: &mut Vec<String>,
    binds: &mut Vec<Box<dyn ToSql>>,
) -> String {
    match order {
        ListOrder::Newest => {
            clauses.push(format!("{id} < ?"));
            binds.push(Box::new(cursor.map_or(i64::MAX, |c| c.id.0)));
            format!("{id} DESC")
        }
        ListOrder::Oldest => {
            clauses.push(format!("{id} > ?"));
            binds.push(Box::new(cursor.map_or(i64::MIN, |c| c.id.0)));
            format!("{id} ASC")
        }
        ListOrder::Published => {
            if let Some(cursor) = cursor {
                // `(date, id) < (cursor date, cursor id)`, spelled so the
                // planner can seek: the `<=` bounds the index range (a
                // row-value compare would not, as the id is the index's
                // implicit rowid), the `OR` settles the date's ties.
                let date = if cursor.dated.is_some() {
                    "?"
                } else {
                    "(SELECT COALESCE(published_at, saved_at) FROM articles WHERE id = ?)"
                };
                clauses.push(format!(
                    "{DATED} <= {date} AND ({DATED} < {date} OR a.id < ?)"
                ));
                for _ in 0..2 {
                    match cursor.dated {
                        Some(dated) => binds.push(Box::new(dated.to_string())),
                        None => binds.push(Box::new(cursor.id.0)),
                    }
                }
                binds.push(Box::new(cursor.id.0));
            }
            format!("{DATED} DESC, a.id DESC")
        }
    }
}

/// The shared half of [`list_articles_query`] and
/// [`count_articles_query`]: picks the query shape (see the former) and
/// builds the keyset and filter clauses. `listed` says the select list
/// needs the state flags, so `article_state` is joined even with no state
/// filter.
fn filtered_articles(params: &ListArticles, listed: bool) -> Filtered {
    let filter = params.filter();
    let state_filters = filter.state_flags();
    let membership = MEMBERSHIP_INDEXES.iter().find(|(column, _)| {
        state_filters
            .iter()
            .any(|(filtered, want)| filtered == column && *want == Some(true))
    });
    let needs_state = listed || state_filters.iter().any(|(_, want)| want.is_some());
    let (from, id) = match membership {
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
    let mut clauses = Vec::new();
    let mut binds: Vec<Box<dyn ToSql>> = Vec::new();
    let order_by = keyset(
        params.order,
        params.after.as_ref(),
        id,
        &mut clauses,
        &mut binds,
    );
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
    let filter = if clauses.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", clauses.join(" AND "))
    };
    Filtered {
        from,
        order_by,
        filter,
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
    read_permille: u16,
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
            progress: ReadProgress::START,
        },
        read_permille: row.get(16)?,
        tags: row.get(17)?,
    })
}

impl RawListed {
    pub(super) fn into_listed(mut self) -> Result<ListedArticle, StorageError> {
        self.state.progress = decode_progress(self.read_permille)?;
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
mod tests;
