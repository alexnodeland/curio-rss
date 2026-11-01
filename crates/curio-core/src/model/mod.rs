//! Domain types of the reading engine.
//!
//! These are the storage-layer's vocabulary: what a feed, an article, and
//! its state *are* inside Curio. They map cleanly onto the published wire
//! types in [`curio_types`] where they cross the boundary (see
//! [`Article::snapshot`]), but they are not the wire types — the contracts
//! in `docs/design/contracts-draft.md` own the wire.

use curio_types::{ArticleSnapshot, Checksum, CurioId, DestinationName, Timestamp};

/// Row id of a feed (`feeds.id`, an `INTEGER PRIMARY KEY` rowid alias).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FeedId(pub i64);

impl std::fmt::Display for FeedId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Row id of an article (`articles.id`, an `INTEGER PRIMARY KEY` rowid
/// alias — the stable FTS5 external-content mapping).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArticleId(pub i64);

impl std::fmt::Display for ArticleId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// Lifecycle state of a feed subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FeedStatus {
    /// Fetched on the normal schedule.
    #[default]
    Active,
    /// Excluded from refresh (user-paused or backing off).
    Paused,
    /// HTTP 410 auto-pause: the feed is gone and is never fetched again.
    Dead,
}

impl FeedStatus {
    /// The stored TEXT form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Dead => "dead",
        }
    }

    /// Parses the stored TEXT form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "dead" => Some(Self::Dead),
            _ => None,
        }
    }
}

/// A subscribed feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    /// Row id.
    pub id: FeedId,
    /// Feed URL — the unique key of a subscription.
    pub url: String,
    /// Human-readable feed name, if known.
    pub title: Option<String>,
    /// The feed's website, if known.
    pub site_url: Option<String>,
    /// Feed self-description, if any.
    pub description: Option<String>,
    /// Conditional-GET `ETag` from the last successful fetch.
    pub etag: Option<String>,
    /// Conditional-GET `Last-Modified` from the last successful fetch.
    pub last_modified: Option<String>,
    /// Lifecycle state.
    pub status: FeedStatus,
    /// Contract W1: exempts this feed from the SSRF private-network guard.
    /// Settable only by explicit config edit — never from feed content.
    pub allow_private_network: bool,
    /// When the subscription was added.
    pub added_at: Timestamp,
    /// When the feed was last fetched, if ever.
    pub last_fetched_at: Option<Timestamp>,
    /// Error text of the feed's most recent fetch attempt, when that attempt
    /// errored (`None` ⇒ currently healthy). Derived from the persisted
    /// `fetch_log`, so the sidebar health dot is honest on a cold start,
    /// before any in-session refresh has run.
    pub last_error: Option<String>,
    /// When the feed last fetched *successfully* (`ok`/`not_modified`), if ever.
    /// Also derived from `fetch_log`.
    pub last_ok_at: Option<Timestamp>,
    /// Tags assigned to the feed (normalized: non-empty, unique). Carried
    /// on `feed.added` and exported as the OPML `category` attribute, so
    /// an import/export cycle keeps folder/category structure.
    pub tags: Vec<String>,
    /// Full-text mode: fetch + readability-extract the source page of
    /// every new article at refresh time (for content-free feeds).
    pub fetch_full_text: bool,
    /// Failed fetch attempts in a row (reset by any success, or by a
    /// resume). Drives the backoff and the auto-pause.
    pub consecutive_failures: u32,
    /// The earliest time a refresh sweep fetches this feed again — set
    /// by a backoff, a server's `Retry-After`, the feed's own refresh
    /// hint (`<ttl>`, `sy:updatePeriod`, `Cache-Control: max-age`) or
    /// [`Self::refresh_interval_min`]. `None` ⇒ due on every sweep.
    pub next_attempt_at: Option<Timestamp>,
    /// What kind of failure the most recent errored attempt was; `None`
    /// once a fetch succeeds.
    pub last_error_kind: Option<FetchErrorKind>,
    /// The user's per-feed refresh interval in minutes: sweeps leave the
    /// feed alone for at least this long after each attempt. `None` ⇒
    /// the global schedule (and the feed's own hints) decide.
    pub refresh_interval_min: Option<u32>,
    /// When the current run of failed attempts began; `None` while the
    /// feed is healthy.
    pub failing_since: Option<Timestamp>,
    /// When the engine paused this feed by itself after a run of failed
    /// fetches; `None` for an active feed or one the user paused.
    pub auto_paused_at: Option<Timestamp>,
}

