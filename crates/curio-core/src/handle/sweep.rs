//! Refresh sweeps: many feeds at bounded concurrency, one sweep in flight
//! per handle, cancellable, with per-feed progress.
//!
//! A sweep is the only multi-feed refresh path — both heads call it. Up to
//! [`RefreshOptions::concurrency`] feeds are in flight at once, so a few
//! hung hosts cost about one request timeout between them, not one each.
//! The slots are shared fairly between hosts: a politeness-paced host (the
//! default delay paces every host) has at most one feed in flight — a
//! second would only sleep out the host's delay in a slot another host
//! could use — and the hosts take turns at the free slots, so a long run
//! of one host's feeds in the list never holds the others up. A per-feed
//! `Err` (the feed was removed mid-sweep, a transient storage error, a
//! panicking parse) becomes that feed's outcome; it never aborts the sweep.

use std::collections::{HashMap, VecDeque};
use std::convert::Infallible;
use std::future::Future;
use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::StreamExt;
use futures_util::future::{Either, select};
use futures_util::stream::FuturesUnordered;
use tokio::sync::{Notify, watch};

use crate::model::{Feed, FeedId, FeedStatus, FetchStatus};

use super::{CoreError, CoreHandle, RefreshOutcome};

/// How many feeds a sweep refreshes at once unless told otherwise.
pub const DEFAULT_REFRESH_CONCURRENCY: usize = 8;

/// Options for a refresh sweep.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshOptions {
    /// Feeds in flight at once; `0` is treated as `1`.
    pub concurrency: usize,
}

impl Default for RefreshOptions {
    fn default() -> Self {
        Self {
            concurrency: DEFAULT_REFRESH_CONCURRENCY,
        }
    }
}

/// One feed of a sweep finished — handed to the sweep's progress callback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepProgress<'a> {
    /// The sweep this progress belongs to.
    pub sweep_id: u64,
    /// Feeds finished so far, this one included. A feed whose fetch a
    /// cancel abandoned never finishes, so a cancelled sweep stops short
    /// of `total`.
    pub done: usize,
    /// Feeds the sweep set out to refresh.
    pub total: usize,
    /// The finished feed's outcome.
    pub outcome: &'a RefreshOutcome,
}

/// What a whole sweep did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// This handle's sweep counter (starts at 1). A joined request reports
    /// the id of the sweep it joined.
    pub sweep_id: u64,
    /// Feeds the sweep set out to refresh.
    pub total: usize,
    /// One outcome per feed the sweep refreshed, in the requested order.
    /// A cancelled sweep has none for the feeds it never started or whose
    /// fetch it abandoned — those were not refreshed, they did not fail.
    pub outcomes: Vec<RefreshOutcome>,
    /// Whether [`CoreHandle::cancel_refresh`] stopped the sweep short of
    /// some feed (a cancel landing after the last feed finished stops
    /// nothing, and leaves this `false`).
    pub cancelled: bool,
    /// Whether this request found a sweep already running and waited for
    /// it instead of starting another. The report is that sweep's: its
    /// feed list (not necessarily this request's) and its outcomes; this
    /// request's progress callback was never called.
    pub joined: bool,
}

impl CoreHandle {
    /// Refreshes every active feed at the default concurrency. Per-feed
    /// failures land in their outcome rows. Joins a sweep already in
    /// flight (see [`Self::refresh_many`]).
    ///
    /// # Errors
    ///
    /// Storage errors listing the feeds only.
    pub async fn refresh_all(&self) -> Result<Vec<RefreshOutcome>, CoreError> {
        Ok(self
            .refresh_all_with(RefreshOptions::default(), |_| {})
            .await?
            .outcomes)
    }

