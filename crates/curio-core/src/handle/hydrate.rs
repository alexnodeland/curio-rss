//! The full-text hydration queue: the articles a full-text feed's refresh
//! inserted, loaded from their source pages at bounded concurrency.
//!
//! Hydration is a queue of its own, not part of a feed's refresh: a sweep
//! hands a refreshed feed's new articles to the queue and moves the feed's
//! slot on to the next feed, so a full-text feed with many new items never
//! holds a feed slot while its pages load. The queue shares its slots
//! fairly between page hosts, like the sweep's feed list: a
//! politeness-paced host has at most one page in flight. Each hydration is
//! best-effort — a dead page keeps its feed excerpt, and
//! [`CoreHandle::hydrate_article`]'s never-clobber guard protects feeds
//! that ship full content — and its extraction and storage writes run on
//! the blocking pool, off the async runtime.

use std::collections::HashSet;

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;

use crate::fetch::PolicedClient;
use crate::model::{ArticleId, FeedId};

use super::sweep::{RefreshCancel, until_cancelled};
use super::work_queue::{Work, WorkQueue};
use super::{CoreError, CoreHandle};

/// How many article pages are hydrated at once unless told otherwise.
pub const DEFAULT_HYDRATE_CONCURRENCY: usize = 4;

/// One article queued for full-text hydration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Hydration {
    /// The feed whose refresh inserted it.
    pub(super) feed_id: FeedId,
    /// The article to hydrate.
    pub(super) article_id: ArticleId,
    /// Its source page — which host lane it queues in.
    pub(super) url: String,
}

/// What one hydration came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Hydrated {
    /// Hydrated, or failed in a way that concerns this article alone.
    Done,
    /// Blocked upstream (a rate limit, rejected API credentials): every
    /// other queued article of the feed would fail the same way.
    Blocked,
    /// The sweep was cancelled while its page was fetched.
    Cancelled,
}

/// Pending hydrations, handed out fairly across page hosts.
#[derive(Debug, Default)]
pub(super) struct HydrationQueue {
    queue: WorkQueue<Hydration>,
    /// Feeds whose hydrations are blocked upstream — their queued
    /// articles are dropped (they keep their feed excerpts, and a later
    /// manual load-full picks them up).
    blocked: HashSet<FeedId>,
}

impl HydrationQueue {
    /// Queues `pending`, each in its page host's lane.
    pub(super) fn extend(&mut self, client: &PolicedClient, pending: Vec<Hydration>) {
        for hydration in pending {
            let host = client.paced_host(&hydration.url);
            self.queue.push(hydration, host);
        }
    }

    /// The next hydration to start, skipping blocked feeds' articles.
    /// `None` when every waiting page's host is busy, or none is left.
    pub(super) fn take(&mut self) -> Option<Work<Hydration>> {
        loop {
            let work = self.queue.take()?;
            if !self.blocked.contains(&work.job.feed_id) {
                return Some(work);
            }
            self.queue.release(work.lane);
        }
    }

    /// A hydration finished: its host may take the next page, and a
    /// blocked one drops its feed's queued articles.
    pub(super) fn settle(&mut self, work: &Work<Hydration>, hydrated: Hydrated) {
        self.queue.release(work.lane);
        if hydrated == Hydrated::Blocked {
            self.blocked.insert(work.job.feed_id);
        }
    }
}

impl CoreHandle {
    /// Hydrates `pending` to the end, up to `concurrency` pages at once
    /// (`0` is treated as `1`) — the single-feed refresh's hydration.
    pub(super) async fn hydrate_all(&self, pending: Vec<Hydration>, concurrency: usize) {
        let mut queue = HydrationQueue::default();
        queue.extend(&self.client, pending);
        let mut in_flight = FuturesUnordered::new();
        loop {
            while in_flight.len() < concurrency.max(1) {
                let Some(work) = queue.take() else { break };
                in_flight.push(self.hydrate_queued(work, None));
            }
            let Some((work, hydrated)) = in_flight.next().await else {
                break;
            };
            queue.settle(&work, hydrated);
        }
    }

    /// Runs one queued hydration — dropped at its page fetch once
    /// `cancel` fires (a hydrate writes nothing before its fetch is in).
    pub(super) async fn hydrate_queued(
        &self,
        work: Work<Hydration>,
        cancel: Option<&RefreshCancel>,
    ) -> (Work<Hydration>, Hydrated) {
        let article = work.job.article_id;
        let hydrated = match until_cancelled(cancel, self.hydrate_article(article)).await {
            None => Hydrated::Cancelled,
            Some(Ok(_)) => Hydrated::Done,
            Some(Err(CoreError::RateLimited { .. } | CoreError::RedditAuth { .. })) => {
                tracing::warn!(
                    feed = %work.job.feed_id,
                    "full-text hydrate blocked upstream; dropping the feed's queued pages"
                );
                Hydrated::Blocked
            }
            Some(Err(err)) => {
                tracing::warn!(
                    %article,
                    %err,
                    "full-text hydrate failed; keeping the feed excerpt"
                );
                Hydrated::Done
            }
        };
        (work, hydrated)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::Duration;

    use crate::fetch::FetchConfig;

    use super::*;

    fn hydration(feed: i64, article: i64, url: &str) -> Hydration {
        Hydration {
            feed_id: FeedId(feed),
            article_id: ArticleId(article),
            url: url.to_owned(),
        }
    }

    /// Pages queue by host: a paced host has one page out at a time, and
    /// a feed blocked upstream has its other queued pages dropped.
    #[test]
    fn the_queue_paces_hosts_and_drops_a_blocked_feeds_pages() {
        let client = PolicedClient::new(FetchConfig {
            politeness_delay: Duration::from_millis(500),
            ..FetchConfig::default()
        });
        let mut queue = HydrationQueue::default();
        queue.extend(
            &client,
            vec![
                hydration(1, 10, "https://a.example/1"),
                hydration(1, 11, "https://a.example/2"),
                hydration(2, 20, "https://b.example/1"),
                hydration(2, 21, "https://a.example/3"),
            ],
        );
        let first = queue.take().unwrap();
        assert_eq!(first.job.article_id, ArticleId(10));
        let second = queue.take().unwrap();
        assert_eq!(second.job.article_id, ArticleId(20), "a.example is busy");
        assert!(queue.take().is_none(), "both hosts are busy");

        // Feed 1 is blocked upstream: its other page is dropped, feed 2's
        // page on the same host goes next.
        queue.settle(&first, Hydrated::Blocked);
        let third = queue.take().unwrap();
        assert_eq!(third.job.article_id, ArticleId(21));
        queue.settle(&third, Hydrated::Done);
        queue.settle(&second, Hydrated::Cancelled);
        assert!(queue.take().is_none(), "nothing is left");
    }
}
