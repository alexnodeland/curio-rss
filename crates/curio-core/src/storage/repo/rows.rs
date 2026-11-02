//! Shared helpers: tag normalization, dedupe-key scoping, the article
//! list/search query compilers, and the raw-row -> domain-type parsers.

use super::{ARTICLE_COLS, ArticleFilter, FEED_COLS, ListArticles, SearchArticles};
use crate::model::{Article, ArticleContent, ArticleId, Feed, FeedId, FeedStatus, FetchErrorKind};
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

/// One searchable unit of a parsed query: a bare word or a quoted phrase.
#[derive(Debug, PartialEq, Eq)]
struct Atom {
    /// The literal text (quotes stripped; escaped only when rendered).
    text: String,
    /// Matched as a prefix (`"text"*`).
    prefix: bool,
    /// A `-` exclusion.
    negated: bool,
    /// Came from `"…"` — never gets the implicit last-word prefix.
    quoted: bool,
}

impl Atom {
    /// The atom as one FTS5 string (embedded quotes doubled) — always a
    /// valid FTS5 phrase, whatever the text holds.
    fn render(&self) -> String {
        let star = if self.prefix { "*" } else { "" };
        format!("\"{}\"{star}", self.text.replace('"', "\"\""))
    }
}

/// A lexed query item.
#[derive(Debug, PartialEq, Eq)]
enum Item {
    Atom(Atom),
    /// The upper-case `OR` operator.
    Or,
}

/// The implicit prefix on the last word needs this many characters: a
/// one-letter prefix expands to a large share of the vocabulary for
/// almost no narrowing.
const IMPLICIT_PREFIX_MIN_CHARS: usize = 2;

/// Splits user input into atoms and `OR`s (see [`super::SearchArticles`]
/// for the syntax). Atoms without a single letter or digit are dropped —
/// they hold no token the index could match, and an empty phrase would
/// empty an AND of everything else.
fn lex_query(input: &str) -> Vec<Item> {
    let mut items = Vec::new();
    let mut chars = input.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&first) = chars.peek() else {
            break;
        };
        let negated = first == '-';
        if negated {
            chars.next();
        }
        let quoted = chars.next_if_eq(&'"').is_some();
        let mut text = String::new();
        let mut prefix = false;
        if quoted {
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                text.push(c);
            }
            while chars.next_if_eq(&'*').is_some() {
                prefix = true;
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                text.push(c);
            }
            let body = text.trim_end_matches('*').len();
            prefix = body < text.len();
            text.truncate(body);
            if !negated && text == "OR" && !prefix {
                items.push(Item::Or);
                continue;
            }
        }
        if text.chars().any(char::is_alphanumeric) {
            items.push(Item::Atom(Atom {
                text,
                prefix,
                negated,
                quoted,
            }));
        }
    }
    // Search-as-you-type: the word still being typed matches as a prefix.
    if let Some(Item::Atom(last)) = items.last_mut()
        && !last.negated
        && !last.quoted
        && last.text.chars().count() >= IMPLICIT_PREFIX_MIN_CHARS
    {
        last.prefix = true;
    }
    items
}