    /// Refreshes every active feed as one sweep — see
    /// [`Self::refresh_many`]. The feeds are listed once this request
    /// owns the sweep slot; a request that joins a running sweep lists
    /// nothing.
    ///
    /// # Errors
    ///
    /// Storage errors listing the feeds only.
    pub async fn refresh_all_with(
        &self,
        options: RefreshOptions,
        on_progress: impl FnMut(SweepProgress<'_>),
    ) -> Result<SweepReport, CoreError> {
        self.sweep(options, on_progress, |feeds| {
            Ok(feeds?
                .into_iter()
                .filter(|feed| feed.status == FeedStatus::Active)
                .map(|feed| (feed.id, Some(feed.url)))
                .collect())
        })
        .await
    }

    /// Refreshes `ids` as one sweep: up to `options.concurrency` feeds in
    /// flight at once (at most one per politeness-paced host — see the
    /// module docs), `on_progress` called as each one finishes (in
    /// completion order, with a running `done`/`total`), and every
    /// per-feed `Err` turned into an [`FetchStatus::Error`] outcome so
    /// the sweep always runs to the end.
    ///
    /// **One sweep at a time.** A request made while another sweep is in
    /// flight joins it: it waits for that sweep and returns its report
    /// with [`SweepReport::joined`] set — no feed is fetched twice. The
    /// joined report covers the *running* sweep's feeds, so any of `ids`
    /// that sweep does not include are not refreshed by this call; ask
    /// again once it is over. (If the running sweep's caller gives up on
    /// it, a joined request starts a sweep of its own.)
    ///
    /// [`Self::cancel_refresh`] stops the sweep: no further feed starts,
    /// fetches in flight (feed and full-text page fetches alike) are
    /// dropped, and the report comes back with [`SweepReport::cancelled`]
    /// set and no outcomes for the feeds that were not refreshed.
    pub async fn refresh_many(
        &self,
        ids: Vec<FeedId>,
        options: RefreshOptions,
        on_progress: impl FnMut(SweepProgress<'_>),
    ) -> SweepReport {
        let Ok(report) = self
            .sweep(options, on_progress, |feeds| -> Result<_, Infallible> {
                // The URLs only pick each feed's host lane; without them
                // the sweep still runs, just without the per-host spread.
                let urls: HashMap<FeedId, String> = match feeds {
                    Ok(feeds) => feeds.into_iter().map(|feed| (feed.id, feed.url)).collect(),
                    Err(err) => {
                        tracing::warn!(%err, "listing feeds failed; sweeping without host lanes");
                        HashMap::new()
                    }
                };
                Ok(ids
                    .into_iter()
                    .map(|id| (id, urls.get(&id).cloned()))
                    .collect())
            })
            .await;
        report
    }

    /// Cancels the sweep in flight, if any: no further feed starts and
    /// fetches in flight are dropped. `false` when no sweep is running.
    pub fn cancel_refresh(&self) -> bool {
        let active = self
            .sweep
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        active.as_ref().is_some_and(|sweep| {
            sweep.cancel.cancel();
            true
        })
    }

    /// Claims the sweep slot (or joins the sweep holding it), then runs
    /// the feeds `targets` picks from the feed list — each with its URL,
    /// when known, for its host lane.
    async fn sweep<E>(
        &self,
        options: RefreshOptions,
        mut on_progress: impl FnMut(SweepProgress<'_>),
        targets: impl FnOnce(Result<Vec<Feed>, CoreError>) -> Result<Vec<(FeedId, Option<String>)>, E>,
    ) -> Result<SweepReport, E> {
        let run = loop {
            match self.sweep.claim() {
                Claim::Join(mut running) => {
                    // A sender dropped without a report: that sweep's
                    // caller gave up on it — claim again.
                    if let Ok(report) = running.wait_for(Option::is_some).await
                        && let Some(report) = report.as_ref()
                    {
                        return Ok(SweepReport {
                            joined: true,
                            ..SweepReport::clone(report)
                        });
                    }
                }
                Claim::Run(run) => break run,
            }
        };
        let feeds = self
            .on_blocking_pool(|storage| Ok(storage.list_feeds()?))
            .await;
        // An `Err` drops `run`, which frees the slot for any joiner.
        let targets = targets(feeds)?;
        let report = self
            .run_sweep(run.id, &run.cancel, targets, options, &mut on_progress)
            .await;
        run.finish(&report);
        Ok(report)
    }

    /// The body of a claimed sweep.
    async fn run_sweep(
        &self,
        sweep_id: u64,
        cancel: &RefreshCancel,
        targets: Vec<(FeedId, Option<String>)>,
        options: RefreshOptions,
        on_progress: &mut impl FnMut(SweepProgress<'_>),
    ) -> SweepReport {
        let total = targets.len();
        let mut slots: Vec<Option<RefreshOutcome>> = (0..total).map(|_| None).collect();
        let mut queue =
            WorkQueue::new(targets.into_iter().enumerate().map(|(index, (id, url))| {
                let host = url.and_then(|url| self.client.paced_host(&url));
                (index, id, host)
            }));
        let refresh = |work: Work| async move {
            let outcome = match self.refresh_feed_until(work.id, Some(cancel)).await {
                Ok(outcome) => outcome,
                Err(err) => {
                    tracing::warn!(feed = %work.id, %err, "feed refresh failed; the sweep carries on");
                    Some(errored(work.id, &err))
                }
            };
            (work, outcome)
        };
        let concurrency = options.concurrency.max(1);
        let mut in_flight = FuturesUnordered::new();
        let mut done = 0;
        loop {
            // A cancelled sweep starts nothing new. `take` comes back
            // empty while every waiting feed's host is busy; a feed
            // finishing frees its host for the next round.
            while in_flight.len() < concurrency && !cancel.is_cancelled() {
                let Some(work) = queue.take() else { break };
                in_flight.push(refresh(work));
            }
            let Some((work, outcome)) = in_flight.next().await else {
                break;
            };
            queue.release(work.lane);
            // A fetch the cancel abandoned has no outcome: that feed was
            // not refreshed, like the ones never started.
            let Some(outcome) = outcome else { continue };
            done += 1;
            on_progress(SweepProgress {
                sweep_id,
                done,
                total,
                outcome: &outcome,
            });
            if let Some(slot) = slots.get_mut(work.index) {
                *slot = Some(outcome);
            }
        }
        let outcomes: Vec<RefreshOutcome> = slots.into_iter().flatten().collect();
        SweepReport {
            sweep_id,
            total,
            // Only a cancel leaves a feed without an outcome.
            cancelled: outcomes.len() < total,
            outcomes,
            joined: false,
        }
    }
}

/// A per-feed `Err` as that feed's outcome.
fn errored(feed_id: FeedId, err: &CoreError) -> RefreshOutcome {
    let error = match err {
        CoreError::NotFound { .. } => "the feed was removed during the refresh".to_owned(),
        other => other.to_string(),
    };
    RefreshOutcome {
        feed_id,
        status: FetchStatus::Error,
        http_status: None,
        new_articles: 0,
        updated_articles: 0,
        error: Some(error),
    }
}

/// One feed of a sweep's work list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Work {
    /// Its position in the requested order.
    index: usize,
    id: FeedId,
    /// Its host lane in the [`WorkQueue`].
    lane: usize,
}

/// A sweep's work list, handed out fairly across hosts. Each
/// politeness-paced host gets a lane with at most one feed in flight; the
/// feeds of unpaced hosts (and of feeds whose URL is unknown) share one
/// lane without that cap. The lanes take turns, each keeping its feeds in
/// the requested order.
#[derive(Debug)]
struct WorkQueue {
    lanes: Vec<Lane>,
    /// The lane the next [`WorkQueue::take`] looks at first.
    turn: usize,
}

#[derive(Debug)]
struct Lane {
    pending: VecDeque<Work>,
    /// One feed in flight at most (a paced host's lane).
    capped: bool,
    /// A capped lane's feed is in flight.
    busy: bool,
}

impl WorkQueue {
    /// Lanes in the order their hosts first appear; `host` is the feed's
    /// paced host, `None` for an unpaced or unknown one.
    fn new(feeds: impl IntoIterator<Item = (usize, FeedId, Option<String>)>) -> Self {
        let mut lanes: Vec<Lane> = Vec::new();
        let mut by_host: HashMap<Option<String>, usize> = HashMap::new();
        for (index, id, host) in feeds {
            let capped = host.is_some();
            let lane = *by_host.entry(host).or_insert_with(|| {
                lanes.push(Lane {
                    pending: VecDeque::new(),
                    capped,
                    busy: false,
                });
                lanes.len() - 1
            });
            if let Some(entry) = lanes.get_mut(lane) {
                entry.pending.push_back(Work { index, id, lane });
            }
        }
        Self { lanes, turn: 0 }
    }

    /// The next feed to start: the first lane, from this turn on, with a
    /// feed waiting and room in flight. `None` when every waiting feed's
    /// host already has one in flight, or no feed is left.
    fn take(&mut self) -> Option<Work> {
        let count = self.lanes.len();
        for step in 0..count {
            let at = (self.turn + step) % count;
            let Some(lane) = self.lanes.get_mut(at) else {
                continue;
            };
            if lane.busy {
                continue;
            }
            if let Some(work) = lane.pending.pop_front() {
                lane.busy = lane.capped;
                self.turn = (at + 1) % count;
                return Some(work);
            }
        }
        None
    }

    /// A feed of `lane` finished: its host may take the next one.
    fn release(&mut self, lane: usize) {
        if let Some(lane) = self.lanes.get_mut(lane) {
            lane.busy = false;
        }
    }
}

/// Runs `work` to the end — or, once `cancel` fires, drops it at the
/// await it is parked on and returns `None`.
pub(super) async fn until_cancelled<T>(
    cancel: Option<&RefreshCancel>,
    work: impl Future<Output = T>,
) -> Option<T> {
    let Some(cancel) = cancel else {
        return Some(work.await);
    };
    match select(pin!(work), pin!(cancel.cancelled())).await {
        Either::Left((done, _)) => Some(done),
        Either::Right(((), _)) => None,
    }
}

/// The handle's one sweep slot.
#[derive(Debug, Default)]
pub(super) struct SweepSlot {
    active: Mutex<Option<ActiveSweep>>,
    last_id: AtomicU64,
}

/// The sweep in flight: what a joiner waits on and what cancel fires.
#[derive(Debug)]
struct ActiveSweep {
    id: u64,
    cancel: RefreshCancel,
    report: watch::Receiver<Option<Arc<SweepReport>>>,
}

/// The result of asking for the sweep slot.
enum Claim<'a> {
    /// A sweep is running — wait for its report.
    Join(watch::Receiver<Option<Arc<SweepReport>>>),
    /// The slot is ours.
    Run(RunningSweep<'a>),
}

impl SweepSlot {
    fn claim(&self) -> Claim<'_> {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(sweep) = active.as_ref() {
            return Claim::Join(sweep.report.clone());
        }
        let id = self.last_id.fetch_add(1, Ordering::Relaxed) + 1;
        let cancel = RefreshCancel::default();
        let (report, receiver) = watch::channel(None);
        *active = Some(ActiveSweep {
            id,
            cancel: cancel.clone(),
            report: receiver,
        });
        Claim::Run(RunningSweep {
            slot: self,
            id,
            cancel,
            report,
        })
    }

    /// Frees the slot if sweep `id` still holds it.
    fn free(&self, id: u64) {
        let mut active = self.active.lock().unwrap_or_else(PoisonError::into_inner);
        if active.as_ref().is_some_and(|sweep| sweep.id == id) {
            *active = None;
        }
    }
}

/// Ownership of the sweep slot. Dropping it frees the slot — on success
/// and when the sweep's future is dropped half-way — *before* the report
/// sender goes, so a joiner woken by a sender dropped without a report
/// finds the slot free when it claims again.
struct RunningSweep<'a> {
    slot: &'a SweepSlot,
    id: u64,
    cancel: RefreshCancel,
    report: watch::Sender<Option<Arc<SweepReport>>>,
}