impl Feed {
    /// Whether a refresh sweep should fetch this feed at `now`: it is
    /// active and not inside a backoff / `Retry-After` / refresh-hint
    /// window.
    #[must_use]
    pub fn is_due(&self, now: Timestamp) -> bool {
        self.status == FeedStatus::Active && self.next_attempt_at.is_none_or(|next| next <= now)
    }
}

/// The class of a failed fetch attempt — what the backoff, the 410
/// confirmation and the rate-limit guard key on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchErrorKind {
    /// HTTP 429, or a 503 carrying `Retry-After`: the host asked us to
    /// slow down. Not even a manual refresh fetches before the deadline.
    RateLimited,
    /// HTTP 410. A second one in a row marks the feed dead.
    Gone,
    /// Any other 5xx.
    Server,
    /// Any other non-2xx/304 status (404, 403, …).
    Client,
    /// No response: DNS, connect, timeout, TLS or transport failure.
    Network,
    /// The policed client refused the request (private address, bad URL,
    /// redirect limits, body cap).
    Refused,
    /// A 2xx whose body is not a recognizable feed.
    Parse,
}

impl FetchErrorKind {
    /// The stored TEXT form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RateLimited => "rate_limited",
            Self::Gone => "gone",
            Self::Server => "server",
            Self::Client => "client",
            Self::Network => "network",
            Self::Refused => "refused",
            Self::Parse => "parse",
        }
    }

    /// Parses the stored TEXT form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        [
            Self::RateLimited,
            Self::Gone,
            Self::Server,
            Self::Client,
            Self::Network,
            Self::Refused,
            Self::Parse,
        ]
        .into_iter()
        .find(|kind| kind.as_str() == s)
    }
}

/// Input for subscribing to a feed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NewFeed {
    /// Feed URL.
    pub url: String,
    /// Human-readable feed name, if already known.
    pub title: Option<String>,
    /// Tags assigned to the feed at subscription time. Carried on the
    /// emitted `feed.added` event (tags-in-payload rule).
    pub tags: Vec<String>,
}

/// The stored content of an article: sanitized HTML plus the extracted
/// plain text that feeds full-text search. Raw feed HTML never reaches
/// storage — sanitize-at-ingest is a pipeline invariant.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArticleContent {
    /// Sanitized HTML (post-ammonia, post-readability).
    pub html: String,
    /// Extracted plain text — the FTS5 `content_text` column.
    pub text: String,
}

/// Where an article's stored content (`content_html`, `content_text`,
/// `word_count`) came from. A refresh upsert writes [`Self::Feed`] (or
/// [`Self::Manual`] for feedless saves) and only ever replaces content of
/// those two origins; [`Self::FullText`] and [`Self::Enriched`] bodies are
/// never reverted to the feed excerpt by a later refresh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentOrigin {
    /// The feed entry's own body (refresh ingest).
    Feed,
    /// A feedless save: a save-URL clip or an import.
    Manual,
    /// The readability extract of the source page (load-full, per-feed
    /// full-text mode).
    FullText,
    /// A source-specific enrichment provider's body (e.g. reddit JSON).
    Enriched,
}

impl ContentOrigin {
    /// The stored TEXT form (`articles.content_origin`).
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feed => "feed",
            Self::Manual => "manual",
            Self::FullText => "full_text",
            Self::Enriched => "enriched",
        }
    }
}