/// Compiles user input into a safe FTS5 query (syntax on
/// [`super::SearchArticles`]): positive atoms become an AND of OR-groups,
/// exclusions one `NOT (… OR …)` — a single flat OR, so any number of
/// exclusions stays inside FTS5's expression-depth limit. Every atom is a
/// quoted FTS5 string, so input text never acts as FTS5 syntax. `None`
/// when nothing positive is left to match.
pub(super) fn fts_match_expr(input: &str) -> Option<String> {
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let mut or_pending = false;
    for item in lex_query(input) {
        match item {
            Item::Or => or_pending = !groups.is_empty(),
            Item::Atom(atom) if atom.negated => {
                excluded.push(atom.render());
                or_pending = false;
            }
            Item::Atom(atom) => {
                match groups.last_mut() {
                    Some(group) if or_pending => group.push(atom.render()),
                    _ => groups.push(vec![atom.render()]),
                }
                or_pending = false;
            }
        }
    }
    if groups.is_empty() {
        return None;
    }
    let positive = groups
        .into_iter()
        .map(|group| {
            if group.len() == 1 {
                group.join("")
            } else {
                format!("({})", group.join(" OR "))
            }
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    if excluded.is_empty() {
        Some(positive)
    } else {
        Some(format!("({positive}) NOT ({})", excluded.join(" OR ")))
    }
}

/// BM25 column weights for `articles_fts(title, content_text)`: a title
/// hit outranks a body hit.
const SEARCH_RANK: &str = "bm25(articles_fts, 10.0, 1.0)";

/// Appends the [`ArticleFilter`] clauses (static fragments, every value a
/// bind) over the `a` (articles) alias. Returns whether a clause reads the
/// `s` (`article_state`) alias, so the caller adds that join only then; a
/// missing state row defaults every flag to 0 via `COALESCE`.
fn push_filter_clauses(
    filter: &ArticleFilter,
    clauses: &mut Vec<String>,
    binds: &mut Vec<Box<dyn ToSql>>,
) -> bool {
    if let Some(feed) = filter.feed_id {
        clauses.push("a.feed_id = ?".to_owned());
        binds.push(Box::new(feed.0));
    }
    let state_filters = [
        ("is_read", filter.read),
        ("is_starred", filter.starred),
        ("is_read_later", filter.read_later),
        ("is_archived", filter.archived),
    ];
    let mut needs_state = false;
    for (column, want) in state_filters {
        if let Some(want) = want {
            clauses.push(format!("COALESCE(s.{column}, 0) = ?"));
            binds.push(Box::new(want));
            needs_state = true;
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
    needs_state
}

/// Compiles [`ListArticles`] into SQL + positional binds. Static clause
/// fragments only — no user input ever lands in the SQL string (the tag
/// value is a bind). The `article_state` join appears only when a state
/// filter asks for it, so the unfiltered hot path stays join-free.
pub(super) fn list_articles_query(params: &ListArticles) -> (String, Vec<Box<dyn ToSql>>) {
    let cols = article_cols_prefixed("a");
    let mut clauses = vec!["a.id < ?".to_owned()];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(params.before.map_or(i64::MAX, |id| id.0))];
    let needs_state = push_filter_clauses(&params.filter(), &mut clauses, &mut binds);
    let join = if needs_state { STATE_JOIN } else { "" };
    let sql = format!(
        "SELECT {cols} FROM articles a{join} WHERE {} ORDER BY a.id DESC LIMIT ?",
        clauses.join(" AND ")
    );
    binds.push(Box::new(params.limit));
    (sql, binds)
}

/// The optional `article_state` join both compilers share.
const STATE_JOIN: &str = " LEFT JOIN article_state s ON s.article_id = a.id";

/// Compiles [`SearchArticles`] into SQL + positional binds — the FTS
/// expression is the first bind, then the [`ArticleFilter`] clauses
/// (exactly as [`list_articles_query`] applies them), ordered by the
/// title-weighted BM25 rank with `id DESC` breaking ties. `None` when the
/// query has nothing to match.
pub(super) fn search_articles_query(
    params: &SearchArticles,
) -> Option<(String, Vec<Box<dyn ToSql>>)> {
    let expr = fts_match_expr(&params.query)?;
    let cols = article_cols_prefixed("a");
    let mut clauses = vec!["articles_fts MATCH ?".to_owned()];
    let mut binds: Vec<Box<dyn ToSql>> = vec![Box::new(expr)];
    let needs_state = push_filter_clauses(&params.filter, &mut clauses, &mut binds);
    let join = if needs_state { STATE_JOIN } else { "" };
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

#[cfg(test)]
mod tests {
    // Test code: a panicking unwrap is the failure report.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn fts_match_expr_quotes_every_token_and_prefixes_the_last() {
        assert_eq!(
            fts_match_expr("rust sqlite"),
            Some("\"rust\" AND \"sqlite\"*".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_keeps_quoted_phrases_whole() {
        assert_eq!(
            fts_match_expr("say \"hello world\""),
            Some("\"say\" AND \"hello world\"".to_owned()),
            "a phrase is one atom and never gets the implicit prefix"
        );
        assert_eq!(
            fts_match_expr("\"unbalanced phrase"),
            Some("\"unbalanced phrase\"".to_owned()),
            "an unterminated quote runs to the end"
        );
        assert_eq!(
            fts_match_expr("\"hello wor\"*"),
            Some("\"hello wor\"*".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_escapes_embedded_quotes() {
        assert_eq!(
            fts_match_expr("it\"s ok"),
            Some("\"it\"\"s\" AND \"ok\"*".to_owned()),
            "a quote inside a bare word is doubled, not a phrase start"
        );
    }

    #[test]
    fn fts_match_expr_explicit_and_implicit_prefixes() {
        assert_eq!(
            fts_match_expr("sql* tutorial"),
            Some("\"sql\"* AND \"tutorial\"*".to_owned())
        );
        assert_eq!(
            fts_match_expr("rust a"),
            Some("\"rust\" AND \"a\"".to_owned()),
            "a one-letter last word is not widened to a prefix"
        );
        assert_eq!(fts_match_expr("a*"), Some("\"a\"*".to_owned()));
    }

    #[test]
    fn fts_match_expr_compiles_exclusions_into_one_flat_not() {
        assert_eq!(
            fts_match_expr("rust -async -\"tokio runtime\""),
            Some("(\"rust\") NOT (\"async\" OR \"tokio runtime\")".to_owned()),
            "an exclusion is never prefixed, nor is a word before it"
        );
        assert_eq!(
            fts_match_expr("-only -exclusions"),
            None,
            "nothing positive to match"
        );
    }

    #[test]
    fn fts_match_expr_groups_or_tighter_than_and() {
        assert_eq!(
            fts_match_expr("rust OR go tutorial"),
            Some("(\"rust\" OR \"go\") AND \"tutorial\"*".to_owned())
        );
        assert_eq!(
            fts_match_expr("OR rust OR"),
            Some("\"rust\"".to_owned()),
            "a dangling OR is dropped (and the last item was the OR)"
        );
        assert_eq!(
            fts_match_expr("rust or go"),
            Some("\"rust\" AND \"or\" AND \"go\"*".to_owned()),
            "lower-case or is a plain word"
        );
        assert_eq!(
            fts_match_expr("\"OR\""),
            Some("\"OR\"".to_owned()),
            "a quoted OR is the word"
        );
    }

    #[test]
    fn fts_match_expr_drops_atoms_without_a_letter_or_digit() {
        assert_eq!(
            fts_match_expr("rust - * \"\" ( )"),
            Some("\"rust\"*".to_owned())
        );
    }

    #[test]
    fn fts_match_expr_rejects_empty_input() {
        assert_eq!(fts_match_expr("   "), None);
        assert_eq!(fts_match_expr(""), None);
        assert_eq!(fts_match_expr("- * -- OR"), None);
    }

    #[test]
    fn search_query_ranks_title_first_and_reuses_the_list_filters() {
        let (sql, binds) = search_articles_query(&SearchArticles {
            query: "rust".to_owned(),
            filter: ArticleFilter {
                feed_id: Some(FeedId(3)),
                read: Some(false),
                feed_tag: Some("Tech".to_owned()),
                ..ArticleFilter::default()
            },
            limit: 20,
        })
        .unwrap();
        assert!(sql.contains("articles_fts MATCH ?"));
        assert!(sql.contains("a.feed_id = ?"));
        assert!(sql.contains("LEFT JOIN article_state s"));
        assert!(sql.contains("COALESCE(s.is_read, 0) = ?"));
        assert!(
            sql.ends_with("ORDER BY bm25(articles_fts, 10.0, 1.0), a.id DESC LIMIT ?"),
            "{sql}"
        );
        assert_eq!(binds.len(), 7, "match + feed + read + feed_tag ×3 + limit");
    }

    #[test]
    fn unscoped_search_query_stays_join_free() {
        let (sql, binds) = search_articles_query(&SearchArticles::new("rust", 5)).unwrap();
        assert!(!sql.contains("article_state"), "{sql}");
        assert_eq!(binds.len(), 2, "match + limit");
        assert!(search_articles_query(&SearchArticles::new(" - ", 5)).is_none());
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
        let (sql, binds) = list_articles_query(&ListArticles::default());
        assert!(!sql.contains("JOIN"), "no filters → no joins: {sql}");
        assert!(sql.ends_with("ORDER BY a.id DESC LIMIT ?"));
        assert_eq!(binds.len(), 2, "before + limit");
    }

    #[test]
    fn state_filters_join_and_coalesce_missing_rows() {
        let (sql, binds) = list_articles_query(&ListArticles {
            feed_id: Some(FeedId(7)),
            read: Some(false),
            archived: Some(true),
            ..ListArticles::default()
        });
        assert!(sql.contains("LEFT JOIN article_state s ON s.article_id = a.id"));
        assert!(sql.contains("COALESCE(s.is_read, 0) = ?"));
        assert!(sql.contains("COALESCE(s.is_archived, 0) = ?"));
        assert!(sql.contains("a.feed_id = ?"));
        assert!(!sql.contains("is_starred"), "unset filters stay out");
        assert!(sql.ends_with("ORDER BY a.id DESC LIMIT ?"), "order fixed");
        assert_eq!(binds.len(), 5, "before + feed + 2 flags + limit");
    }

    #[test]
    fn tag_filter_binds_the_tag_instead_of_splicing_it() {
        let (sql, binds) = list_articles_query(&ListArticles {
            tag: Some("rust'; DROP TABLE articles; --".to_owned()),
            ..ListArticles::default()
        });
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
        let (sql, binds) = list_articles_query(&ListArticles {
            feed_tag: Some("Tech".to_owned()),
            ..ListArticles::default()
        });
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

    /// Property suite for the FTS query builder (issues #133, #138): no
    /// user input compiles to a query FTS5 rejects, FTS5 keywords in the
    /// input are matched literally, and every plain word finds the row
    /// holding it.
    mod fts_match_expr_props {
        #![allow(clippy::unwrap_used)]

        use proptest::prelude::*;

        use super::super::{Item, fts_match_expr, lex_query};

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
                    // `None` only when nothing positive survived lexing.
                    None => prop_assert!(
                        !lex_query(&input)
                            .iter()
                            .any(|item| matches!(item, Item::Atom(atom) if !atom.negated)),
                        "{input:?}"
                    ),
                    Some(expr) => {
                        let result = hits(&conn, &expr);
                        prop_assert!(result.is_ok(), "{input:?} → {expr:?}: {result:?}");
                    }
                }
            }

            #[test]
            fn operator_syntax_never_breaks_the_query(
                words in prop::collection::vec(prop::sample::select(OPERATORS), 1..6),
            ) {
                // Raw, most of these are FTS5 syntax errors (`NEAR(`, a
                // lone `"`, `title:` with no term) or operators. Compiled,
                // a fragment is a quoted atom, Curio's own `OR`/`-`/`*`, or
                // dropped (no letter or digit): always a valid query.
                let conn = index(&["a apple and or not near", "title content"]);
                if let Some(expr) = fts_match_expr(&words.join(" ")) {
                    let result = hits(&conn, &expr);
                    prop_assert!(result.is_ok(), "{words:?} → {expr:?}: {result:?}");
                }
            }

            #[test]
            fn fts5_keywords_are_matched_literally(
                words in prop::collection::vec(
                    prop::sample::select(&["AND", "NOT", "NEAR", "title:", "a"][..]),
                    1..6,
                ),
            ) {
                // Every word holds a letter, so each survives as a quoted
                // atom: the compiled query is a pure AND of literals, never
                // an FTS5 operator, and matches the row holding them all.
                let conn = index(&["a and not near title", "unrelated filler"]);
                let expr = fts_match_expr(&words.join(" ")).unwrap();
                prop_assert!(!expr.contains(" OR ") && !expr.contains(" NOT "), "{expr}");
                prop_assert_eq!(hits(&conn, &expr).unwrap(), 1);
            }

            #[test]
            fn any_number_of_terms_stays_within_fts5_limits(
                count in 1_usize..400,
                negated in any::<bool>(),
                ored in any::<bool>(),
            ) {
                // FTS5 caps expression depth at 256: exclusions compile to
                // one flat `NOT (… OR …)`, never a chain of NOTs.
                let conn = index(&["w0 body"]);
                let words: Vec<String> = (0..count)
                    .map(|i| if negated { format!("-x{i}") } else { format!("w{i}") })
                    .collect();
                let joiner = if ored && !negated { " OR " } else { " " };
                let input = format!("w0 {}", words.join(joiner));
                let expr = fts_match_expr(&input).unwrap();
                let result = hits(&conn, &expr);
                prop_assert!(result.is_ok(), "{count} terms: {result:?}");
            }

            #[test]
            fn every_plain_word_finds_its_row(
                words in prop::collection::vec("[a-z]{2,10}", 1..5),
            ) {
                let row = words.join(" ");
                // Digits only: no lower-case prefix can reach the filler.
                let conn = index(&[&row, "12345 67890"]);
                let expr = fts_match_expr(&format!("  {}\t", words.join("  "))).unwrap();
                prop_assert_eq!(expr.matches('"').count(), words.len() * 2);
                prop_assert_eq!(hits(&conn, &expr).unwrap(), 1);
            }
        }
    }
}