impl RunningSweep<'_> {
    /// Frees the slot, then publishes the report to every joiner — in that
    /// order, so a request arriving after the sweep is over starts a sweep
    /// of its own rather than joining a finished one (and a late
    /// [`CoreHandle::cancel_refresh`] finds nothing to cancel).
    fn finish(self, report: &SweepReport) {
        self.slot.free(self.id);
        self.report.send_replace(Some(Arc::new(report.clone())));
    }
}

impl Drop for RunningSweep<'_> {
    fn drop(&mut self) {
        self.slot.free(self.id);
    }
}

/// A sweep's cancellation signal: a sticky flag plus a wake-up for the
/// fetches racing it.
#[derive(Debug, Clone, Default)]
pub(super) struct RefreshCancel(Arc<CancelState>);

#[derive(Debug, Default)]
struct CancelState {
    fired: AtomicBool,
    notify: Notify,
}

impl RefreshCancel {
    /// Fires the signal (idempotent) and wakes every waiter.
    pub(super) fn cancel(&self) {
        self.0.fired.store(true, Ordering::SeqCst);
        self.0.notify.notify_waiters();
    }

    /// Whether the signal has fired.
    pub(super) fn is_cancelled(&self) -> bool {
        self.0.fired.load(Ordering::SeqCst)
    }