/// Input for inserting (or re-ingesting) an article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewArticle {
    /// The feed the article came from; `None` for manual saves.
    pub feed_id: Option<FeedId>,
    /// Total dedupe key: `guid → link → hash(title+published)` — computed
    /// by ingest, unique per feed item. Storage namespaces it by
    /// provenance (`f<feed_id>:`/`m:`) so it only ever matches within the
    /// same feed (or within manual saves).
    pub dedupe_key: String,
    /// Article title.
    pub title: String,
    /// Canonical article URL.
    pub source_url: String,
    /// Author, if known.
    pub author: Option<String>,
    /// Publication time, if known. Storage clamps a value later than the
    /// article's first-seen time (`saved_at`) to that time.
    pub published_at: Option<Timestamp>,
    /// Sanitized content.
    pub content: ArticleContent,
    /// BCP-47 language tag, if detected.
    pub lang: Option<String>,
    /// Word count of the extracted text, if computed.
    pub word_count: Option<u32>,
    /// The source's own last-update time (distinct from Curio's row
    /// `modified_at`), if the feed carried one.
    pub source_updated_at: Option<Timestamp>,
    /// Lead image URL, if the feed declared one (`media:thumbnail` /
    /// `media:content` / image `enclosure`) or the body carried a
    /// leading `<img>`. Always an absolute `http(s)` URL — consumers
    /// load it through the policed image cache, never directly.
    pub lead_image: Option<String>,
}

/// A stored article.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Article {
    /// Row id (the FTS5 external-content rowid).
    pub id: ArticleId,
    /// Article identity: `UUIDv7`, assigned at insert, never changes.
    pub curio_id: CurioId,
    /// The feed the article came from; `None` for manual saves.
    pub feed_id: Option<FeedId>,
    /// Total dedupe key (`guid → link → hash(title+published)`), in its
    /// stored provenance-scoped form: `f<feed_id>:…` for feed articles,
    /// `m:…` for manual saves — colliding guids across feeds never match.
    pub dedupe_key: String,
    /// Article title.
    pub title: String,
    /// Canonical article URL.
    pub source_url: String,
    /// Author, if known.
    pub author: Option<String>,
    /// Publication time, if known.
    pub published_at: Option<Timestamp>,
    /// Sanitized content.
    pub content: ArticleContent,
    /// BCP-47 language tag, if detected.
    pub lang: Option<String>,
    /// Word count of the extracted text, if computed.
    pub word_count: Option<u32>,
    /// When the article was first stored.
    pub saved_at: Timestamp,
    /// The source's own last-update time, if any.
    pub source_updated_at: Option<Timestamp>,
    /// Lead image URL, if known — the feed's declared image or the body's
    /// first inline `<img>`. Always an absolute `http(s)` URL, loaded
    /// through the policed image cache, never directly.
    pub lead_image: Option<String>,
}

impl Article {
    /// Maps this article onto the wire-side [`ArticleSnapshot`] carried by
    /// the state-carrying `article.saved` / `article.updated` events.
    ///
    /// The export layer supplies what only it knows: the feed's URL/title,
    /// the article's tags at event time (tags-in-payload rule), the named
    /// destination, the in-destination path, and the managed-region
    /// checksum (a change token only — never identity).
    #[must_use]
    pub fn snapshot(
        &self,
        feed: Option<String>,
        feed_title: Option<String>,
        tags: Vec<String>,
        destination: DestinationName,
        path: String,
        checksum: Checksum,
    ) -> ArticleSnapshot {
        ArticleSnapshot {
            curio_id: self.curio_id,
            title: self.title.clone(),
            source: self.source_url.clone(),
            feed,
            feed_title,
            tags,
            published: self.published_at,
            destination,
            path,
            checksum,
        }
    }
}

