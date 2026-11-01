//! The blocking half of a refresh: map the fetch result onto feed state
//! (parse → sanitize → ingest on success, validators preserved on every
//! other path), schedule the feed's next attempt and settle the fetch-log
//! row with it.

use curio_types::Timestamp;

use crate::content;
use crate::feeds::ParsedEntry;
use crate::fetch::{FetchError, FetchResponse};
use crate::model::{
    ArticleContent, Feed, FeedId, FetchErrorKind, FetchRecord, FetchStatus, NewArticle,
};
use crate::storage::Storage;

use super::hydrate::Hydration;

use super::refresh::is_feed_own_url;
use super::schedule::{Attempt, RefreshPolicy};
use super::{CoreError, RefreshOutcome};

/// What the blocking half of a refresh settled.
pub(super) struct Settled {
    /// The outcome, already recorded in the fetch log.
    pub(super) outcome: RefreshOutcome,
    /// The rows the upsert *inserted* — the full-text hydration queue
    /// (empty unless the feed has full-text mode on).
    pub(super) hydrate_pending: Vec<Hydration>,
    /// A permanent redirect was adopted: feed events are staged.
    pub(super) url_adopted: bool,
}

/// The blocking half of a refresh: maps the fetch result onto feed state
/// (parse → ingest on success, validators preserved on every other path),
/// works out the feed's next attempt (see [`RefreshPolicy`]) and settles
/// the fetch-log row together with that schedule. Runs on the blocking
/// pool, inside the caller's per-feed locks.
pub(super) fn settle(
    storage: &Storage,
    policy: &RefreshPolicy,
    feed: &Feed,
    fetched: Result<FetchResponse, FetchError>,
    now: Timestamp,
    duration_ms: Option<u32>,
) -> Result<Settled, CoreError> {
    let id = feed.id;
    // Conditional-GET validators are preserved on every non-ingest
    // path — error, 304, 410, unparseable body. Only a successfully
    // *parsed* 2xx adopts the response's validators (the sketch
    // clobbered them on any failure).
    let mut hydrate_pending: Vec<Hydration> = Vec::new();
    let mut url_adopted = false;
    let parsed = match fetched {
        Ok(response) if response.is_success() => {
            match crate::feeds::parse_fetched_feed(&response) {
                Ok(parsed) => Ok((response, parsed)),
                // A body we could not parse must not update the
                // validators, or a broken feed 304s forever.
                Err(err) => {
                    let (kind, message) = unparseable(&response, &err);
                    Err((
                        failed(id, Some(response.status), kind, message),
                        Attempt::Failed {
                            kind,
                            retry_after: None,
                        },
                    ))
                }
            }
        }
        other => Err(unsuccessful(feed, other)),
    };
    let (outcome, attempt) = match parsed {
        Ok((response, parsed)) => {
            let attempt = Attempt::Fetched {
                feed_hint: parsed.meta.refresh_hint,
                max_age: response.max_age,
            };
            let ingested = ingest_parsed(storage, id, feed, &response, parsed, now)?;
            hydrate_pending = ingested.hydrate_pending;
            url_adopted = ingested.url_adopted;
            (ingested.outcome, attempt)
        }
        Err(unsuccessful) => {
            preserve_validators(storage, id, feed, now)?;
            unsuccessful
        }
    };

    // The schedule builds on the feed row as it stands when the attempt
    // settles, not on `feed` (read before the fetch): a pause or resume
    // made while the fetch was in flight is kept.
    let policy = *policy;
    storage.settle_fetch(
        FetchRecord {
            feed_id: id,
            fetched_at: now,
            status: outcome.status,
            http_status: outcome.http_status,
            error: outcome.error.clone(),
            error_kind: outcome.error_kind,
            articles_new: u32::try_from(outcome.new_articles).unwrap_or(u32::MAX),
            duration_ms,
        },
        move |current| policy.after(current, attempt, now),
    )?;
    Ok(Settled {
        outcome,
        hydrate_pending,
        url_adopted,
    })
}

/// An errored outcome.
fn failed(
    feed_id: FeedId,
    http_status: Option<u16>,
    kind: FetchErrorKind,
    error: String,
) -> RefreshOutcome {
    RefreshOutcome {
        feed_id,
        status: FetchStatus::Error,
        http_status,
        new_articles: 0,
        updated_articles: 0,
        error: Some(error),
        error_kind: Some(kind),
    }
}

