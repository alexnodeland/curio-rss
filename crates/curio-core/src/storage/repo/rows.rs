//! Shared helpers: tag normalization, dedupe-key scoping, the article
//! list/search query compilers, and the raw-row -> domain-type parsers.

use super::{ARTICLE_COLS, FEED_COLS, ListArticles};
use crate::model::{
    Article, ArticleContent, ArticleId, ArticleState, Feed, FeedId, FeedStatus, FetchErrorKind,
    ListedArticle, SNIPPET_SOURCE_CHARS, snippet,
};
use crate::storage::StorageError;
use curio_types::{CurioId, EventEnvelope, Timestamp};
use rusqlite::types::ToSql;
use rusqlite::{Connection, OptionalExtension as _, Row};

/// Namespaces an ingest-computed dedupe key by its provenance:
/// `f<feed_id>:` for feed articles, `m:` for manual saves — so two feeds
/// carrying the same guid (or the same hash-tier key) can never match
/// each other's rows, while the global UNIQUE constraint stays as
/// defense in depth. Migration 0002 applied the same transform to
/// pre-existing rows.
///
/// Provenance is the feed's row id, not its URL, so permanent-redirect
/// URL adoption never churns keys. Rows orphaned by a feed removal keep
/// their historical prefix (feed row ids are effectively never reused at
/// personal scale, and a re-subscription is a new subscription).
pub(super) fn scoped_dedupe_key(feed_id: Option<FeedId>, key: &str) -> String {
    match feed_id {
        Some(feed) => format!("f{}:{key}", feed.0),
        None => format!("m:{key}"),
    }
}

/// Trims a tag and refuses the empty result — the producer-side
/// enforcement of the published schemas' `minLength: 1` on tags.
pub(super) fn validated_tag(tag: &str) -> Result<String, StorageError> {
    let tag = tag.trim();
    if tag.is_empty() {
        return Err(StorageError::InvalidTag);
    }
    Ok(tag.to_owned())
}

/// Normalizes a tag list: trim, drop empties, dedupe in first-seen order
/// (`uniqueItems` in the published schemas).
pub(super) fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim();
        if !tag.is_empty() && !out.iter().any(|t| t == tag) {
            out.push(tag.to_owned());
        }
    }
    out
}

/// Compiles user input into a safe FTS5 query: every whitespace-separated
/// token becomes a quoted phrase (embedded quotes doubled), joined by the
/// implicit AND. `None` for effectively-empty input.
pub(super) fn fts_match_expr(input: &str) -> Option<String> {
    let tokens: Vec<String> = input
        .split_whitespace()
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect();
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}

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
    let state_filters = [
        ("is_read", params.read),
        ("is_starred", params.starred),
        ("is_read_later", params.read_later),
        ("is_archived", params.archived),
    ];
    let membership = MEMBERSHIP_INDEXES.iter().find(|(column, _)| {
        state_filters
            .iter()
            .any(|(filtered, want)| filtered == column && *want == Some(true))
    });
    let cols = match projection {
        Projection::Full => article_cols_prefixed("a"),
        Projection::Listed => listed_cols(),
    };
    let needs_state =
        projection == Projection::Listed || state_filters.iter().any(|(_, want)| want.is_some());
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
    let mut sql = format!("SELECT {cols} FROM {from}");
    let mut clauses = vec![format!("{key} < ?")];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(params.before.map_or(i64::MAX, |id| id.0))];
    if let Some((column, _)) = membership {
        // A literal, not a bind: a partial index only serves a query whose
        // WHERE visibly implies the index's own `= 1`.
        clauses.push(format!("s.{column} = 1"));
    }
    if let Some(feed) = params.feed_id {
        clauses.push("a.feed_id = ?".to_owned());
        binds.push(Box::new(feed.0));
    }
    for (column, want) in state_filters {
        if membership.is_some_and(|(driving, _)| *driving == column) {
            continue;
        }
        if let Some(want) = want {
            clauses.push(format!("COALESCE(s.{column}, 0) = ?"));
            binds.push(Box::new(want));
        }
    }
    if let Some(tag) = &params.tag {
        clauses.push(
            "EXISTS (SELECT 1 FROM article_tags at JOIN tags t ON t.id = at.tag_id \
             WHERE at.article_id = a.id AND t.name = ?)"
                .to_owned(),
        );
        binds.push(Box::new(tag.trim().to_owned()));
    }
    if let Some(feed_tag) = &params.feed_tag {
        // Folder scope: the feed carries a path tag equal to `feed_tag` or
        // nested beneath it (`Tech` → `Tech`, `Tech/Databases`, …).
        let feed_tag = feed_tag.trim().to_owned();
        clauses.push(
            "a.feed_id IN (SELECT f.id FROM feeds f, json_each(f.tags) je \
             WHERE je.value = ? OR je.value LIKE ? || '/%')"
                .to_owned(),
        );
        binds.push(Box::new(feed_tag.clone()));
        binds.push(Box::new(feed_tag));
    }
    sql.push_str(" WHERE ");
    sql.push_str(&clauses.join(" AND "));
    sql.push_str(" ORDER BY ");
    sql.push_str(key);
    sql.push_str(" DESC LIMIT ?");
    binds.push(Box::new(params.limit));
    (sql, binds)
}

