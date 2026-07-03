//! Folding an event stream back into state.
//!
//! The consumer-side (and test-side) reconstruction of the contract's
//! semantics: replay events in order, **dedupe by `event_id`** (replay
//! idempotency), and honor **negation events** — histories are not
//! monotone, so `article.unstarred` removes starred-set membership, and
//! likewise for read-later, archive, tags, and feeds.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use curio_types::{ArticleSnapshot, CurioId, EventEnvelope, EventId, EventPayload};

/// A feed subscription as reconstructed from `feed.added`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FeedSubscription {
    /// Human-readable feed name at event time, if any.
    pub feed_title: Option<String>,
    /// Tags assigned to the feed at event time.
    pub tags: Vec<String>,
}

/// State reconstructed by folding a `curio.events.v1` stream.
#[derive(Debug, Clone, Default)]
pub struct FoldedState {
    /// Last-seen full snapshot per article (`article.saved` /
    /// `article.updated`).
    pub articles: BTreeMap<CurioId, ArticleSnapshot>,
    /// The starred set (net of `article.unstarred` negations).
    pub starred: BTreeSet<CurioId>,
    /// The read-later set (net of `article.read_later.removed`).
    pub read_later: BTreeSet<CurioId>,
    /// The archived set (net of `article.unarchived`).
    pub archived: BTreeSet<CurioId>,
    /// Per-article tag sets from `article.tagged` / `article.untagged`.
    pub tags: BTreeMap<CurioId, BTreeSet<String>>,
    /// Live feed subscriptions keyed by feed URL (net of `feed.removed`).
    pub feeds: BTreeMap<String, FeedSubscription>,
    /// `article.opened` counts per article.
    pub opened: BTreeMap<CurioId, u64>,
    seen: HashSet<EventId>,
}

impl FoldedState {
    /// An empty fold.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Folds a whole stream (events must already be in stream order).
    pub fn fold(events: impl IntoIterator<Item = EventEnvelope>) -> Self {
        let mut state = Self::new();
        for envelope in events {
            state.apply(&envelope);
        }
        state
    }

    /// Applies one envelope; `false` if its `event_id` was already seen
    /// (the duplicate is ignored — replay idempotency).
    pub fn apply(&mut self, envelope: &EventEnvelope) -> bool {
        if !self.seen.insert(envelope.event_id) {
            return false;
        }
        match &envelope.event {
            EventPayload::ArticleSaved(snapshot) | EventPayload::ArticleUpdated(snapshot) => {
                self.articles.insert(snapshot.curio_id, snapshot.clone());
            }
            EventPayload::ArticleStarred { curio_id, .. } => {
                self.starred.insert(*curio_id);
            }
            EventPayload::ArticleUnstarred { curio_id } => {
                self.starred.remove(curio_id);
            }
            EventPayload::ReadLaterAdded { curio_id, .. } => {
                self.read_later.insert(*curio_id);
            }
            EventPayload::ReadLaterRemoved { curio_id } => {
                self.read_later.remove(curio_id);
            }
            EventPayload::ArticleArchived { curio_id } => {
                self.archived.insert(*curio_id);
            }
            EventPayload::ArticleUnarchived { curio_id } => {
                self.archived.remove(curio_id);
            }
            EventPayload::ArticleOpened { curio_id, .. } => {
                *self.opened.entry(*curio_id).or_default() += 1;
            }
            EventPayload::ArticleTagged { curio_id, tag } => {
                self.tags.entry(*curio_id).or_default().insert(tag.clone());
            }
            EventPayload::ArticleUntagged { curio_id, tag } => {
                if let Some(tags) = self.tags.get_mut(curio_id) {
                    tags.remove(tag);
                }
            }
            EventPayload::FeedAdded {
                feed,
                feed_title,
                tags,
            } => {
                self.feeds.insert(
                    feed.clone(),
                    FeedSubscription {
                        feed_title: feed_title.clone(),
                        tags: tags.clone(),
                    },
                );
            }
            EventPayload::FeedRemoved { feed } => {
                self.feeds.remove(feed);
            }
        }
        true
    }

    /// How many distinct events have been folded.
    #[must_use]
    pub fn events_seen(&self) -> usize {
        self.seen.len()
    }
}
