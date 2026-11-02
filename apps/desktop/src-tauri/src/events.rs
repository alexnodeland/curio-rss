//! Rust-emitted invalidation events (specta-typed).
//!
//! Core has no push/subscribe API, so the head is the event source: every
//! mutating command emits after success, and the frontend query cache
//! subscribes through the generated bindings — it never tails the JSONL
//! contract event log (that log is for external consumers).
//!
//! Idempotency gating: commands whose core call returns `changed: false`
//! emit nothing, so the cache can trust every event to mean real change.

use serde::{Deserialize, Serialize};
use tauri_specta::Event;

use crate::dto::{ArticleFlagDto, RefreshOutcomeDto};

/// The feed set or feed metadata changed (add/remove/status/metadata/
/// refresh bookkeeping). Invalidate the feed tree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, Event)]
pub struct FeedsChanged;

/// Article rows or per-article state changed. `feed_id: Some(id)` scopes
/// the invalidation to one feed's lists; `None` means not scoped to a
/// single feed (deletes, refresh-all, tag edits) — invalidate broadly.
///
/// `flags: Some(..)` narrows it further (#135): the change was only one state
/// flag set to one value on exactly those articles — no row was added or
/// removed and no content changed. The UI cache then patches the articles'
/// flags in place and refetches only the lists and counts that flag can move,
/// instead of the whole cache.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct ArticlesChanged {
    /// The feed whose articles changed, when known.
    pub feed_id: Option<i64>,
    /// The state-flag flip this change consists of, when that is all it is.
    pub flags: Option<FlagChange>,
}

impl ArticlesChanged {
    /// Rows or content changed (refresh, delete, import, tags, full text) —
    /// in one feed (`Some`) or anywhere (`None`).
    pub(crate) fn rows(feed_id: Option<i64>) -> Self {
        Self {
            feed_id,
            flags: None,
        }
    }

    /// `field` was set to `value` on `article_ids`, and nothing else changed.
    /// `feed_id` scopes it to one feed when every id is known to be in it.
    pub(crate) fn flag(
        feed_id: Option<i64>,
        field: FlagField,
        article_ids: Vec<i64>,
        value: bool,
    ) -> Self {
        Self {
            feed_id,
            flags: Some(FlagChange {
                article_ids,
                field,
                value,
            }),
        }
    }
}

/// One state flag set to one value on a set of articles.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct FlagChange {
    /// The articles whose flag now holds `value` (an id that no longer
    /// exists may appear; it matches no cached row).
    pub article_ids: Vec<i64>,
    /// Which flag.
    pub field: FlagField,
    /// Its value on every listed article now.
    pub value: bool,
}

/// A per-article state flag, named as its `ArticleStateDto` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum FlagField {
    /// Read / unread.
    Read,
    /// Starred.
    Starred,
    /// In the read-later queue.
    ReadLater,
    /// Archived.
    Archived,
}

impl From<ArticleFlagDto> for FlagField {
    fn from(flag: ArticleFlagDto) -> Self {
        match flag {
            ArticleFlagDto::Read => Self::Read,
            ArticleFlagDto::Starred => Self::Starred,
            ArticleFlagDto::ReadLater => Self::ReadLater,
            ArticleFlagDto::Archived => Self::Archived,
        }
    }
}

/// One article's reading position moved (`set_read_progress` stored a new
/// value). Narrower than [`ArticlesChanged`] on purpose: a position save
/// changes no list, count or tag, so only that article's state query
/// refreshes — the reader saves as you scroll, and a list-wide refetch per
/// save would be waste.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, Event)]
pub struct ReadProgressChanged {
    /// The article whose position changed.
    pub article_id: i64,
}