/// The per-feed unread counts: one `feed_unread` row per feed (key 0 =
/// no feed), kept by migration 0012's triggers — never a scan of the
/// articles.
pub(super) const UNREAD_COUNTS: &str = "SELECT feed_key, unread FROM feed_unread WHERE unread > 0";

/// The full-text search over [`Projection::Listed`] rows, best match
/// first; the binds are the compiled `MATCH` expression and the limit.
pub(super) fn search_listed_query() -> String {
    format!(
        "SELECT {} FROM articles_fts \
         JOIN articles a ON a.id = articles_fts.rowid \
         LEFT JOIN article_state s ON s.article_id = a.id \
         WHERE articles_fts MATCH ?1 ORDER BY rank LIMIT ?2",
        listed_cols()
    )
}

pub(super) fn article_cols_prefixed(alias: &str) -> String {
    ARTICLE_COLS
        .split(',')
        .map(|col| format!("{alias}.{}", col.trim()))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Asserts the article exists and returns its `curio_id`.
pub(super) fn require_article(conn: &Connection, article_id: i64) -> Result<CurioId, StorageError> {
    let raw: String = conn
        .prepare_cached("SELECT curio_id FROM articles WHERE id = ?1")?
        .query_row([article_id], |row| row.get(0))
        .optional()?
        .ok_or(StorageError::NotFound { entity: "article" })?;
    parse_curio_id(&raw)
}

/// Upserts one boolean flag in `article_state`; `Ok(true)` iff the stored
/// value actually changed. `column` is a compile-time constant — never
/// user input.
pub(super) fn set_flag(
    conn: &Connection,
    article_id: i64,
    column: &'static str,
    value: bool,
) -> Result<bool, StorageError> {
    let current: Option<bool> = conn
        .prepare_cached(&format!(
            "SELECT {column} FROM article_state WHERE article_id = ?1"
        ))?
        .query_row([article_id], |row| row.get(0))
        .optional()?;
    if current.unwrap_or(false) == value {
        return Ok(false);
    }
    conn.prepare_cached(&format!(
        "INSERT INTO article_state (article_id, {column}, updated_at) VALUES (?1, ?2, ?3) \
         ON CONFLICT(article_id) DO UPDATE SET {column} = excluded.{column}, \
         updated_at = excluded.updated_at"
    ))?
    .execute((article_id, value, Timestamp::now().to_string()))?;
    Ok(true)
}

/// The article's tags inside an open transaction (tags-in-payload rule).
pub(super) fn article_tags_tx(
    conn: &Connection,
    article_id: i64,
) -> Result<Vec<String>, StorageError> {
    let mut stmt = conn.prepare_cached(
        "SELECT t.name FROM tags t JOIN article_tags at ON at.tag_id = t.id \
         WHERE at.article_id = ?1 ORDER BY t.name",
    )?;
    let tags = stmt
        .query_map([article_id], |row| row.get(0))?
        .collect::<Result<Vec<String>, _>>()?;
    Ok(tags)
}

/// Stages an envelope as a write-ahead intent row (same transaction as
/// the state change it narrates).
pub(super) fn insert_intent(
    conn: &Connection,
    envelope: &EventEnvelope,
) -> Result<(), StorageError> {
    let json = serde_json::to_string(envelope)?;
    conn.prepare_cached(
        "INSERT INTO event_intents (event_id, ts, envelope_json) VALUES (?1, ?2, ?3)",
    )?
    .execute((envelope.event_id.to_string(), envelope.ts.to_string(), json))?;
    Ok(())
}

pub(super) fn feed_by_id(conn: &Connection, id: i64) -> Result<Option<Feed>, StorageError> {
    conn.prepare_cached(&format!("SELECT {FEED_COLS} FROM feeds WHERE id = ?1"))?
        .query_row([id], raw_feed)
        .optional()?
        .map(RawFeed::into_feed)
        .transpose()
}

pub(super) fn article_by_id(conn: &Connection, id: i64) -> Result<Option<Article>, StorageError> {
    conn.prepare_cached(&format!(
        "SELECT {ARTICLE_COLS} FROM articles WHERE id = ?1"
    ))?
    .query_row([id], raw_article)
    .optional()?
    .map(RawArticle::into_article)
    .transpose()
}

pub(super) fn parse_ts(column: &'static str, s: &str) -> Result<Timestamp, StorageError> {
    s.parse().map_err(|err| StorageError::Corrupt {
        column,
        message: format!("{err}"),
    })
}

pub(super) fn parse_opt_ts(
    column: &'static str,
    s: Option<String>,
) -> Result<Option<Timestamp>, StorageError> {
    s.map(|s| parse_ts(column, &s)).transpose()
}

/// Reads a stored `fetch_log.error_kind`. Deliberately lenient: an unknown
/// value reads as `None` rather than [`StorageError::Corrupt`] — kinds are
/// added without a schema migration, so a profile a newer build touched
/// can carry one this build does not know, and that must never break
/// listing feeds. The human `error` text still says what happened.
pub(super) fn parse_fetch_error_kind(s: Option<&str>) -> Option<FetchErrorKind> {
    s.and_then(FetchErrorKind::parse)
}

pub(super) fn parse_curio_id(s: &str) -> Result<CurioId, StorageError> {
    s.parse().map_err(|err| StorageError::Corrupt {
        column: "articles.curio_id",
        message: format!("{err}"),
    })
}

/// Raw row image of `feeds` — primitives out of rusqlite, parsed into the
/// domain type in a second step so parse failures surface as
/// [`StorageError::Corrupt`], not panics.
pub(super) struct RawFeed {
    id: i64,
    url: String,
    title: Option<String>,
    site_url: Option<String>,
    description: Option<String>,
    etag: Option<String>,
    last_modified: Option<String>,
    status: String,
    allow_private_network: bool,
    added_at: String,
    last_fetched_at: Option<String>,
    tags: String,
    fetch_full_text: bool,
    last_error: Option<String>,
    last_ok_at: Option<String>,
    last_error_kind: Option<String>,
    consecutive_failures: u32,
    next_attempt_at: Option<String>,
    refresh_interval_min: Option<u32>,
    failing_since: Option<String>,
    auto_paused_at: Option<String>,
    refresh_hint_secs: Option<u32>,
}

pub(super) fn raw_feed(row: &Row<'_>) -> rusqlite::Result<RawFeed> {
    Ok(RawFeed {
        id: row.get(0)?,
        url: row.get(1)?,
        title: row.get(2)?,
        site_url: row.get(3)?,
        description: row.get(4)?,
        etag: row.get(5)?,
        last_modified: row.get(6)?,
        status: row.get(7)?,
        allow_private_network: row.get(8)?,
        added_at: row.get(9)?,
        last_fetched_at: row.get(10)?,
        tags: row.get(11)?,
        fetch_full_text: row.get(12)?,
        last_error: row.get(13)?,
        last_ok_at: row.get(14)?,
        last_error_kind: row.get(15)?,
        consecutive_failures: row.get(16)?,
        next_attempt_at: row.get(17)?,
        refresh_interval_min: row.get(18)?,
        failing_since: row.get(19)?,
        auto_paused_at: row.get(20)?,
        refresh_hint_secs: row.get(21)?,
    })
}

impl RawFeed {
    pub(super) fn into_feed(self) -> Result<Feed, StorageError> {
        Ok(Feed {
            id: FeedId(self.id),
            url: self.url,
            title: self.title,
            site_url: self.site_url,
            description: self.description,
            etag: self.etag,
            last_modified: self.last_modified,
            status: FeedStatus::parse(&self.status).ok_or_else(|| StorageError::Corrupt {
                column: "feeds.status",
                message: format!("unknown status {:?}", self.status),
            })?,
            allow_private_network: self.allow_private_network,
            added_at: parse_ts("feeds.added_at", &self.added_at)?,
            last_fetched_at: parse_opt_ts("feeds.last_fetched_at", self.last_fetched_at)?,
            last_error: self.last_error,
            last_error_kind: parse_fetch_error_kind(self.last_error_kind.as_deref()),
            last_ok_at: parse_opt_ts("fetch_log.fetched_at", self.last_ok_at)?,
            tags: serde_json::from_str(&self.tags).map_err(|err| StorageError::Corrupt {
                column: "feeds.tags",
                message: err.to_string(),
            })?,
            fetch_full_text: self.fetch_full_text,
            consecutive_failures: self.consecutive_failures,
            next_attempt_at: parse_opt_ts("feeds.next_attempt_at", self.next_attempt_at)?,
            refresh_interval_min: self.refresh_interval_min,
            failing_since: parse_opt_ts("feeds.failing_since", self.failing_since)?,
            auto_paused_at: parse_opt_ts("feeds.auto_paused_at", self.auto_paused_at)?,
            refresh_hint_secs: self.refresh_hint_secs,
        })
    }
}

/// Raw row image of `articles` (see [`RawFeed`]).
pub(super) struct RawArticle {
    id: i64,
    curio_id: String,
    feed_id: Option<i64>,
    dedupe_key: String,
    title: String,
    source_url: String,
    author: Option<String>,
    published_at: Option<String>,
    content_html: String,
    content_text: String,
    lang: Option<String>,
    word_count: Option<u32>,
    saved_at: String,
    source_updated_at: Option<String>,
    lead_image: Option<String>,
}

pub(super) fn raw_article(row: &Row<'_>) -> rusqlite::Result<RawArticle> {
    Ok(RawArticle {
        id: row.get(0)?,
        curio_id: row.get(1)?,
        feed_id: row.get(2)?,
        dedupe_key: row.get(3)?,
        title: row.get(4)?,
        source_url: row.get(5)?,
        author: row.get(6)?,
        published_at: row.get(7)?,
        content_html: row.get(8)?,
        content_text: row.get(9)?,
        lang: row.get(10)?,
        word_count: row.get(11)?,
        saved_at: row.get(12)?,
        source_updated_at: row.get(13)?,
        lead_image: row.get(14)?,
    })
}

impl RawArticle {
    pub(super) fn into_article(self) -> Result<Article, StorageError> {
        Ok(Article {
            id: ArticleId(self.id),
            curio_id: parse_curio_id(&self.curio_id)?,
            feed_id: self.feed_id.map(FeedId),
            dedupe_key: self.dedupe_key,
            title: self.title,
            source_url: self.source_url,
            author: self.author,
            published_at: parse_opt_ts("articles.published_at", self.published_at)?,
            content: ArticleContent {
                html: self.content_html,
                text: self.content_text,
            },
            lang: self.lang,
            word_count: self.word_count,
            saved_at: parse_ts("articles.saved_at", &self.saved_at)?,
            source_updated_at: parse_opt_ts("articles.source_updated_at", self.source_updated_at)?,
            lead_image: self.lead_image,
        })
    }
}

/// Raw row image of a [`Projection::Listed`] row (see [`RawFeed`]).
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

    #[test]
    fn fts_match_expr_quotes_every_token() {
        assert_eq!(
            fts_match_expr("rust sqlite"),
            Some("\"rust\" \"sqlite\"".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_escapes_embedded_quotes() {
        assert_eq!(
            fts_match_expr("say \"hi\""),
            Some("\"say\" \"\"\"hi\"\"\"".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_rejects_empty_input() {
        assert_eq!(fts_match_expr("   "), None);
        assert_eq!(fts_match_expr(""), None);
    }

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
    fn the_sparsest_required_membership_drives() {
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
        let search = search_listed_query();
        assert!(!search.contains("content_html"), "{search}");
        assert!(search.contains("LEFT JOIN article_state s ON s.article_id = a.id"));
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
            sql.contains("je.value = ? OR je.value LIKE ? || '/%'"),
            "matches the folder itself and its subtree: {sql}"
        );
        assert!(
            !sql.contains("article_state"),
            "feed_tag alone needs no state join"
        );
        assert_eq!(
            binds.len(),
            4,
            "before + feed_tag (exact + subtree) + limit"
        );
    }

    /// Property suite for the FTS query builder (issue #133): no user input
    /// compiles to a query FTS5 rejects, FTS5 operator syntax in the input
    /// is matched literally, and every plain word finds the row holding it.
    mod fts_match_expr_props {
        #![allow(clippy::unwrap_used)]

        use proptest::prelude::*;

        use super::super::fts_match_expr;

        /// An FTS5 table shaped like `articles_fts` (same columns, default
        /// tokenizer), holding `rows`.
        fn index(rows: &[&str]) -> rusqlite::Connection {
            let conn = rusqlite::Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE VIRTUAL TABLE t USING fts5(title, content_text);")
                .unwrap();
            for row in rows {
                conn.execute("INSERT INTO t(title, content_text) VALUES ('', ?1)", [row])
                    .unwrap();
            }
            conn
        }

        fn hits(conn: &rusqlite::Connection, expr: &str) -> rusqlite::Result<i64> {
            conn.query_row("SELECT count(*) FROM t WHERE t MATCH ?1", [expr], |row| {
                row.get(0)
            })
        }

        /// FTS5 query-language fragments a raw `MATCH` would interpret.
        const OPERATORS: &[&str] = &[
            "AND",
            "OR",
            "NOT",
            "NEAR",
            "NEAR(",
            "*",
            "^",
            "-",
            "+",
            ":",
            "(",
            ")",
            "\"",
            "\"\"",
            "title:",
            "{title content_text}:",
            "a*",
            "NEAR(a b, 2)",
            "'",
            ";",
            "--",
        ];

        proptest! {
            #[test]
            fn every_compiled_query_is_valid_fts5(input in "\\PC{0,120}") {
                let conn = index(&["rust sqlite query", "日本語 テキスト", "say \"hi\""]);
                match fts_match_expr(&input) {
                    None => prop_assert!(input.split_whitespace().next().is_none()),
                    Some(expr) => {
                        let result = hits(&conn, &expr);
                        prop_assert!(result.is_ok(), "{input:?} → {expr:?}: {result:?}");
                    }
                }
            }

            #[test]
            fn operator_syntax_is_matched_literally(
                words in prop::collection::vec(prop::sample::select(OPERATORS), 1..6),
            ) {
                // Raw, most of these are FTS5 syntax errors (`NEAR(`, a
                // lone `"`, `title:` with no term) or operators. Compiled,
                // each is a quoted phrase: always a valid query.
                let conn = index(&["a apple and or not near", "title content"]);
                let expr = fts_match_expr(&words.join(" ")).unwrap();
                let result = hits(&conn, &expr);
                prop_assert!(result.is_ok(), "{words:?} → {expr:?}: {result:?}");
            }

            #[test]
            fn every_plain_word_finds_its_row(
                words in prop::collection::vec("[a-z]{2,10}", 1..5),
            ) {
                let row = words.join(" ");
                let conn = index(&[&row, "unrelated filler"]);
                let expr = fts_match_expr(&format!("  {}\t", words.join("  "))).unwrap();
                prop_assert_eq!(expr.matches('"').count(), words.len() * 2);
                prop_assert_eq!(hits(&conn, &expr).unwrap(), 1);
            }
        }
    }
}
