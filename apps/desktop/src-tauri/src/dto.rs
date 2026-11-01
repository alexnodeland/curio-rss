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
use std::path::Path;

use curio_core::export::ExportDisposition;
use curio_core::model::{
    Article, ArticleState, Feed, FeedId, FeedStatus, FetchErrorKind, FetchRecord, FetchStatus,
};
use curio_core::paths::{LeftReason, LegacyProfile};
use curio_core::storage::ListArticles;
use curio_core::{
    ImportKind, ImportOutcome, ImportSource, OpmlImportOutcome, RefreshOutcome, RejectReason,
    Rejected, SaveOutcome,
};
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

/// Mirror of [`FetchErrorKind`]: why a fetch failed, for the UI to branch
/// on (an icon, a localized hint) instead of parsing the English `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FetchErrorKindDto {
    /// Malformed, non-http(s), userinfo-bearing or host-less URL.
    InvalidUrl,
    /// The host name did not resolve.
    Dns,
    /// The SSRF guard refused a non-public address.
    Blocked,
    /// Nothing accepted the connection.
    Refused,
    /// The connection was reset mid-request.
    Reset,
    /// The server's TLS certificate was rejected.
    TlsCertificate,
    /// The TLS handshake failed for another reason.
    TlsHandshake,
    /// Connecting timed out.
    ConnectTimeout,
    /// The server did not finish answering in time.
    ReadTimeout,
    /// A redirect could not be followed.
    Redirect,
    /// The body exceeded the size cap.
    TooLarge,
    /// Any other transport failure.
    Transport,
    /// A non-success HTTP status (see `http_status`).
    Http,
    /// HTTP 410 — the feed is dead.
    Gone,
    /// The URL serves a web page, not a feed.
    NotAFeed,
    /// The body looked like a feed but did not parse.
    Parse,
    /// Another Curio process was refreshing the feed; nothing was fetched.
    Busy,
}

impl From<FetchErrorKind> for FetchErrorKindDto {
    fn from(kind: FetchErrorKind) -> Self {
        match kind {
            FetchErrorKind::InvalidUrl => Self::InvalidUrl,
            FetchErrorKind::Dns => Self::Dns,
            FetchErrorKind::Blocked => Self::Blocked,
            FetchErrorKind::Refused => Self::Refused,
            FetchErrorKind::Reset => Self::Reset,
            FetchErrorKind::TlsCertificate => Self::TlsCertificate,
            FetchErrorKind::TlsHandshake => Self::TlsHandshake,
            FetchErrorKind::ConnectTimeout => Self::ConnectTimeout,
            FetchErrorKind::ReadTimeout => Self::ReadTimeout,
            FetchErrorKind::Redirect => Self::Redirect,
            FetchErrorKind::TooLarge => Self::TooLarge,
            FetchErrorKind::Transport => Self::Transport,
            FetchErrorKind::Http => Self::Http,
            FetchErrorKind::Gone => Self::Gone,
            FetchErrorKind::NotAFeed => Self::NotAFeed,
            FetchErrorKind::Parse => Self::Parse,
            FetchErrorKind::Busy => Self::Busy,
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
    /// Error text of the feed's most recent fetch, when that attempt errored
    /// (`null` ⇒ currently healthy). Persisted-derived, so the sidebar health
    /// dot is honest on a cold start before any in-session refresh.
    pub last_error: Option<String>,
    /// Failure class of that errored fetch, when known.
    pub last_error_kind: Option<FetchErrorKindDto>,
    /// When the feed last fetched successfully (RFC 3339 UTC), if ever.
    pub last_ok_at: Option<String>,
    /// Feed-level tags (OPML folders land here).
    pub tags: Vec<String>,
    /// Full-text mode: new articles are hydrated from their source pages
    /// at refresh time.
    pub fetch_full_text: bool,
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
            last_error: feed.last_error,
            last_error_kind: feed.last_error_kind.map(Into::into),
            last_ok_at: feed.last_ok_at.map(|t| t.to_string()),
            tags: feed.tags,
            fetch_full_text: feed.fetch_full_text,
        }
    }
}

/// Args for adding a subscription.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct NewFeedDto {
    /// What to subscribe: a feed URL, or anything discovery understands
    /// (a site, a bare domain, a page a rewrite rule knows).
    pub url: String,
    /// Optional user-supplied title.
    pub title: Option<String>,
    /// Initial tags.
    pub tags: Vec<String>,
    /// Resolve `url` through the core's validating discovery and subscribe
    /// to the feed it finds — the default. `false` is the explicit
    /// "subscribe anyway" escape hatch: `url` is only normalized (a bare
    /// domain gains `https://`), never fetched.
    pub validate: bool,
}

