//! Per-feed refresh scheduling: what one settled attempt means for the
//! feed's next one.
//!
//! - **Success** clears the failure run. The next attempt waits for the
//!   longest of the user's per-feed interval and the feed's own hints
//!   (`<ttl>`, `sy:updatePeriod`, `Cache-Control: max-age`, capped at
//!   [`RefreshPolicy::max_hint`]); with none of them the feed is due on
//!   every sweep.
//! - **Failure** extends the run and backs off exponentially from
//!   [`RefreshPolicy::backoff_base`] (doubling per failure, capped, with
//!   up to 10% jitter so feeds that failed together don't retry in
//!   lockstep). A `Retry-After` longer than the backoff wins.
//! - A **410** marks the feed dead only when the previous attempt was a
//!   410 as well — one CDN glitch never kills a subscription.
//! - A run of [`RefreshPolicy::auto_pause_after`] failures spanning at
//!   least [`RefreshPolicy::auto_pause_min_failing`] **auto-pauses** the
//!   feed; resuming it (any switch back to active) clears the run.
//!
//! Only sweeps honour the schedule. A manual one-feed refresh fetches at
//! once — except while a host's `Retry-After` is still running.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::Duration;

use curio_types::Timestamp;

use crate::model::{Feed, FeedStatus, FetchErrorKind};
use crate::storage::FeedSchedule;

/// Tunables of the refresh schedule ([`crate::CoreOptions::refresh`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RefreshPolicy {
    /// The wait after the first failure; it doubles with each further
    /// one.
    pub backoff_base: Duration,
    /// The longest backoff.
    pub backoff_cap: Duration,
    /// The longest `Retry-After` honoured (a bigger one is clamped).
    pub max_retry_after: Duration,
    /// The longest a feed's own refresh hint may stretch its schedule.
    pub max_hint: Duration,
    /// Failures in a row that auto-pause a feed…
    pub auto_pause_after: u32,
    /// …provided the run has lasted at least this long (so a burst of
    /// manual retries never pauses a feed).
    pub auto_pause_min_failing: Duration,
}

impl Default for RefreshPolicy {
    /// 15 min doubling to 24 h; `Retry-After` up to 7 days; hints up to
    /// 24 h; auto-pause after 10 failures over at least 3 days (the
    /// backoff alone spaces 10 failures about 3.3 days apart).
    fn default() -> Self {
        Self {
            backoff_base: Duration::from_secs(15 * 60),
            backoff_cap: Duration::from_secs(24 * 3_600),
            max_retry_after: Duration::from_secs(7 * 24 * 3_600),
            max_hint: Duration::from_secs(24 * 3_600),
            auto_pause_after: 10,
            auto_pause_min_failing: Duration::from_secs(3 * 24 * 3_600),
        }
    }
}

/// How one attempt ended, as far as scheduling cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Attempt {
    /// `ok` or `not_modified`, with the response's refresh hint (the
    /// longer of the feed's own and `Cache-Control: max-age`).
    Succeeded { hint: Option<Duration> },
    /// An errored attempt.
    Failed {
        kind: FetchErrorKind,
        retry_after: Option<Duration>,
    },
}

impl RefreshPolicy {
    /// The feed's scheduling state after `attempt`, which ended at `now`.
    pub(super) fn after(&self, feed: &Feed, attempt: Attempt, now: Timestamp) -> FeedSchedule {
        let interval = feed
            .refresh_interval_min
            .map(|minutes| Duration::from_secs(u64::from(minutes) * 60));
        match attempt {
            Attempt::Succeeded { hint } => {
                let wait = hint.map(|hint| hint.min(self.max_hint)).max(interval);
                FeedSchedule {
                    consecutive_failures: 0,
                    next_attempt_at: wait.filter(|wait| !wait.is_zero()).map(|w| later(now, w)),
                    last_error_kind: None,
                    failing_since: None,
                    status: None,
                }
            }
            Attempt::Failed { kind, retry_after } => {
                let failures = feed.consecutive_failures.saturating_add(1);
                let failing_since = feed.failing_since.unwrap_or(now);
                let backoff = self.backoff(feed, failures, now);
                let wait = backoff
                    .max(retry_after.map_or(Duration::ZERO, |d| d.min(self.max_retry_after)))
                    .max(interval.unwrap_or(Duration::ZERO));
                let failing_for = Duration::from_millis(
                    u64::try_from(now.unix_millis() - failing_since.unix_millis()).unwrap_or(0),
                );
                let status = if kind == FetchErrorKind::Gone
                    && feed.last_error_kind == Some(FetchErrorKind::Gone)
                {
                    Some(FeedStatus::Dead)
                } else if failures >= self.auto_pause_after
                    && failing_for >= self.auto_pause_min_failing
                    && feed.status == FeedStatus::Active
                {
                    Some(FeedStatus::Paused)
                } else {
                    None
                };
                FeedSchedule {
                    consecutive_failures: failures,
                    next_attempt_at: Some(later(now, wait)),
                    last_error_kind: Some(kind),
                    failing_since: Some(failing_since),
                    status,
                }
            }
        }
    }