/// A feed or a full-text page finished during a `refresh_all` sweep —
/// drive the progress UI ("12/300", then "Loading full text… 3/12").
/// Feeds refresh concurrently, so progress arrives in completion order. A
/// full-text feed's pages load after the feed, so a sweep can run on past
/// its last feed: each page that finishes reports too, keeping the sweep
/// visibly alive.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct RefreshProgress {
    /// The sweep this progress belongs to (counts up per app run).
    pub sweep_id: u64,
    /// Feeds finished so far.
    pub done: u32,
    /// Feeds the sweep set out to refresh.
    pub total: u32,
    /// Full-text pages finished so far (loaded, failed or dropped).
    pub pages_done: u32,
    /// Full-text pages queued so far — final once `done` reaches `total`.
    pub pages_total: u32,
    /// The feed that finished; none when a page did.
    pub feed_id: Option<i64>,
    /// Its outcome; none when a page did.
    pub outcome: Option<RefreshOutcomeDto>,
}

/// The whole `refresh_all` sweep finished (or was cancelled).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct RefreshFinished {
    /// The sweep that finished.
    pub sweep_id: u64,
    /// Whether `cancel_refresh` stopped it early.
    pub cancelled: bool,
    /// One outcome per feed the sweep refreshed, in feed-list order (a
    /// cancelled sweep has none for the feeds it never started or whose
    /// fetch it abandoned — those were not refreshed, they did not fail).
    pub outcomes: Vec<RefreshOutcomeDto>,
}

/// The background scheduler's connectivity verdict changed (#123): a probe
/// before a scheduled sweep found the network gone — the sweep was skipped,
/// so no feed errors were recorded — or back. Drives the "Offline" badge;
/// fires on change only, and `connectivity_online` answers the current state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, Event)]
pub struct ConnectivityChanged {
    /// Whether the network is reachable.
    pub online: bool,
}

/// A native menu item was chosen — the head emits its id and the frontend
/// routes it through the same action layer as a keyboard shortcut (so the
/// menu, shortcuts, and toolbars stay one source of behavior). `id` is a
/// [`crate::menu`] item id: a `ShortcutId` (e.g. `app.addFeed`) or a
/// menu-only id (`menu.docs`, `menu.reportIssue`).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct MenuAction {
    /// The chosen item's id.
    pub id: String,
}

/// Emits an event, logging (never failing the command) on error — a
/// missed invalidation degrades freshness, not correctness.
pub(crate) fn emit_or_log<E>(app: &tauri::AppHandle, event: &E)
where
    E: Event + Serialize + Clone,
{
    if let Err(error) = event.emit(app) {
        tracing::warn!(%error, "event emission failed");
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_flag_flip_names_its_articles_field_and_value_on_the_wire() {
        let event = ArticlesChanged::flag(None, FlagField::ReadLater, vec![7, 9], true);
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            serde_json::json!({
                "feed_id": null,
                "flags": { "article_ids": [7, 9], "field": "read_later", "value": true },
            }),
        );
    }

    #[test]
    fn a_row_change_carries_no_flag_scope() {
        assert_eq!(
            serde_json::to_value(ArticlesChanged::rows(Some(3))).unwrap(),
            serde_json::json!({ "feed_id": 3, "flags": null }),
        );
    }

    #[test]
    fn flag_fields_are_the_article_state_field_names() {
        let names: Vec<serde_json::Value> = [
            FlagField::Read,
            FlagField::Starred,
            FlagField::ReadLater,
            FlagField::Archived,
        ]
        .iter()
        .map(|field| serde_json::to_value(field).unwrap())
        .collect();
        assert_eq!(
            names,
            ["read", "starred", "read_later", "archived"].map(serde_json::Value::from)
        );
    }

    #[test]
    fn a_bulk_flag_names_the_same_field_on_the_wire() {
        for flag in [
            ArticleFlagDto::Read,
            ArticleFlagDto::Starred,
            ArticleFlagDto::ReadLater,
            ArticleFlagDto::Archived,
        ] {
            assert_eq!(
                serde_json::to_value(FlagField::from(flag)).unwrap(),
                serde_json::to_value(flag).unwrap(),
            );
        }
    }
}
