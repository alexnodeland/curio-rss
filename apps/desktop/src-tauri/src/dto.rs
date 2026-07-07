//! IPC mirror types (DTOs) for curio-core's model.
//!
//! Core model types deliberately carry no serde (keeping specta/serde out
//! of the engine's tree — build-spec decision §0.6), so the head defines
//! mirrors deriving `Serialize`/`Deserialize` + `specta::Type` with `From`
//! impls. Conventions across the boundary:
//!
//! - IDs (`FeedId`, `ArticleId`) cross as `i64` (`SQLite` rowids — safely
//!   inside JS number range; the bindings exporter maps 64-bit ints to
//!   `number`).
//! - `Timestamp` / `Checksum` / `CurioId` / `DestinationName` cross as
//!   their canonical string forms (they all have `Display`/`FromStr`).
//! - List rows use [`ArticleSummaryDto`] (no content) to keep list
//!   payloads small; only [`ArticleDto`] carries the sanitized HTML.

use std::collections::BTreeMap;

use curio_core::export::ExportDisposition;
use curio_core::model::{
    Article, ArticleState, Feed, FeedId, FeedStatus, FetchRecord, FetchStatus, NewFeed,
};
use curio_core::storage::ListArticles;
use curio_core::{OpmlImportOutcome, RefreshOutcome, SaveOutcome};
use curio_types::Destination;
use serde::{Deserialize, Serialize};

/// Mirror of [`FeedStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FeedStatusDto {
    /// Fetched on refresh.
    Active,
    /// Kept but not fetched.
    Paused,
    /// Auto-paused after HTTP 410.
    Dead,
}

impl From<FeedStatus> for FeedStatusDto {
    fn from(status: FeedStatus) -> Self {
        match status {
            FeedStatus::Active => Self::Active,
            FeedStatus::Paused => Self::Paused,
            FeedStatus::Dead => Self::Dead,
        }
    }
}

impl From<FeedStatusDto> for FeedStatus {
    fn from(status: FeedStatusDto) -> Self {
        match status {
            FeedStatusDto::Active => Self::Active,
            FeedStatusDto::Paused => Self::Paused,
            FeedStatusDto::Dead => Self::Dead,
        }
    }
}

/// Mirror of [`FetchStatus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FetchStatusDto {
    /// 2xx with a parseable body.
    Ok,
    /// 304 — validators still fresh.
    NotModified,
    /// Anything else (recorded, not raised).
    Error,
}

impl From<FetchStatus> for FetchStatusDto {
    fn from(status: FetchStatus) -> Self {
        match status {
            FetchStatus::Ok => Self::Ok,
            FetchStatus::NotModified => Self::NotModified,
            FetchStatus::Error => Self::Error,
        }
    }
}

/// Mirror of [`ExportDisposition`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ExportDispositionDto {
    /// First export → `article.saved`.
    Created,
    /// Changed checksum → `article.updated`.
    Updated,
    /// Idempotency hit — no write, no event.
    Unchanged,
}

impl From<ExportDisposition> for ExportDispositionDto {
    fn from(disposition: ExportDisposition) -> Self {
        match disposition {
            ExportDisposition::Created => Self::Created,
            ExportDisposition::Updated => Self::Updated,
            ExportDisposition::Unchanged => Self::Unchanged,
        }
    }
}

/// Mirror of [`Feed`]. Fetch plumbing (etag/last-modified) and the W1
/// SSRF-exemption flag stay backend-side on purpose — the contract says
/// the allowlist is config-edit only, never a UI surface fed by feed data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FeedDto {
    /// Row id.
    pub id: i64,
    /// Subscription URL.
    pub url: String,
    /// Human title (feed-provided or user-edited).
    pub title: Option<String>,
    /// The site the feed belongs to.
    pub site_url: Option<String>,
    /// Feed-provided description.
    pub description: Option<String>,
    /// Lifecycle status.
    pub status: FeedStatusDto,
    /// When the subscription was added (RFC 3339 UTC).
    pub added_at: String,
    /// Last fetch attempt, if any (RFC 3339 UTC).
    pub last_fetched_at: Option<String>,
    /// Feed-level tags (OPML folders land here).
    pub tags: Vec<String>,
}

