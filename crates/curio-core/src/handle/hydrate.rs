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

use futures_util::StreamExt;
use futures_util::stream::FuturesUnordered;

use crate::fetch::PolicedClient;
use crate::model::{ArticleId, FeedId};

use super::sweep::RefreshCancel;
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

/// Pending hydrations, handed out fairly across page hosts, and a count
/// of how far they have come.
#[derive(Debug, Default)]
pub(super) struct HydrationQueue {
    queue: WorkQueue<Hydration>,
    /// Pages queued so far.
    queued: usize,
    /// Pages finished so far: loaded, failed, cancelled, or dropped
    /// with a feed blocked upstream.
    finished: usize,
    /// Pages a cancel stopped while they were fetched.
    cancelled: usize,
}

impl HydrationQueue {
    /// Queues `pending`, each in its page host's lane.
    pub(super) fn extend(&mut self, client: &PolicedClient, pending: Vec<Hydration>) {
        self.queued += pending.len();
        for hydration in pending {
            let host = client.paced_host(&hydration.url);
            self.queue.push(hydration, host);
        }
    }

    /// The next hydration to start. `None` when every waiting page's
    /// host is busy, or none is left.
    pub(super) fn take(&mut self) -> Option<Work<Hydration>> {
        self.queue.take()
    }

    /// A hydration finished: its host may take the next page, and one
    /// blocked upstream drops its feed's queued pages (they keep their
    /// feed excerpts, and a later manual load-full picks them up). A
    /// feed's pages are all queued at once, as its refresh settles, so
    /// none of them can arrive after the drop.
    pub(super) fn settle(&mut self, work: &Work<Hydration>, hydrated: Hydrated) {
        self.queue.release(work.lane);
        self.finished += 1;
        match hydrated {
            Hydrated::Done => {}
            Hydrated::Blocked => {
                let feed = work.job.feed_id;
                self.finished += self.queue.retain(|queued| queued.feed_id != feed);
            }
            Hydrated::Cancelled => self.cancelled += 1,
        }
    }

    /// Pages finished so far (see [`Self::settle`]).
    pub(super) fn finished(&self) -> usize {
        self.finished
    }

    /// Pages queued so far.
    pub(super) fn queued(&self) -> usize {
        self.queued
    }

    /// Whether a cancel left pages unloaded: one dropped while it was
    /// fetched, or one that never started.
    pub(super) fn cut_short(&self) -> bool {
        self.cancelled > 0 || self.queue.waiting() > 0
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

    /// Runs one queued hydration — stopped at its page fetch once
    /// `cancel` fires. Only the fetches race the cancel: a page whose
    /// fetch is in is extracted and stored before the hydration ends, so
    /// no write lands after the sweep has returned.
    pub(super) async fn hydrate_queued(
        &self,
        work: Work<Hydration>,
        cancel: Option<&RefreshCancel>,
    ) -> (Work<Hydration>, Hydrated) {
        let article = work.job.article_id;
        let hydrated = match self.hydrate_article_until(article, cancel).await {
            Ok(None) => Hydrated::Cancelled,
            Ok(Some(_)) => Hydrated::Done,
            Err(CoreError::RateLimited { .. } | CoreError::RedditAuth { .. }) => {
                tracing::warn!(
                    feed = %work.job.feed_id,
                    "full-text hydrate blocked upstream; dropping the feed's queued pages"
                );
                Hydrated::Blocked
            }
            Err(err) => {
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
        assert_eq!((queue.finished(), queue.queued()), (0, 4));
        queue.settle(&first, Hydrated::Blocked);
        assert_eq!(queue.finished(), 2, "the dropped page counts as finished");
        let third = queue.take().unwrap();
        assert_eq!(third.job.article_id, ArticleId(21));
        queue.settle(&third, Hydrated::Done);
        assert!(!queue.cut_short(), "nothing was cancelled");
        queue.settle(&second, Hydrated::Cancelled);
        assert!(queue.take().is_none(), "nothing is left");
        assert_eq!((queue.finished(), queue.queued()), (4, 4));
        assert!(queue.cut_short(), "a page was cancelled mid-fetch");
    }

    /// A page a cancel never let start leaves the queue cut short too.
    #[test]
    fn pages_left_waiting_cut_the_queue_short() {
        let client = PolicedClient::new(FetchConfig::default());
        let mut queue = HydrationQueue::default();
        queue.extend(&client, vec![hydration(1, 10, "https://a.example/1")]);
        assert!(queue.cut_short());
        let work = queue.take().unwrap();
        assert!(!queue.cut_short(), "the page is in flight, not waiting");
        queue.settle(&work, Hydrated::Done);
        assert!(!queue.cut_short());
    }
}