/// Classifies a 2xx body that did not parse as a feed. A body that sniffs
/// as an HTML page is [`FetchErrorKind::NotAFeed`] — the subscription
/// points at a web page — and the message counts the feeds that page
/// declares (fetch-free discovery over the body already in hand), so the
/// user knows whether there is a real feed to switch to. Anything else is
/// a genuine [`FetchErrorKind::Parse`] failure.
fn unparseable(
    response: &crate::fetch::FetchResponse,
    err: &crate::feeds::FeedParseError,
) -> (FetchErrorKind, String) {
    let Some(feeds) = crate::feeds::discover::web_page_feeds(response) else {
        return (FetchErrorKind::Parse, err.to_string());
    };
    let message = match feeds.as_slice() {
        [] => "this is a web page, not a feed — found no feeds on it".to_owned(),
        [only] => format!(
            "this is a web page, not a feed — found 1 feed on it: {}",
            only.url
        ),
        [first, ..] => format!(
            "this is a web page, not a feed — found {} feeds on it (first: {})",
            feeds.len(),
            first.url
        ),
    };
    (FetchErrorKind::NotAFeed, message)
}

/// The outcome and schedule input of every fetch result but a 2xx: a
/// transport/policy error, 304, 410, a rate limit or another status.
fn unsuccessful(
    feed: &Feed,
    fetched: Result<FetchResponse, FetchError>,
) -> (RefreshOutcome, Attempt) {
    let id = feed.id;
    let failure = |kind, retry_after| Attempt::Failed { kind, retry_after };
    match fetched {
        Err(err) => {
            let kind = err.kind();
            (failed(id, None, kind, err.to_string()), failure(kind, None))
        }
        Ok(response) if response.is_not_modified() => (
            RefreshOutcome {
                feed_id: id,
                status: FetchStatus::NotModified,
                http_status: Some(response.status),
                new_articles: 0,
                updated_articles: 0,
                error: None,
                error_kind: None,
            },
            Attempt::NotModified {
                max_age: response.max_age,
            },
        ),
        Ok(response) if response.is_gone() => {
            // 410: gone for good — once a second 410 in a row confirms
            // it (the schedule marks the feed dead then).
            let confirmed =
                feed.consecutive_failures > 0 && feed.last_error_kind == Some(FetchErrorKind::Gone);
            let error = if confirmed {
                "HTTP 410 Gone again — feed marked dead"
            } else {
                "HTTP 410 Gone — the feed is marked dead if the next attempt says so too"
            };
            (
                failed(
                    id,
                    Some(response.status),
                    FetchErrorKind::Gone,
                    error.to_owned(),
                ),
                failure(FetchErrorKind::Gone, None),
            )
        }
        Ok(response) if response.is_rate_limited() => {
            let error = match response.retry_after {
                Some(wait) => format!(
                    "HTTP {} — the host is rate-limiting Curio (Retry-After: {} s)",
                    response.status,
                    wait.as_secs()
                ),
                None => format!("HTTP {} — the host is rate-limiting Curio", response.status),
            };
            (
                failed(
                    id,
                    Some(response.status),
                    FetchErrorKind::RateLimited,
                    error,
                ),
                failure(FetchErrorKind::RateLimited, response.retry_after),
            )
        }
        Ok(response) => (
            failed(
                id,
                Some(response.status),
                FetchErrorKind::Http,
                format!("HTTP {}", response.status),
            ),
            failure(FetchErrorKind::Http, response.retry_after),
        ),
    }
}

/// Stamps `last_fetched_at` while keeping the stored conditional-GET
/// validators exactly as they were.
fn preserve_validators(
    storage: &Storage,
    id: FeedId,
    feed: &Feed,
    now: Timestamp,
) -> Result<(), CoreError> {
    storage.update_feed_fetch_state(id, feed.etag.clone(), feed.last_modified.clone(), now)?;
    Ok(())
}