impl From<Feed> for FeedDto {
    fn from(feed: Feed) -> Self {
        Self {
            id: feed.id.0,
            url: feed.url,
            title: feed.title,
            site_url: feed.site_url,
            description: feed.description,
            status: feed.status.into(),
            added_at: feed.added_at.to_string(),
            last_fetched_at: feed.last_fetched_at.map(|t| t.to_string()),
            tags: feed.tags,
        }
    }
}

/// Args for adding a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NewFeedDto {
    /// Feed URL.
    pub url: String,
    /// Optional user-supplied title.
    pub title: Option<String>,
    /// Initial tags.
    pub tags: Vec<String>,
}

impl From<NewFeedDto> for NewFeed {
    fn from(new: NewFeedDto) -> Self {
        Self {
            url: new.url,
            title: new.title,
            tags: new.tags,
        }
    }
}

/// A list row: everything the article list renders, no content payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ArticleSummaryDto {
    /// Row id (keyset cursor currency).
    pub id: i64,
    /// Owning feed, if any.
    pub feed_id: Option<i64>,
    /// Title.
    pub title: String,
    /// Canonical source URL.
    pub source_url: String,
    /// Author, if the feed said.
    pub author: Option<String>,
    /// Publication time (RFC 3339 UTC).
    pub published_at: Option<String>,
    /// When Curio stored it (RFC 3339 UTC).
    pub saved_at: String,
    /// Extracted-text word count.
    pub word_count: Option<u32>,
    /// BCP-47 language guess.
    pub lang: Option<String>,
    /// Lead image URL (feed-declared or the body's first `<img>`), if any.
    /// An absolute `http(s)` URL the frontend loads through the policed
    /// image cache — never fetched directly.
    pub image: Option<String>,
}

impl From<Article> for ArticleSummaryDto {
    fn from(article: Article) -> Self {
        Self {
            id: article.id.0,
            feed_id: article.feed_id.map(|id| id.0),
            title: article.title,
            source_url: article.source_url,
            author: article.author,
            published_at: article.published_at.map(|t| t.to_string()),
            saved_at: article.saved_at.to_string(),
            word_count: article.word_count,
            lang: article.lang,
            image: article.lead_image,
        }
    }
}

/// The full article, including sanitized content for the reader pane.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ArticleDto {
    /// Row id.
    pub id: i64,
    /// Portable identity (`UUIDv7`, string form).
    pub curio_id: String,
    /// Owning feed, if any.
    pub feed_id: Option<i64>,
    /// Title.
    pub title: String,
    /// Canonical source URL.
    pub source_url: String,
    /// Author, if the feed said.
    pub author: Option<String>,
    /// Publication time (RFC 3339 UTC).
    pub published_at: Option<String>,
    /// Sanitized-at-ingest HTML (ammonia) — still rendered only through
    /// the frontend's single sanitized-render component, belt-and-braces.
    pub content_html: String,
    /// Extracted plain text.
    pub content_text: String,
    /// BCP-47 language guess.
    pub lang: Option<String>,
    /// Extracted-text word count.
    pub word_count: Option<u32>,
    /// When Curio stored it (RFC 3339 UTC).
    pub saved_at: String,
    /// Source-declared update time (RFC 3339 UTC).
    pub source_updated_at: Option<String>,
}

impl From<Article> for ArticleDto {
    fn from(article: Article) -> Self {
        Self {
            id: article.id.0,
            curio_id: article.curio_id.to_string(),
            feed_id: article.feed_id.map(|id| id.0),
            title: article.title,
            source_url: article.source_url,
            author: article.author,
            published_at: article.published_at.map(|t| t.to_string()),
            content_html: article.content.html,
            content_text: article.content.text,
            lang: article.lang,
            word_count: article.word_count,
            saved_at: article.saved_at.to_string(),
            source_updated_at: article.source_updated_at.map(|t| t.to_string()),
        }
    }
}

/// Mirror of [`ArticleState`] — the per-article flag projection.
// Four independent booleans is the shape of the projection (core's
// `ArticleState` is identical) — not a disguised state machine.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ArticleStateDto {
    /// Read/unread.
    pub read: bool,
    /// Starred.
    pub starred: bool,
    /// In the read-later queue.
    pub read_later: bool,
    /// Archived.
    pub archived: bool,
}

