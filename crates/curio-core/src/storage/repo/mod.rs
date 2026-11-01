//! Typed repository functions — the only place SQL exists.
//!
//! Every function either runs on the writer thread (mutations, always
//! transactional) or on a pooled read-only connection (queries). State
//! changes that the `curio.events.v1` contract cares about mint their
//! event envelope and stage it as a write-ahead intent row **in the same
//! transaction** as the state change; the events layer appends staged
//! intents to the JSONL log and deletes them (crash-recoverable — see
//! `events::EventEmitter`).
//!
//! The functions are one `impl Storage` block per table family: `feeds`,
//! `articles`, `state`, `tags`, `fetch_log`, `settings` and `intents`.
//! `rows` holds the helpers they share (tag normalization, dedupe-key
//! scoping, the list/search query compilers, the raw-row parsers). This
//! file holds the public parameter/outcome types, the column lists and
//! [`begin_write`], the one way a mutation opens its transaction.

mod articles;
mod feeds;
mod fetch_log;
mod intents;
mod rows;
mod settings;
mod state;
mod tags;

pub(crate) use settings::SettingsTx;

use curio_types::EventEnvelope;
use rusqlite::{Connection, Transaction, TransactionBehavior};

use super::StorageError;
use crate::model::{ArticleId, FeedId};

/// Begins a write-intent transaction on the writer connection.
///
/// Always `IMMEDIATE`: every writer-thread transaction here intends to write,
/// and rusqlite's default `BEGIN DEFERRED` takes no lock until the first
/// statement — so a transaction that reads (e.g. a dedupe `SELECT`) before it
/// writes starts as a reader and, under concurrent readers in WAL mode, can
/// fail to upgrade with `SQLITE_BUSY_SNAPSHOT`, an error `busy_timeout` does
/// **not** retry. Taking the write lock up front removes that failure mode
/// entirely; it never contends here because a single writer thread owns the
/// only read-write connection. Every mutation routes through this so the
/// deferred read-then-write pattern can't creep back in.
fn begin_write(conn: &mut Connection) -> Result<Transaction<'_>, StorageError> {
    Ok(conn.transaction_with_behavior(TransactionBehavior::Immediate)?)
}

/// What a batch upsert did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UpsertOutcome {
    /// Articles that did not exist and were inserted.
    pub inserted: u64,
    /// Articles that existed (by dedupe key) and actually changed — a
    /// re-ingest of identical values writes nothing and is not counted.
    pub updated: u64,
}

/// Keyset-paginated article listing parameters.
///
/// Filters are backend-owned so heads never re-filter client-side: every
/// `Some` filter is `AND`ed into the query. An article with no
/// `article_state` row counts as unread/unstarred/not-read-later/
/// unarchived. The keyset order is fixed — `id DESC` (newest row first),
/// paginated by `before` — and filters never change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListArticles {
    /// Restrict to one feed; `None` lists across feeds.
    pub feed_id: Option<FeedId>,
    /// Keyset cursor: return articles with `id` strictly below this.
    pub before: Option<ArticleId>,
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
    /// Keep only articles carrying this tag (trimmed before matching,
    /// mirroring how tags are stored).
    pub tag: Option<String>,
    /// Keep only articles whose *feed* sits in this folder — i.e. the feed
    /// carries a `/`-path tag equal to this value or nested beneath it
    /// (`Tech` matches `Tech` and `Tech/Databases`). The folder-tree filter;
    /// distinct from `tag`, which matches an *article's* own tags.
    pub feed_tag: Option<String>,
}

impl Default for ListArticles {
    fn default() -> Self {
        Self {
            feed_id: None,
            before: None,
            limit: 50,
            read: None,
            starred: None,
            read_later: None,
            archived: None,
            tag: None,
            feed_tag: None,
        }
    }
}

/// A staged, not-yet-emitted event intent (write-ahead row for the JSONL
/// append).
#[derive(Debug, Clone, PartialEq)]
pub struct PendingIntent {
    /// Row id — pass back to
    /// [`Storage::mark_intents_emitted`](super::Storage::mark_intents_emitted).
    pub intent_id: i64,
    /// The full envelope to append, exactly as staged.
    pub envelope: EventEnvelope,
}

// The trailing two columns derive current health from `fetch_log`: the error
// text iff the *newest* attempt errored, and the timestamp of the newest
// successful attempt. Correlated on `feeds.id` — every FEED_COLS query is a
// bare `FROM feeds` (no aliases/joins), so the reference is unambiguous.
const FEED_COLS: &str = "id, url, title, site_url, description, etag, last_modified, status, \
                         allow_private_network, added_at, last_fetched_at, tags, fetch_full_text, \
                         (SELECT CASE WHEN status = 'error' THEN error END FROM fetch_log \
                          WHERE feed_id = feeds.id ORDER BY id DESC LIMIT 1) AS last_error, \
                         (SELECT fetched_at FROM fetch_log WHERE feed_id = feeds.id \
                          AND status IN ('ok', 'not_modified') ORDER BY id DESC LIMIT 1) \
                          AS last_ok_at";

const ARTICLE_COLS: &str = "id, curio_id, feed_id, dedupe_key, title, source_url, author, \
                            published_at, content_html, content_text, lang, word_count, \
                            saved_at, source_updated_at, lead_image";