    /// Resolves once the signal fires (at once if it already has).
    pub(super) async fn cancelled(&self) {
        loop {
            let mut notified = pin!(self.0.notify.notified());
            // Registered before the flag check, so a cancel landing in
            // between still wakes this waiter.
            notified.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::time::Duration;

    use super::*;

    #[tokio::test]
    async fn a_cancel_wakes_a_waiter_and_stays_fired() {
        let cancel = RefreshCancel::default();
        assert!(!cancel.is_cancelled());
        let waiter = {
            let cancel = cancel.clone();
            tokio::spawn(async move { cancel.cancelled().await })
        };
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
        tokio::time::timeout(Duration::from_secs(5), waiter)
            .await
            .unwrap()
            .unwrap();
        // Sticky: a later waiter resolves at once.
        assert!(cancel.is_cancelled());
        tokio::time::timeout(Duration::from_secs(1), cancel.cancelled())
            .await
            .unwrap();
    }

    /// Work raced against a cancel: it finishes when nothing cancels it,
    /// and is dropped where it is parked once the cancel fires.
    #[tokio::test]
    async fn work_runs_to_the_end_unless_cancelled() {
        assert_eq!(until_cancelled(None, async { 7 }).await, Some(7));
        let cancel = RefreshCancel::default();
        assert_eq!(until_cancelled(Some(&cancel), async { 7 }).await, Some(7));
        let parked = until_cancelled(Some(&cancel), std::future::pending::<()>());
        let fire = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            cancel.cancel();
        };
        let (parked, ()) =
            tokio::time::timeout(Duration::from_secs(5), async { tokio::join!(parked, fire) })
                .await
                .unwrap();
        assert_eq!(parked, None);
    }

    #[test]
    fn the_slot_admits_one_sweep_and_frees_itself_when_dropped() {
        let slot = SweepSlot::default();
        let Claim::Run(first) = slot.claim() else {
            unreachable!("an empty slot is claimed");
        };
        assert_eq!(first.id, 1);
        let Claim::Join(joined) = slot.claim() else {
            unreachable!("a busy slot is joined");
        };
        // Dropped without a report (the owner gave up): the slot is free
        // again and the joiner sees the sender gone.
        drop(first);
        assert!(joined.has_changed().is_err(), "the sender is gone");
        let Claim::Run(second) = slot.claim() else {
            unreachable!("a freed slot is claimed again");
        };
        assert_eq!(second.id, 2, "ids keep counting");
    }

    /// A finished sweep's slot is free by the time its report is out: a
    /// joiner still gets the report, but nothing new joins the finished
    /// sweep — the next claim runs a sweep of its own.
    #[test]
    fn a_finished_sweep_frees_the_slot_before_it_reports() {
        let slot = SweepSlot::default();
        let Claim::Run(first) = slot.claim() else {
            unreachable!("an empty slot is claimed");
        };
        let Claim::Join(joined) = slot.claim() else {
            unreachable!("a busy slot is joined");
        };
        let first_id = first.id;
        let report = SweepReport {
            sweep_id: first_id,
            total: 0,
            outcomes: Vec::new(),
            cancelled: false,
            joined: false,
        };
        first.finish(&report);
        assert_eq!(joined.borrow().as_deref(), Some(&report));
        let Claim::Run(second) = slot.claim() else {
            unreachable!("a finished sweep's slot is free");
        };
        // Freeing under a stale id leaves the new sweep's slot alone.
        slot.free(first_id);
        assert!(matches!(slot.claim(), Claim::Join(_)));
        drop(second);
        assert!(matches!(slot.claim(), Claim::Run(_)));
    }

    fn ids(works: &[Work]) -> Vec<i64> {
        works.iter().map(|work| work.id.0).collect()
    }

    /// One paced host's long run of feeds does not crowd the others out:
    /// the hosts take turns, a paced host never has two feeds out at once,
    /// and the unpaced lane has no such cap.
    #[test]
    fn the_work_queue_takes_turns_between_hosts() {
        let host = |name: &str| Some(name.to_owned());
        let feeds = [
            (0, FeedId(0), host("reddit.com")),
            (1, FeedId(1), host("reddit.com")),
            (2, FeedId(2), host("reddit.com")),
            (3, FeedId(3), host("a.example")),
            (4, FeedId(4), None),
            (5, FeedId(5), None),
            (6, FeedId(6), host("b.example")),
        ];
        let mut queue = WorkQueue::new(feeds);
        let mut started = Vec::new();
        while let Some(work) = queue.take() {
            started.push(work);
        }
        // One per paced host, every unpaced feed, lanes in turn.
        assert_eq!(ids(&started), [0, 3, 4, 6, 5]);
        assert!(queue.take().is_none(), "reddit.com is busy");

        let reddit = started.first().unwrap().lane;
        queue.release(reddit);
        let next = queue.take().unwrap();
        assert_eq!((next.index, next.id), (1, FeedId(1)));
        assert!(queue.take().is_none(), "reddit.com is busy again");
        queue.release(reddit);
        assert_eq!(queue.take().unwrap().id, FeedId(2));
        queue.release(reddit);
        assert!(queue.take().is_none(), "every feed has started");
        // Releasing a lane that does not exist is harmless.
        queue.release(99);
        assert!(WorkQueue::new([]).take().is_none());
    }

    #[test]
    fn a_per_feed_error_becomes_an_error_outcome() {
        let removed = errored(FeedId(7), &CoreError::NotFound { entity: "feed" });
        assert_eq!(removed.status, FetchStatus::Error);
        assert_eq!(removed.feed_id, FeedId(7));
        assert!(removed.error.unwrap().contains("removed"));
        let panicked = errored(
            FeedId(8),
            &CoreError::Panicked {
                message: "boom".to_owned(),
            },
        );
        assert!(panicked.error.unwrap().contains("boom"));
    }
}