impl From<ArticleState> for ArticleStateDto {
    fn from(state: ArticleState) -> Self {
        Self {
            read: state.read,
            starred: state.starred,
            read_later: state.read_later,
            archived: state.archived,
        }
    }
}

/// Mirror of [`ListArticles`] — filters are backend-owned; the head never
/// re-filters or re-sorts client-side. Keyset order is fixed (`id DESC`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ListArticlesDto {
    /// Restrict to one feed; `None` lists across feeds.
    pub feed_id: Option<i64>,
    /// Keyset cursor: rows with id strictly below.
    pub before: Option<i64>,
    /// Page size.
    pub limit: u32,
    /// Keep only (un)read articles.
    pub read: Option<bool>,
    /// Keep only (un)starred articles.
    pub starred: Option<bool>,
    /// Keep only articles (not) in the read-later queue.
    pub read_later: Option<bool>,
    /// Keep only (un)archived articles.
    pub archived: Option<bool>,
    /// Keep only articles carrying this tag.
    pub tag: Option<String>,
}

impl From<ListArticlesDto> for ListArticles {
    fn from(params: ListArticlesDto) -> Self {
        Self {
            feed_id: params.feed_id.map(FeedId),
            before: params.before.map(curio_core::model::ArticleId),
            limit: params.limit,
            read: params.read,
            starred: params.starred,
            read_later: params.read_later,
            archived: params.archived,
            tag: params.tag,
        }
    }
}

/// Mirror of [`RefreshOutcome`]. Fetch/parse failures are outcomes
/// (`status: "error"` + `error`), never command errors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RefreshOutcomeDto {
    /// The refreshed feed.
    pub feed_id: i64,
    /// Outcome class.
    pub status: FetchStatusDto,
    /// HTTP status, if a response arrived.
    pub http_status: Option<u16>,
    /// Newly stored articles.
    pub new_articles: u64,
    /// Updated existing articles.
    pub updated_articles: u64,
    /// Error description for `status: "error"`.
    pub error: Option<String>,
}

impl From<RefreshOutcome> for RefreshOutcomeDto {
    fn from(outcome: RefreshOutcome) -> Self {
        Self {
            feed_id: outcome.feed_id.0,
            status: outcome.status.into(),
            http_status: outcome.http_status,
            new_articles: outcome.new_articles,
            updated_articles: outcome.updated_articles,
            error: outcome.error,
        }
    }
}

/// Mirror of [`FetchRecord`] — the feed-health surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct FetchRecordDto {
    /// The fetched feed.
    pub feed_id: i64,
    /// When the attempt happened (RFC 3339 UTC).
    pub fetched_at: String,
    /// Outcome class.
    pub status: FetchStatusDto,
    /// HTTP status, if a response arrived.
    pub http_status: Option<u16>,
    /// Error description for `status: "error"`.
    pub error: Option<String>,
    /// New articles stored by this fetch.
    pub articles_new: u32,
    /// Wall-clock duration, if measured.
    pub duration_ms: Option<u32>,
}

impl From<FetchRecord> for FetchRecordDto {
    fn from(record: FetchRecord) -> Self {
        Self {
            feed_id: record.feed_id.0,
            fetched_at: record.fetched_at.to_string(),
            status: record.status.into(),
            http_status: record.http_status,
            error: record.error,
            articles_new: record.articles_new,
            duration_ms: record.duration_ms,
        }
    }
}

/// Mirror of [`SaveOutcome`] — what a promote wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SaveOutcomeDto {
    /// The destination written to (name form).
    pub destination: String,
    /// Note path relative to the destination root.
    pub path: String,
    /// Managed-region checksum (change token, `sha256:<hex>`).
    pub checksum: String,
    /// Created / updated / unchanged.
    pub disposition: ExportDispositionDto,
}

impl From<SaveOutcome> for SaveOutcomeDto {
    fn from(outcome: SaveOutcome) -> Self {
        Self {
            destination: outcome.destination.to_string(),
            path: outcome.path,
            checksum: outcome.checksum.to_string(),
            disposition: outcome.disposition.into(),
        }
    }
}

/// Mirror of [`Destination`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DestinationDto {
    /// Registry name (the only promote currency — paths never cross IPC).
    pub name: String,
    /// Root directory, for display only.
    pub root: String,
}

