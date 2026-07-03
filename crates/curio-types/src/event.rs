//! `curio.events.v1` — the append-only behavioral event log.

use serde::{Deserialize, Serialize};

use crate::checksum::Checksum;
use crate::destination::DestinationName;
use crate::id::{CurioId, EventId};
use crate::marker::EventsSchemaV1Marker;
use crate::timestamp::Timestamp;

/// One line of a `.curio/events/events-YYYYMMDD.jsonl` file.
///
/// Published schema: `https://curio.dev/schemas/events.v1.json`
/// (`schemas/events.v1.json`).
///
/// Wire form (a single JSON object per line):
///
/// ```json
/// {"schema":"curio.events.v1","event_id":"01J1PZ2M3N4P5Q6R7S8T9V0W1X","ts":"2026-07-03T09:15:00.123Z","type":"article.saved","payload":{...}}
/// ```
///
/// Consumers dedupe (and gain replay-idempotency) by `event_id`. Within a
/// single file `ts` is non-decreasing; across files, file naming gives
/// ordering. The log is never committed to git.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// The literal `curio.events.v1`.
    pub schema: EventsSchemaV1Marker,
    /// ULID, unique per event.
    pub event_id: EventId,
    /// Event time, RFC 3339 UTC with millisecond precision.
    pub ts: Timestamp,
    /// The `type` + `payload` pair, flattened into the envelope.
    #[serde(flatten)]
    pub event: EventPayload,
}

impl EventEnvelope {
    /// Wraps a payload in a fresh envelope: new ULID, `ts` = now.
    #[must_use]
    pub fn new(event: EventPayload) -> Self {
        Self {
            schema: EventsSchemaV1Marker,
            event_id: EventId::new(),
            ts: Timestamp::now(),
            event,
        }
    }
}

/// The 13 event types of `curio.events.v1`, tagged on the wire as the
/// sibling fields `type` (namespaced string) and `payload` (object).
///
/// **Negation rule:** state reconstruction = fold events in order; a
/// negation event removes membership. Histories are NOT monotone —
/// consumers implementing a "starred set" (etc.) MUST honor negations
/// (see [`negates`](Self::negates)).
///
/// **Tags-in-payload rule:** state-carrying events carry the article's
/// tags in the payload, so consumers never need a DB join.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload")]
pub enum EventPayload {
    /// An article was saved. Full snapshot payload.
    #[serde(rename = "article.saved")]
    ArticleSaved(ArticleSnapshot),
    /// A saved article was re-exported with a changed checksum. Full
    /// snapshot payload.
    #[serde(rename = "article.updated")]
    ArticleUpdated(ArticleSnapshot),
    /// The article joined the starred set. State-carrying: includes tags.
    #[serde(rename = "article.starred")]
    ArticleStarred {
        /// Article identity.
        curio_id: CurioId,
        /// The article's tags at event time (tags-in-payload rule).
        tags: Vec<String>,
    },
    /// Negation of [`ArticleStarred`](Self::ArticleStarred): removes the
    /// article from the starred set.
    #[serde(rename = "article.unstarred")]
    ArticleUnstarred {
        /// Article identity.
        curio_id: CurioId,
    },
    /// The article joined the read-later set. State-carrying: includes tags.
    #[serde(rename = "article.read_later.added")]
    ReadLaterAdded {
        /// Article identity.
        curio_id: CurioId,
        /// The article's tags at event time (tags-in-payload rule).
        tags: Vec<String>,
    },
    /// Negation of [`ReadLaterAdded`](Self::ReadLaterAdded): removes the
    /// article from the read-later set.
    #[serde(rename = "article.read_later.removed")]
    ReadLaterRemoved {
        /// Article identity.
        curio_id: CurioId,
    },
    /// The article was archived.
    #[serde(rename = "article.archived")]
    ArticleArchived {
        /// Article identity.
        curio_id: CurioId,
    },
    /// Negation of [`ArticleArchived`](Self::ArticleArchived): removes the
    /// article from the archived set.
    #[serde(rename = "article.unarchived")]
    ArticleUnarchived {
        /// Article identity.
        curio_id: CurioId,
    },
    /// The article was opened for reading.
    #[serde(rename = "article.opened")]
    ArticleOpened {
        /// Article identity.
        curio_id: CurioId,
        /// Reading dwell time in milliseconds, if measured.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dwell_ms: Option<u64>,
    },
    /// A single tag was added to the article.
    #[serde(rename = "article.tagged")]
    ArticleTagged {
        /// Article identity.
        curio_id: CurioId,
        /// The tag that was added.
        tag: String,
    },
    /// Negation of [`ArticleTagged`](Self::ArticleTagged) for the same
    /// `(curio_id, tag)`: removes the tag from the article.
    #[serde(rename = "article.untagged")]
    ArticleUntagged {
        /// Article identity.
        curio_id: CurioId,
        /// The tag that was removed.
        tag: String,
    },
    /// A feed subscription was added.
    #[serde(rename = "feed.added")]
    FeedAdded {
        /// Feed URL.
        feed: String,
        /// Human-readable feed name; `None` (serialized as null) if unknown.
        feed_title: Option<String>,
        /// Tags assigned to the feed at event time.
        tags: Vec<String>,
    },
    /// Negation of [`FeedAdded`](Self::FeedAdded) for the same feed URL:
    /// removes the subscription.
    #[serde(rename = "feed.removed")]
    FeedRemoved {
        /// Feed URL.
        feed: String,
    },
}