/// The happy path of a refresh: pipeline every entry, batch-upsert,
/// fill in feed metadata, adopt permanent redirects and the response
/// validators. The settled outcome is not recorded yet — [`settle`]
/// does that.
fn ingest_parsed(
    storage: &Storage,
    id: FeedId,
    feed: &Feed,
    response: &FetchResponse,
    mut parsed: crate::feeds::ParsedFeed,
    now: Timestamp,
) -> Result<Settled, CoreError> {
    // A reused guid's keys are chosen per fetch; settle them against
    // what earlier fetches stored so no item changes key (and splits
    // into a duplicate article) when the guid's sharing changes.
    let stored = storage.source_urls_by_dedupe_key(Some(id), parsed.settle_lookups())?;
    parsed.settle_keys(&feed.url, |key| stored.get(key).map(String::as_str));
    let articles: Vec<NewArticle> = parsed
        .entries
        .into_iter()
        .map(|entry| ingest_entry(id, &feed.url, entry))
        .collect();
    // Which entries are new, decided BEFORE the upsert (afterwards
    // everything exists). Same-feed refreshes are serialized by the
    // caller's per-feed lock, so this read-then-write cannot race. A
    // link-less entry has no page of its own (its source URL is the
    // feed's — see `ingest_entry`), so it is never queued: hydrating it
    // would fetch the feed XML as if it were an article.
    let fresh: Vec<(String, String)> = if feed.fetch_full_text {
        articles
            .iter()
            .filter(|article| !is_feed_own_url(&article.source_url, feed))
            .filter(|article| {
                storage
                    .article_id_by_dedupe_key(Some(id), &article.dedupe_key)
                    .ok()
                    .flatten()
                    .is_none()
            })
            .map(|article| (article.dedupe_key.clone(), article.source_url.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let upserted = storage.upsert_articles(articles)?;
    let mut hydrate_pending = Vec::with_capacity(fresh.len());
    for (key, url) in fresh {
        if let Some(article_id) = storage.article_id_by_dedupe_key(Some(id), &key)? {
            hydrate_pending.push(Hydration {
                feed_id: id,
                article_id,
                url,
            });
        }
    }
    storage.update_feed_metadata(
        id,
        parsed.meta.title,
        parsed.meta.site_url,
        parsed.meta.description,
    )?;
    let url_adopted = response.permanent_redirect && response.final_url != feed.url;
    if url_adopted {
        // The URL is the identity key of feed.added/feed.removed;
        // adoption stages a removed/added pair (the caller flushes it).
        storage.update_feed_url(id, response.final_url.clone())?;
    }
    storage.update_feed_fetch_state(
        id,
        response.etag.clone(),
        response.last_modified.clone(),
        now,
    )?;
    Ok(Settled {
        outcome: RefreshOutcome {
            feed_id: id,
            status: FetchStatus::Ok,
            http_status: Some(response.status),
            new_articles: upserted.inserted,
            updated_articles: upserted.updated,
            error: None,
            error_kind: None,
        },
        hydrate_pending,
        url_adopted,
    })
}

/// Turns a parsed entry into a storage row via the content pipeline
/// (sanitize → extract whole pages only → text — the D6 invariant lives
/// here).
///
/// An entry without a link of its own gets the feed URL as `source_url`:
/// the published contracts require every article to carry a source URI,
/// and the feed is the only one it has. That value marks the article as
/// link-less — full-text hydration recognizes it and never fetches it.
fn ingest_entry(feed_id: FeedId, feed_url: &str, entry: ParsedEntry) -> NewArticle {
    let base = entry.link.as_deref().unwrap_or(feed_url).to_owned();
    let processed = content::process(&entry.content_html, Some(&base));
    // Prefer the feed's own declared image (media:thumbnail / media:content /
    // enclosure); fall back to the first inline <img> in the sanitized body
    // (already base-resolved). Either way it's a URL loaded through the
    // policed image cache, never fetched directly.
    let lead_image = entry
        .lead_image
        .or_else(|| content::first_image(&processed.html));
    NewArticle {
        feed_id: Some(feed_id),
        dedupe_key: entry.dedupe_key,
        title: entry.title,
        source_url: entry.link.unwrap_or(base),
        author: entry.author,
        published_at: entry.published,
        content: ArticleContent {
            html: processed.html,
            text: processed.text,
        },
        lang: entry.lang,
        word_count: Some(processed.word_count),
        source_updated_at: entry.updated,
        lead_image,
    }
}