    /// `base × 2^(failures-1)`, capped, plus up to 10% jitter derived from
    /// the feed and the attempt (deterministic, but different per feed).
    fn backoff(&self, feed: &Feed, failures: u32, now: Timestamp) -> Duration {
        let doublings = failures.saturating_sub(1).min(30);
        let backoff = self
            .backoff_base
            .saturating_mul(1 << doublings)
            .min(self.backoff_cap);
        let mut hasher = DefaultHasher::new();
        (feed.id, failures, now.unix_millis()).hash(&mut hasher);
        let permille = u32::try_from(hasher.finish() % 101).unwrap_or(0);
        backoff.saturating_add(backoff.saturating_mul(permille) / 1_000)
    }
}

/// The longest wait any schedule asks for — a guard against a policy
/// with absurd caps, so the arithmetic below always fits.
const MAX_WAIT: Duration = Duration::from_secs(366 * 24 * 3_600);

/// `at + wait`, the wait clamped to [`MAX_WAIT`].
fn later(at: Timestamp, wait: Duration) -> Timestamp {
    let millis = i64::try_from(wait.min(MAX_WAIT).as_millis()).unwrap_or(0);
    Timestamp::from_unix_millis(at.unix_millis().saturating_add(millis)).unwrap_or(at)
}

/// Whether a manual refresh must hold off: the host's `Retry-After` is
/// still running. `Some(until)` when it is.
pub(super) fn rate_limited_until(feed: &Feed, now: Timestamp) -> Option<Timestamp> {
    feed.next_attempt_at
        .filter(|&next| feed.last_error_kind == Some(FetchErrorKind::RateLimited) && next > now)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::model::FeedId;

    fn feed() -> Feed {
        Feed {
            id: FeedId(7),
            url: "https://example.com/feed.xml".to_owned(),
            title: None,
            site_url: None,
            description: None,
            etag: None,
            last_modified: None,
            status: FeedStatus::Active,
            allow_private_network: false,
            added_at: at(0),
            last_fetched_at: None,
            last_error: None,
            last_ok_at: None,
            tags: Vec::new(),
            fetch_full_text: false,
            consecutive_failures: 0,
            next_attempt_at: None,
            last_error_kind: None,
            refresh_interval_min: None,
            failing_since: None,
            auto_paused_at: None,
        }
    }

    fn at(secs: i64) -> Timestamp {
        Timestamp::from_unix_millis(1_800_000_000_000 + secs * 1_000).unwrap()
    }

    fn wait(schedule: &FeedSchedule, now: Timestamp) -> Duration {
        let next = schedule.next_attempt_at.unwrap();
        Duration::from_millis(u64::try_from(next.unix_millis() - now.unix_millis()).unwrap())
    }

    fn failed(kind: FetchErrorKind) -> Attempt {
        Attempt::Failed {
            kind,
            retry_after: None,
        }
    }

    /// Applies a schedule to the feed the way storage does.
    fn apply(feed: &mut Feed, schedule: FeedSchedule) {
        feed.consecutive_failures = schedule.consecutive_failures;
        feed.next_attempt_at = schedule.next_attempt_at;
        feed.last_error_kind = schedule.last_error_kind;
        feed.failing_since = schedule.failing_since;
        if let Some(status) = schedule.status {
            feed.status = status;
        }
    }

    #[test]
    fn failures_back_off_exponentially_with_bounded_jitter() {
        let policy = RefreshPolicy::default();
        let mut feed = feed();
        let mut expected = Duration::from_secs(15 * 60);
        for n in 1..=10_u32 {
            let now = at(i64::from(n) * 10_000);
            let schedule = policy.after(&feed, failed(FetchErrorKind::Server), now);
            assert_eq!(schedule.consecutive_failures, n);
            let waited = wait(&schedule, now);
            assert!(
                waited >= expected && waited <= expected + expected / 10,
                "failure {n}: waited {waited:?}, expected about {expected:?}"
            );
            apply(&mut feed, schedule);
            expected = (expected * 2).min(policy.backoff_cap);
        }
        assert_eq!(
            feed.failing_since,
            Some(at(10_000)),
            "the run's start sticks"
        );
    }

    #[test]
    fn a_longer_retry_after_wins_and_a_huge_one_is_clamped() {
        let policy = RefreshPolicy::default();
        let now = at(0);
        let rate_limited = |secs| Attempt::Failed {
            kind: FetchErrorKind::RateLimited,
            retry_after: Some(Duration::from_secs(secs)),
        };
        let schedule = policy.after(&feed(), rate_limited(7_200), now);
        assert_eq!(wait(&schedule, now), Duration::from_secs(7_200));
        assert_eq!(schedule.last_error_kind, Some(FetchErrorKind::RateLimited));
        let schedule = policy.after(&feed(), rate_limited(5), now);
        assert!(wait(&schedule, now) >= policy.backoff_base, "backoff floor");
        let schedule = policy.after(&feed(), rate_limited(u64::MAX / 4), now);
        assert_eq!(wait(&schedule, now), policy.max_retry_after);
    }

    #[test]
    fn success_clears_the_run_and_waits_for_the_longest_hint() {
        let policy = RefreshPolicy::default();
        let now = at(0);
        let mut failing = feed();
        apply(
            &mut failing,
            policy.after(&feed(), failed(FetchErrorKind::Network), now),
        );
        let healthy = policy.after(&failing, Attempt::Succeeded { hint: None }, now);
        assert_eq!(
            healthy,
            FeedSchedule {
                consecutive_failures: 0,
                next_attempt_at: None,
                last_error_kind: None,
                failing_since: None,
                status: None,
            }
        );
        let hinted = policy.after(
            &feed(),
            Attempt::Succeeded {
                hint: Some(Duration::from_secs(3_600)),
            },
            now,
        );
        assert_eq!(wait(&hinted, now), Duration::from_secs(3_600));
        let mut slow = feed();
        slow.refresh_interval_min = Some(180);
        let hinted = policy.after(
            &slow,
            Attempt::Succeeded {
                hint: Some(Duration::from_secs(3_600)),
            },
            now,
        );
        assert_eq!(
            wait(&hinted, now),
            Duration::from_secs(180 * 60),
            "interval wins"
        );
        let capped = policy.after(
            &feed(),
            Attempt::Succeeded {
                hint: Some(Duration::from_secs(30 * 24 * 3_600)),
            },
            now,
        );
        assert_eq!(wait(&capped, now), policy.max_hint);
        let zero = policy.after(
            &feed(),
            Attempt::Succeeded {
                hint: Some(Duration::ZERO),
            },
            now,
        );
        assert_eq!(zero.next_attempt_at, None);
    }

    #[test]
    fn only_a_second_410_in_a_row_kills_a_feed() {
        let policy = RefreshPolicy::default();
        let mut feed = feed();
        let first = policy.after(&feed, failed(FetchErrorKind::Gone), at(0));
        assert_eq!(first.status, None, "one 410 is a glitch until confirmed");
        apply(&mut feed, first);
        let second = policy.after(&feed, failed(FetchErrorKind::Gone), at(100));
        assert_eq!(second.status, Some(FeedStatus::Dead));

        let mut interrupted = self::feed();
        apply(
            &mut interrupted,
            policy.after(&self::feed(), failed(FetchErrorKind::Gone), at(0)),
        );
        let server_error = policy.after(&interrupted, failed(FetchErrorKind::Server), at(10));
        apply(&mut interrupted, server_error);
        let again = policy.after(&interrupted, failed(FetchErrorKind::Gone), at(20));
        assert_eq!(again.status, None, "the 410s were not consecutive");
    }

    #[test]
    fn a_long_run_of_failures_auto_pauses_but_a_quick_burst_does_not() {
        let policy = RefreshPolicy::default();
        let mut burst = feed();
        for n in 0..20 {
            let schedule = policy.after(&burst, failed(FetchErrorKind::Client), at(n));
            assert_eq!(schedule.status, None, "{n} failures within seconds");
            apply(&mut burst, schedule);
        }
        let days = |d: i64| at(d * 24 * 3_600);
        let paused = policy.after(&burst, failed(FetchErrorKind::Client), days(3));
        assert_eq!(paused.status, Some(FeedStatus::Paused));

        let mut user_paused = burst.clone();
        user_paused.status = FeedStatus::Paused;
        let schedule = policy.after(&user_paused, failed(FetchErrorKind::Client), days(4));
        assert_eq!(schedule.status, None, "never re-pause what is not active");
    }

    #[test]
    fn only_a_running_retry_after_holds_a_manual_refresh() {
        let mut feed = feed();
        feed.next_attempt_at = Some(at(60));
        assert_eq!(rate_limited_until(&feed, at(0)), None, "plain backoff");
        feed.last_error_kind = Some(FetchErrorKind::RateLimited);
        assert_eq!(rate_limited_until(&feed, at(0)), Some(at(60)));
        assert_eq!(rate_limited_until(&feed, at(60)), None, "deadline reached");
    }

    #[test]
    fn a_due_feed_is_active_and_outside_its_window() {
        let mut feed = feed();
        assert!(feed.is_due(at(0)));
        feed.next_attempt_at = Some(at(60));
        assert!(!feed.is_due(at(59)));
        assert!(feed.is_due(at(60)));
        feed.status = FeedStatus::Paused;
        assert!(!feed.is_due(at(60)));
    }

    #[test]
    fn absurd_waits_are_clamped_to_a_year() {
        let now = at(0);
        assert_eq!(later(now, Duration::MAX), later(now, MAX_WAIT));
        assert!(later(now, Duration::MAX) > now);
    }
}