/// The longest article-text excerpt the list DTO carries. The extracted
/// text is already read per row (it backs FTS); the list keeps only this
/// bounded preview, not the full body, so the "small payload" intent holds.
const SNIPPET_MAX_CHARS: usize = 200;

/// A list row: everything the article list renders, plus a short text
/// preview — never the full content payload (that stays on [`ArticleDto`]).
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
    /// A whitespace-normalized, length-bounded preview of the extracted
    /// text (up to `SNIPPET_MAX_CHARS`, `…`-suffixed when truncated).
    /// `None` when the article has no extracted text.
    pub snippet: Option<String>,
}

/// Builds the bounded, whitespace-normalized preview for a list row.
fn article_snippet(text: &str) -> Option<String> {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.is_empty() {
        return None;
    }
    let mut snippet: String = normalized.chars().take(SNIPPET_MAX_CHARS).collect();
    if normalized.chars().count() > SNIPPET_MAX_CHARS {
        snippet.push('…');
    }
    Some(snippet)
}

impl From<Article> for ArticleSummaryDto {
    fn from(article: Article) -> Self {
        let snippet = article_snippet(&article.content.text);
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
            snippet,
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

/// Mirror of [`curio_core::BulkSaveOutcome`] — what a bulk note export did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct BulkSaveOutcomeDto {
    /// Notes written for the first time.
    pub created: u64,
    /// Existing notes whose content changed.
    pub updated: u64,
    /// Idempotency hits — already exported and unchanged.
    pub unchanged: u64,
}

impl From<curio_core::BulkSaveOutcome> for BulkSaveOutcomeDto {
    fn from(outcome: curio_core::BulkSaveOutcome) -> Self {
        Self {
            created: outcome.created,
            updated: outcome.updated,
            unchanged: outcome.unchanged,
        }
    }
}

/// Reddit API credential status (D15). Deliberately secret-free: the
/// client secret never crosses IPC back toward the webview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RedditApiStatusDto {
    /// Whether credentials are installed.
    pub configured: bool,
    /// The stored app's client id, when configured.
    pub client_id: Option<String>,
}

/// Mirror of [`curio_core::SavedUrl`] — what a single-URL save did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct SavedUrlDto {
    /// The stored (or already-present) article.
    pub article: ArticleDto,
    /// `false`: the URL was already in the library and was re-flagged.
    pub created: bool,
    /// Whether full-text content was fetched by this save (`false` for an
    /// unreachable page — the bare link is kept — and for existing rows).
    pub hydrated: bool,
}

impl From<curio_core::SavedUrl> for SavedUrlDto {
    fn from(saved: curio_core::SavedUrl) -> Self {
        Self {
            article: saved.article.into(),
            created: saved.created,
            hydrated: saved.hydrated,
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
    /// Keep only articles whose feed is in this folder (a `/`-path tag equal
    /// to this or nested beneath it) — the folder-tree filter.
    pub feed_tag: Option<String>,
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
            feed_tag: params.feed_tag,
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
    /// Failure class for `status: "error"`.
    pub error_kind: Option<FetchErrorKindDto>,
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
            error_kind: outcome.error_kind.map(Into::into),
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
    /// Failure class for `status: "error"`, when known.
    pub error_kind: Option<FetchErrorKindDto>,
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
            error_kind: record.error_kind.map(Into::into),
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
    /// Outlines dropped as unreadable or with an unusable URL.
    pub invalid: u64,
}

impl From<OpmlImportOutcome> for OpmlImportOutcomeDto {
    fn from(outcome: OpmlImportOutcome) -> Self {
        Self {
            added: u64::try_from(outcome.added).unwrap_or(u64::MAX),
            skipped: u64::try_from(outcome.skipped).unwrap_or(u64::MAX),
            invalid: u64::try_from(outcome.invalid).unwrap_or(u64::MAX),
        }
    }
}

/// Mirror of [`ImportSource`] — the refugee-import format the user picked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ImportSourceDto {
    /// OPML 2.0 subscription list.
    Opml,
    /// Pocket CSV export.
    PocketCsv,
    /// Instapaper CSV export.
    InstapaperCsv,
    /// Readwise Reader CSV export.
    ReadwiseCsv,
}

impl From<ImportSourceDto> for ImportSource {
    fn from(source: ImportSourceDto) -> Self {
        match source {
            ImportSourceDto::Opml => Self::Opml,
            ImportSourceDto::PocketCsv => Self::PocketCsv,
            ImportSourceDto::InstapaperCsv => Self::InstapaperCsv,
            ImportSourceDto::ReadwiseCsv => Self::ReadwiseCsv,
        }
    }
}

/// Mirror of [`ImportKind`] — what a dropped import row would have been.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ImportKindDto {
    /// A feed subscription.
    Feed,
    /// A saved article.
    Article,
}

