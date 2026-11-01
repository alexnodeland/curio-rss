//! Refresh sweeps: many feeds at bounded concurrency, one sweep in flight
//! per handle, cancellable, with per-feed progress.
//!
//! A sweep is the only multi-feed refresh path — both heads call it. Up to
//! [`RefreshOptions::concurrency`] feeds are in flight at once (the policed
//! client's per-host politeness still queues same-host requests), so a few
//! hung hosts cost about one request timeout between them, not one each.
//! A per-feed `Err` (the feed was removed mid-sweep, a transient storage
//! error, a panicking parse) becomes that feed's outcome; it never aborts
//! the sweep.

use std::pin::pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::StreamExt;
use futures_util::stream;
use tokio::sync::{Notify, watch};

use curio_types::Timestamp;

use crate::model::{FeedId, FetchStatus};

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
    /// Feeds finished so far, this one included.
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
    /// One outcome per feed the sweep got to, in the requested order. A
    /// cancelled sweep has none for the feeds it never started.
    pub outcomes: Vec<RefreshOutcome>,
    /// Whether [`CoreHandle::cancel_refresh`] stopped the sweep.
    pub cancelled: bool,
    /// Whether this request found a sweep already running and waited for
    /// it instead of starting another (its own feed list and progress
    /// callback were not used — the running sweep's owner reports
    /// progress).
    pub joined: bool,
}

impl CoreHandle {
    /// Refreshes every *due* feed at the default concurrency (see
    /// [`Self::refresh_all_with`]). Per-feed failures land in their
    /// outcome rows. Joins a sweep already in flight (see
    /// [`Self::refresh_many`]).
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

    /// Refreshes every *due* feed as one sweep — see
    /// [`Self::refresh_many`]. A feed is due when it is active and
    /// outside its backoff / `Retry-After` / refresh-hint window
    /// ([`crate::model::Feed::is_due`]); the others are not fetched and
    /// get no outcome.
    ///
    /// # Errors
    ///
    /// Storage errors listing the feeds only.
    pub async fn refresh_all_with(
        &self,
        options: RefreshOptions,
        on_progress: impl FnMut(SweepProgress<'_>),
    ) -> Result<SweepReport, CoreError> {
        let due = self
            .on_blocking_pool(|storage| {
                let now = Timestamp::now();
                Ok(storage
                    .list_feeds()?
                    .into_iter()
                    .filter(|feed| feed.is_due(now))
                    .map(|feed| feed.id)
                    .collect::<Vec<_>>())
            })
            .await?;
        Ok(self.refresh_many(due, options, on_progress).await)
    }

    /// Refreshes `ids` as one sweep: up to `options.concurrency` feeds in
    /// flight at once, `on_progress` called as each one finishes (in
    /// completion order, with a running `done`/`total`), and every
    /// per-feed `Err` turned into an [`FetchStatus::Error`] outcome so
    /// the sweep always runs to the end.
    ///
    /// **One sweep at a time.** A request made while another sweep is in
    /// flight joins it: it waits for that sweep and returns its report
    /// with [`SweepReport::joined`] set — no feed is fetched twice. (If
    /// the running sweep's caller gives up on it, a joined request starts
    /// a sweep of its own.)
    ///
    /// [`Self::cancel_refresh`] stops the sweep: no further feed starts,
    /// fetches in flight are dropped, and the report comes back with
    /// [`SweepReport::cancelled`] set and no outcomes for the feeds never
    /// started.
    pub async fn refresh_many(
        &self,
        ids: Vec<FeedId>,
        options: RefreshOptions,
        mut on_progress: impl FnMut(SweepProgress<'_>),
    ) -> SweepReport {
        loop {
            match self.sweep.claim() {
                Claim::Join(mut running) => {
                    // A sender dropped without a report: that sweep's
                    // caller gave up on it — claim again.
                    if let Ok(report) = running.wait_for(Option::is_some).await
                        && let Some(report) = report.as_ref()
                    {
                        return SweepReport {
                            joined: true,
                            ..SweepReport::clone(report)
                        };
                    }
                }
                Claim::Run(run) => {
                    let report = self
                        .run_sweep(run.id, &run.cancel, ids, options, &mut on_progress)
                        .await;
                    run.finish(&report);
                    return report;
                }
            }
        }
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

    /// The body of a claimed sweep.
    async fn run_sweep(
        &self,
        sweep_id: u64,
        cancel: &RefreshCancel,
        ids: Vec<FeedId>,
        options: RefreshOptions,
        on_progress: &mut impl FnMut(SweepProgress<'_>),
    ) -> SweepReport {
        let total = ids.len();
        let mut slots: Vec<Option<RefreshOutcome>> = (0..total).map(|_| None).collect();
        let mut finished = pin!(
            stream::iter(ids.into_iter().enumerate())
                .map(|(index, id)| async move {
                    // The cancel check sits where the stream pulls the
                    // next feed, so a cancelled sweep starts nothing new.
                    if cancel.is_cancelled() {
                        return (index, None);
                    }
                    let outcome = match self.refresh_feed_until(id, Some(cancel)).await {
                        Ok(outcome) => outcome,
                        Err(err) => {
                            tracing::warn!(feed = %id, %err, "feed refresh failed; the sweep carries on");
                            errored(id, &err)
                        }
                    };
                    (index, Some(outcome))
                })
                .buffer_unordered(options.concurrency.max(1))
        );
        let mut done = 0;
        while let Some((index, outcome)) = finished.next().await {
            let Some(outcome) = outcome else { continue };
            done += 1;
            on_progress(SweepProgress {
                sweep_id,
                done,
                total,
                outcome: &outcome,
            });
            if let Some(slot) = slots.get_mut(index) {
                *slot = Some(outcome);
            }
        }
        SweepReport {
            sweep_id,
            total,
            outcomes: slots.into_iter().flatten().collect(),
            cancelled: cancel.is_cancelled(),
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
    /// Publishes the report to every joiner, then frees the slot.
    fn finish(self, report: &SweepReport) {
        self.report.send_replace(Some(Arc::new(report.clone())));
    }
}

impl Drop for RunningSweep<'_> {
    fn drop(&mut self) {
        let mut active = self
            .slot
            .active
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if active.as_ref().is_some_and(|sweep| sweep.id == self.id) {
            *active = None;
        }
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
