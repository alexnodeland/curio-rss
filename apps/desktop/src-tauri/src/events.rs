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

use crate::dto::RefreshOutcomeDto;

/// The feed set or feed metadata changed (add/remove/status/metadata/
/// refresh bookkeeping). Invalidate the feed tree.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, Event)]
pub struct FeedsChanged;

/// Article rows or per-article state changed. `feed_id: Some(id)` scopes
/// the invalidation to one feed's lists; `None` means not scoped to a
/// single feed (state flips, deletes, refresh-all) — invalidate broadly.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, Event)]
pub struct ArticlesChanged {
    /// The feed whose articles changed, when known.
    pub feed_id: Option<i64>,
}

/// One feed finished during `refresh_all` — drive the progress UI.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct RefreshProgress {
    /// The feed that finished.
    pub feed_id: i64,
    /// Its outcome.
    pub outcome: RefreshOutcomeDto,
}

/// The whole `refresh_all` sweep finished.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, Event)]
pub struct RefreshFinished {
    /// Every feed's outcome, in refresh order.
    pub outcomes: Vec<RefreshOutcomeDto>,
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