impl From<ImportKind> for ImportKindDto {
    fn from(kind: ImportKind) -> Self {
        match kind {
            ImportKind::Feed => Self::Feed,
            ImportKind::Article => Self::Article,
        }
    }
}

/// Mirror of [`RejectReason`] — why an import row was dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ImportRejectReasonDto {
    /// The row itself could not be read.
    Malformed,
    /// The URL is not a parseable absolute URL.
    InvalidUrl,
    /// The URL is not http(s).
    UnsupportedScheme,
}

impl From<RejectReason> for ImportRejectReasonDto {
    fn from(reason: RejectReason) -> Self {
        match reason {
            RejectReason::Malformed => Self::Malformed,
            RejectReason::InvalidUrl => Self::InvalidUrl,
            RejectReason::UnsupportedScheme => Self::UnsupportedScheme,
        }
    }
}

/// Mirror of [`Rejected`] — one dropped import row and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ImportRejectionDto {
    /// Feed or article.
    pub kind: ImportKindDto,
    /// Why it was dropped.
    pub reason: ImportRejectReasonDto,
    /// The offending value (the raw URL) or a description of the damage.
    pub detail: String,
}

impl From<Rejected> for ImportRejectionDto {
    fn from(rejected: Rejected) -> Self {
        Self {
            kind: rejected.kind.into(),
            reason: rejected.reason.into(),
            detail: rejected.detail,
        }
    }
}

/// Mirror of [`ImportOutcome`] — feeds and articles counted separately so
/// the UI can report "N feeds, M articles" without guessing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct ImportOutcomeDto {
    /// Feeds newly subscribed.
    pub feeds_added: u64,
    /// Feeds skipped (already subscribed).
    pub feeds_skipped: u64,
    /// Feed rows dropped: unreadable, or with an unusable URL.
    pub feeds_invalid: u64,
    /// Articles saved as feedless read-later items.
    pub articles_added: u64,
    /// Articles skipped (already imported).
    pub articles_skipped: u64,
    /// Article rows dropped: unreadable, or with an unusable URL.
    pub articles_invalid: u64,
    /// Why rows were dropped (the first
    /// [`curio_core::MAX_REJECTED_DETAILS`]; the counters stay exact).
    pub rejected: Vec<ImportRejectionDto>,
}

impl From<ImportOutcome> for ImportOutcomeDto {
    fn from(outcome: ImportOutcome) -> Self {
        Self {
            feeds_added: u64::try_from(outcome.feeds_added).unwrap_or(u64::MAX),
            feeds_skipped: u64::try_from(outcome.feeds_skipped).unwrap_or(u64::MAX),
            feeds_invalid: u64::try_from(outcome.feeds_invalid).unwrap_or(u64::MAX),
            articles_added: u64::try_from(outcome.articles_added).unwrap_or(u64::MAX),
            articles_skipped: u64::try_from(outcome.articles_skipped).unwrap_or(u64::MAX),
            articles_invalid: u64::try_from(outcome.articles_invalid).unwrap_or(u64::MAX),
            rejected: outcome.rejected.into_iter().map(Into::into).collect(),
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

/// What startup did with a profile an earlier release kept outside the
/// shared default location ([`curio_core::paths::adopt_legacy_profiles`]):
/// the startup notice and the doctor panel render it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct LegacyProfileDto {
    /// The legacy profile's directory — where it was, when moved (display
    /// only).
    pub path: String,
    /// The default profile shared with the CLI (display only).
    pub default_dir: String,
    /// Moved, or why it was left where it is.
    pub status: LegacyProfileStatusDto,
    /// The OS / database error behind `unreadable` and `move_failed`
    /// (engine English, display only); `null` otherwise.
    pub detail: Option<String>,
    /// `true` when this session opened it where it is, because a fresh
    /// default would have stranded it (see
    /// [`curio_core::paths::in_place_fallback`]).
    pub opened_in_place: bool,
}

/// Mirror of [`curio_core::paths::LegacyProfile`]'s outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum LegacyProfileStatusDto {
    /// Moved into the shared default location.
    Moved,
    /// The default already holds a library; never merged.
    DefaultInUse,
    /// More than one legacy profile exists.
    Ambiguous,
    /// Another process has it open.
    InUse,
    /// The default directory exists and is not empty.
    DefaultNotEmpty,
    /// Its database could not be checked.
    Unreadable,
    /// The move failed.
    MoveFailed,
}

impl LegacyProfileDto {
    /// The wire form of one outcome; `in_place` is the legacy profile this
    /// session opened instead of the default, if any.
    #[must_use]
    pub fn new(outcome: &LegacyProfile, default_dir: &Path, in_place: Option<&Path>) -> Self {
        let (path, status, detail) = match outcome {
            LegacyProfile::Migrated { from, .. } => (from, LegacyProfileStatusDto::Moved, None),
            LegacyProfile::Left { path, reason } => {
                let (status, detail) = match reason {
                    LeftReason::DefaultInUse => (LegacyProfileStatusDto::DefaultInUse, None),
                    LeftReason::Ambiguous => (LegacyProfileStatusDto::Ambiguous, None),
                    LeftReason::InUse => (LegacyProfileStatusDto::InUse, None),
                    LeftReason::DefaultNotEmpty => (LegacyProfileStatusDto::DefaultNotEmpty, None),
                    LeftReason::Unreadable(error) => {
                        (LegacyProfileStatusDto::Unreadable, Some(error.clone()))
                    }
                    LeftReason::MoveFailed(error) => {
                        (LegacyProfileStatusDto::MoveFailed, Some(error.clone()))
                    }
                };
                (path, status, detail)
            }
        };
        Self {
            path: path.display().to_string(),
            default_dir: default_dir.display().to_string(),
            status,
            detail,
            opened_in_place: in_place == Some(path.as_path()),
        }
    }
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