impl EventPayload {
    /// The namespaced wire `type` of this event.
    #[must_use]
    pub fn event_type(&self) -> &'static str {
        match self {
            Self::ArticleSaved(_) => "article.saved",
            Self::ArticleUpdated(_) => "article.updated",
            Self::ArticleStarred { .. } => "article.starred",
            Self::ArticleUnstarred { .. } => "article.unstarred",
            Self::ReadLaterAdded { .. } => "article.read_later.added",
            Self::ReadLaterRemoved { .. } => "article.read_later.removed",
            Self::ArticleArchived { .. } => "article.archived",
            Self::ArticleUnarchived { .. } => "article.unarchived",
            Self::ArticleOpened { .. } => "article.opened",
            Self::ArticleTagged { .. } => "article.tagged",
            Self::ArticleUntagged { .. } => "article.untagged",
            Self::FeedAdded { .. } => "feed.added",
            Self::FeedRemoved { .. } => "feed.removed",
        }
    }

    /// The event type this event negates, if it is a negation event.
    ///
    /// Folding events in order, a negation event removes membership
    /// established by the returned type (per tag for `article.untagged`,
    /// per feed for `feed.removed`). Histories are NOT monotone.
    #[must_use]
    pub fn negates(&self) -> Option<&'static str> {
        match self {
            Self::ArticleUnstarred { .. } => Some("article.starred"),
            Self::ReadLaterRemoved { .. } => Some("article.read_later.added"),
            Self::ArticleUnarchived { .. } => Some("article.archived"),
            Self::ArticleUntagged { .. } => Some("article.tagged"),
            Self::FeedRemoved { .. } => Some("feed.added"),
            _ => None,
        }
    }

    /// The article this event is about, if it is an `article.*` event.
    #[must_use]
    pub fn curio_id(&self) -> Option<CurioId> {
        match self {
            Self::ArticleSaved(snapshot) | Self::ArticleUpdated(snapshot) => {
                Some(snapshot.curio_id)
            }
            Self::ArticleStarred { curio_id, .. }
            | Self::ArticleUnstarred { curio_id }
            | Self::ReadLaterAdded { curio_id, .. }
            | Self::ReadLaterRemoved { curio_id }
            | Self::ArticleArchived { curio_id }
            | Self::ArticleUnarchived { curio_id }
            | Self::ArticleOpened { curio_id, .. }
            | Self::ArticleTagged { curio_id, .. }
            | Self::ArticleUntagged { curio_id, .. } => Some(*curio_id),
            Self::FeedAdded { .. } | Self::FeedRemoved { .. } => None,
        }
    }
}

/// The full article snapshot carried by the state-carrying events
/// `article.saved` and `article.updated` — including `tags`, per the
/// tags-in-payload rule (consumers never need a DB join).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArticleSnapshot {
    /// Article identity: `UUIDv7`, assigned at save, never changes.
    pub curio_id: CurioId,
    /// Article title.
    pub title: String,
    /// Canonical article URL.
    pub source: String,
    /// Feed URL the article came from; `None` (serialized as null) if
    /// saved manually.
    pub feed: Option<String>,
    /// Human-readable feed name; `None` (serialized as null) if unknown.
    pub feed_title: Option<String>,
    /// Curio-assigned tags at event time (tags-in-payload rule).
    pub tags: Vec<String>,
    /// Publication time; `None` (serialized as null) if unknown.
    pub published: Option<Timestamp>,
    /// Named destination the note was exported to — destination names,
    /// never raw filesystem paths.
    pub destination: DestinationName,
    /// Note path relative to the destination root.
    pub path: String,
    /// sha256 of the managed content region bytes. Change token ONLY.
    pub checksum: Checksum,
}
