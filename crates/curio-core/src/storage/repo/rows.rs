//! Shared helpers: tag normalization, dedupe-key scoping and the raw-row
//! -> domain-type parsers (the list/search query compilers live in
//! `query`).

use super::{ARTICLE_COLS, FEED_COLS};
use crate::model::{Article, ArticleContent, ArticleId, Feed, FeedId, FeedStatus, FetchErrorKind};
use crate::storage::StorageError;
use curio_types::{CurioId, EventEnvelope, Timestamp};
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