    use std::path::PathBuf;

    use super::*;

    #[test]
    fn feed_status_round_trips() {
        for status in [FeedStatus::Active, FeedStatus::Paused, FeedStatus::Dead] {
            let dto = FeedStatusDto::from(status);
            assert_eq!(FeedStatus::from(dto), status);
        }
    }

    #[test]
    fn article_snippet_normalizes_and_bounds() {
        assert_eq!(article_snippet(""), None);
        assert_eq!(article_snippet("   \n\t  "), None);
        assert_eq!(
            article_snippet("  hello   world\n\nagain  "),
            Some("hello world again".to_owned())
        );
        let long = "word ".repeat(100);
        let snippet = article_snippet(&long).unwrap();
        assert_eq!(snippet.chars().count(), SNIPPET_MAX_CHARS + 1);
        assert!(snippet.ends_with('…'));
    }

    #[test]
    fn fetch_error_kinds_cross_the_wire_as_the_core_text_form() {
        // The UI branches on the same strings `fetch_log` stores and the
        // CLI prints — one vocabulary, whichever head reports it.
        for kind in FetchErrorKind::ALL {
            let json = serde_json::to_value(FetchErrorKindDto::from(kind)).unwrap();
            assert_eq!(json, kind.as_str(), "{kind:?}");
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
        let json = serde_json::to_value(LegacyProfileStatusDto::DefaultNotEmpty).unwrap();
        assert_eq!(json, "default_not_empty");
    }

    #[test]
    fn legacy_profile_outcomes_map_onto_the_wire() {
        let default = Path::new("/shared");
        let moved = LegacyProfileDto::new(
            &LegacyProfile::Migrated {
                from: PathBuf::from("/old"),
                to: default.to_path_buf(),
            },
            default,
            None,
        );
        assert_eq!(moved.path, "/old");
        assert_eq!(moved.default_dir, "/shared");
        assert_eq!(moved.status, LegacyProfileStatusDto::Moved);
        assert!(!moved.opened_in_place);

        let cases = [
            (
                LeftReason::DefaultInUse,
                LegacyProfileStatusDto::DefaultInUse,
                None,
            ),
            (
                LeftReason::Ambiguous,
                LegacyProfileStatusDto::Ambiguous,
                None,
            ),
            (LeftReason::InUse, LegacyProfileStatusDto::InUse, None),
            (
                LeftReason::DefaultNotEmpty,
                LegacyProfileStatusDto::DefaultNotEmpty,
                None,
            ),
            (
                LeftReason::Unreadable("bad".into()),
                LegacyProfileStatusDto::Unreadable,
                Some("bad"),
            ),
            (
                LeftReason::MoveFailed("xdev".into()),
                LegacyProfileStatusDto::MoveFailed,
                Some("xdev"),
            ),
        ];
        for (reason, status, detail) in cases {
            let outcome = LegacyProfile::Left {
                path: PathBuf::from("/old"),
                reason,
            };
            let dto = LegacyProfileDto::new(&outcome, default, Some(Path::new("/old")));
            assert_eq!(dto.status, status);
            assert_eq!(dto.detail.as_deref(), detail);
            assert!(dto.opened_in_place);
            let elsewhere = LegacyProfileDto::new(&outcome, default, Some(Path::new("/other")));
            assert!(!elsewhere.opened_in_place);
        }
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
            feed_tag: Some("Tech/Databases".into()),
        });
        assert_eq!(params.feed_id, Some(FeedId(7)));
        assert_eq!(params.before, Some(curio_core::model::ArticleId(100)));
        assert_eq!(params.limit, 25);
        assert_eq!(params.read, Some(false));
        assert_eq!(params.tag.as_deref(), Some("rust"));
        assert_eq!(params.feed_tag.as_deref(), Some("Tech/Databases"));
    }
}