/// Current-state projection of an article's flags (`article_state`).
///
/// This is a *projection*: the durable history lives in the event log, and
/// the flags here are what folding that history yields. Splitting state out
/// of `articles` means flag flips never churn the FTS index and retention
/// can never orphan stated articles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "a flags projection IS four booleans"
)]
pub struct ArticleState {
    /// Read/unread. DB-local: `curio.events.v1` carries no read event.
    pub read: bool,
    /// Member of the starred set.
    pub starred: bool,
    /// Member of the read-later set.
    pub read_later: bool,
    /// Member of the archived set.
    pub archived: bool,
}

/// Outcome of a fetch attempt, recorded in `fetch_log`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchStatus {
    /// 2xx with a parseable body.
    Ok,
    /// 304 — conditional GET hit, nothing new.
    NotModified,
    /// Anything else: network error, HTTP error, parse failure.
    Error,
}

impl FetchStatus {
    /// The stored TEXT form.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::NotModified => "not_modified",
            Self::Error => "error",
        }
    }

    /// Parses the stored TEXT form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ok" => Some(Self::Ok),
            "not_modified" => Some(Self::NotModified),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// One `fetch_log` row: a single fetch attempt against a feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchRecord {
    /// The feed that was fetched.
    pub feed_id: FeedId,
    /// When the attempt happened.
    pub fetched_at: Timestamp,
    /// Outcome class.
    pub status: FetchStatus,
    /// HTTP status code, if a response was received.
    pub http_status: Option<u16>,
    /// Error description for [`FetchStatus::Error`].
    pub error: Option<String>,
    /// Number of new articles stored from this fetch.
    pub articles_new: u32,
    /// Wall-clock duration of the attempt, if measured.
    pub duration_ms: Option<u32>,
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn fetch_error_kind_round_trips_its_text_form() {
        for kind in [
            FetchErrorKind::RateLimited,
            FetchErrorKind::Gone,
            FetchErrorKind::Server,
            FetchErrorKind::Client,
            FetchErrorKind::Network,
            FetchErrorKind::Refused,
            FetchErrorKind::Parse,
        ] {
            assert_eq!(FetchErrorKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(FetchErrorKind::parse("bogus"), None);
    }

    #[test]
    fn feed_status_round_trips_its_text_form() {
        for status in [FeedStatus::Active, FeedStatus::Paused, FeedStatus::Dead] {
            assert_eq!(FeedStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(FeedStatus::parse("bogus"), None);
    }

    #[test]
    fn fetch_status_round_trips_its_text_form() {
        for status in [
            FetchStatus::Ok,
            FetchStatus::NotModified,
            FetchStatus::Error,
        ] {
            assert_eq!(FetchStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(FetchStatus::parse("bogus"), None);
    }

    #[test]
    fn snapshot_maps_the_domain_article_onto_the_wire() {
        let article = Article {
            id: ArticleId(7),
            curio_id: CurioId::new(),
            feed_id: Some(FeedId(1)),
            dedupe_key: "guid:abc".to_owned(),
            title: "Title".to_owned(),
            source_url: "https://example.com/a".to_owned(),
            author: Some("Jane".to_owned()),
            published_at: None,
            content: ArticleContent::default(),
            lang: None,
            word_count: Some(12),
            saved_at: Timestamp::now(),
            source_updated_at: None,
            lead_image: None,
        };
        let checksum: Checksum =
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
                .parse()
                .unwrap();
        let snapshot = article.snapshot(
            Some("https://example.com/feed.xml".to_owned()),
            Some("Example".to_owned()),
            vec!["rust".to_owned()],
            "vault".parse().unwrap(),
            "curio/title.md".to_owned(),
            checksum,
        );
        assert_eq!(snapshot.curio_id, article.curio_id);
        assert_eq!(snapshot.source, article.source_url);
        assert_eq!(snapshot.tags, vec!["rust".to_owned()]);
        assert_eq!(snapshot.path, "curio/title.md");
    }
}