impl From<Destination> for DestinationDto {
    fn from(destination: Destination) -> Self {
        Self {
            name: destination.name.to_string(),
            root: destination.root.display().to_string(),
        }
    }
}

/// Mirror of [`OpmlImportOutcome`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct OpmlImportOutcomeDto {
    /// Feeds newly subscribed.
    pub added: u64,
    /// Feeds skipped (already subscribed).
    pub skipped: u64,
}

impl From<OpmlImportOutcome> for OpmlImportOutcomeDto {
    fn from(outcome: OpmlImportOutcome) -> Self {
        Self {
            added: u64::try_from(outcome.added).unwrap_or(u64::MAX),
            skipped: u64::try_from(outcome.skipped).unwrap_or(u64::MAX),
        }
    }
}

/// Backend-owned unread counts (SQL `GROUP BY feed_id` — never client
/// badge math). Articles belonging to no feed count toward `total` only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct UnreadCountsDto {
    /// Total unread articles, across and outside feeds.
    pub total: u64,
    /// Per-feed unread counts as `(feed_id, unread)` pairs.
    pub by_feed: Vec<(i64, u64)>,
}

impl From<BTreeMap<Option<FeedId>, u64>> for UnreadCountsDto {
    fn from(counts: BTreeMap<Option<FeedId>, u64>) -> Self {
        let total = counts.values().sum();
        let by_feed = counts
            .into_iter()
            .filter_map(|(feed_id, unread)| feed_id.map(|id| (id.0, unread)))
            .collect();
        Self { total, by_feed }
    }
}

/// About-box / doctor-panel app facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct AppInfoDto {
    /// The core (workspace) version string.
    pub version: String,
    /// Absolute profile directory path (display only).
    pub profile_dir: String,
    /// Schema version found in the opened database.
    pub db_schema_version: i64,
    /// Highest schema version this build supports.
    pub schema_supported: i64,
}

/// Doctor-panel database stats.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct DbStatsDto {
    /// Total stored articles.
    pub article_count: u64,
    /// Schema version in the database.
    pub schema_version: i64,
    /// Absolute database path (display only).
    pub db_path: String,
}

/// An opaque, single-use handle to a dialog-picked filesystem path
/// (see [`crate::ipc_policy`]) plus a display form for the UI. The token
/// is the only thing path-consuming commands accept — raw paths never
/// cross IPC as free strings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct PathTokenDto {
    /// The opaque token (ULID) to hand back to a consuming command.
    pub token: String,
    /// Absolute path, for display only — never accepted back as input.
    pub path: String,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn feed_status_round_trips() {
        for status in [FeedStatus::Active, FeedStatus::Paused, FeedStatus::Dead] {
            let dto = FeedStatusDto::from(status);
            assert_eq!(FeedStatus::from(dto), status);
        }
    }

    #[test]
    fn status_enums_serialize_snake_case() {
        let json = serde_json::to_value(FetchStatusDto::NotModified).unwrap();
        assert_eq!(json, "not_modified");
        let json = serde_json::to_value(FeedStatusDto::Dead).unwrap();
        assert_eq!(json, "dead");
        let json = serde_json::to_value(ExportDispositionDto::Unchanged).unwrap();
        assert_eq!(json, "unchanged");
    }

    #[test]
    fn unread_counts_total_includes_the_feedless_bucket() {
        let mut counts = BTreeMap::new();
        counts.insert(Some(FeedId(1)), 3);
        counts.insert(Some(FeedId(2)), 2);
        counts.insert(None, 4);
        let dto = UnreadCountsDto::from(counts);
        assert_eq!(dto.total, 9);
        assert_eq!(dto.by_feed, vec![(1, 3), (2, 2)]);
    }

    #[test]
    fn list_params_map_ids_into_newtypes() {
        let params = ListArticles::from(ListArticlesDto {
            feed_id: Some(7),
            before: Some(100),
            limit: 25,
            read: Some(false),
            starred: None,
            read_later: None,
            archived: Some(false),
            tag: Some("rust".into()),
        });
        assert_eq!(params.feed_id, Some(FeedId(7)));
        assert_eq!(params.before, Some(curio_core::model::ArticleId(100)));
        assert_eq!(params.limit, 25);
        assert_eq!(params.read, Some(false));
        assert_eq!(params.tag.as_deref(), Some("rust"));
    }
}
