//! Post-v1 enrichment providers — the named seam from the roadmap and
//! D8/D14: source-specific upgrades of an article's stored content,
//! each behind its own cargo feature.
//!
//! Ground rules (binding, same as everywhere else in the core):
//!
//! - **All network I/O goes through the policed client** — providers
//!   receive a [`crate::fetch::PolicedClient`], never build their own.
//! - **Provider output is raw HTML** headed for the standard content
//!   pipeline: the caller runs it through `content::process` (ammonia
//!   sanitize → extract → text), so sanitize-at-ingest holds no matter
//!   what a provider returns.
//! - **Failures are soft.** Enrichment is an upgrade, never a
//!   requirement — a provider error falls back to the generic
//!   readability path, paced by the [`Breaker`].

#[cfg(feature = "enrich-reddit")]
pub mod reddit;
#[cfg(feature = "enrich-reddit")]
pub mod reddit_auth;

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// What a provider produced: raw HTML for the content pipeline plus the
/// metadata the source API declared. Nothing here is stored directly —
/// the HTML passes the sanitize gate first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Enriched {
    /// Raw HTML body (selftext, inline figures, link placeholders).
    pub html: String,
    /// The post author, if the API declared one.
    pub author: Option<String>,
}

/// Enrichment failures. All of them are soft — callers log, trip the
/// breaker, and fall back to the generic path.
#[derive(Debug, thiserror::Error)]
pub enum EnrichError {
    /// The policed fetch failed.
    #[error(transparent)]
    Fetch(#[from] crate::fetch::FetchError),
    /// The API answered with a non-success status.
    #[error("enrichment endpoint answered HTTP {status}")]
    Http {
        /// The response status.
        status: u16,
        /// The server's `Retry-After`, if it sent one (429/503).
        retry_after: Option<Duration>,
    },
    /// The payload did not have the expected shape.
    #[error("enrichment payload: {0}")]
    Malformed(String),
}

/// Consecutive failures before the breaker opens.
const BREAKER_MAX_FAILURES: u32 = 5;
/// How long an open breaker skips enrichment before re-trying.
const BREAKER_COOL_DOWN: Duration = Duration::from_secs(600);

/// A minimal circuit breaker for enrichment calls (the D8 requirement):
/// after `BREAKER_MAX_FAILURES` consecutive failures the provider is
/// skipped for `BREAKER_COOL_DOWN`, so a rate-limited or broken API
/// cannot turn every refresh into a stall.
#[derive(Debug, Default)]
pub struct Breaker {
    state: Mutex<BreakerState>,
}

#[derive(Debug, Default)]
struct BreakerState {
    consecutive_failures: u32,
    open_until: Option<Instant>,
}

impl Breaker {
    /// Whether enrichment may run right now.
    pub fn closed(&self) -> bool {
        let mut state = self.lock();
        match state.open_until {
            Some(until) if Instant::now() < until => false,
            Some(_) => {
                // Cool-down elapsed: half-open — allow one attempt.
                state.open_until = None;
                true
            }
            None => true,
        }
    }

    /// A successful enrichment resets the failure streak.
    pub fn record_success(&self) {
        let mut state = self.lock();
        state.consecutive_failures = 0;
        state.open_until = None;
    }

    /// A failed enrichment advances the streak and may open the breaker.
    pub fn record_failure(&self) {
        let mut state = self.lock();
        state.consecutive_failures += 1;
        if state.consecutive_failures >= BREAKER_MAX_FAILURES {
            state.open_until = Some(Instant::now() + BREAKER_COOL_DOWN);
            state.consecutive_failures = 0;
        }
    }

    /// Opens the breaker immediately — the explicit rate-limit path (a
    /// 429 must not burn four more requests before the streak counter
    /// catches up). The server's `Retry-After` wins when present (floored
    /// at one minute so `Retry-After: 1` cannot thrash); without one, the
    /// default cool-down applies.
    pub fn trip(&self, retry_after: Option<Duration>) {
        const MIN_TRIP: Duration = Duration::from_secs(60);
        let cool_down = retry_after.map_or(BREAKER_COOL_DOWN, |asked| asked.max(MIN_TRIP));
        let mut state = self.lock();
        state.consecutive_failures = 0;
        state.open_until = Some(Instant::now() + cool_down);
    }

    /// How much longer the breaker stays open, if it is open.
    pub fn open_remaining(&self) -> Option<Duration> {
        let state = self.lock();
        state
            .open_until
            .and_then(|until| until.checked_duration_since(Instant::now()))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BreakerState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// Minimal HTML-attribute escaping for a value placed inside `"…"` by a
/// provider building HTML. Ammonia still judges the result — this only
/// keeps the built markup well-formed.
pub(crate) fn escape_attr(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

/// Minimal HTML text escaping for character data built by a provider.
pub(crate) fn escape_text(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_breaker_opens_after_the_failure_streak_and_reenters_half_open() {
        let breaker = Breaker::default();
        assert!(breaker.closed());
        for _ in 0..BREAKER_MAX_FAILURES - 1 {
            breaker.record_failure();
            assert!(breaker.closed(), "still closed before the threshold");
        }
        breaker.record_failure();
        assert!(!breaker.closed(), "open after the streak");

        // A success while open would not normally happen (calls are
        // skipped), but recording one must fully reset it.
        breaker.record_success();
        assert!(breaker.closed());
    }

    #[test]
    fn a_success_resets_the_streak() {
        let breaker = Breaker::default();
        for _ in 0..BREAKER_MAX_FAILURES - 1 {
            breaker.record_failure();
        }
        breaker.record_success();
        breaker.record_failure();
        assert!(breaker.closed(), "the streak restarted from zero");
    }

    #[test]
    fn a_trip_opens_immediately_and_honors_retry_after_with_a_floor() {
        // One 429 opens the breaker at once — no streak needed.
        let breaker = Breaker::default();
        breaker.trip(Some(Duration::from_secs(120)));
        assert!(!breaker.closed());
        let remaining = breaker.open_remaining().unwrap();
        assert!(remaining <= Duration::from_secs(120));
        assert!(remaining > Duration::from_secs(110), "Retry-After wins");

        // `Retry-After: 1` cannot thrash — floored at a minute.
        let floored = Breaker::default();
        floored.trip(Some(Duration::from_secs(1)));
        assert!(floored.open_remaining().unwrap() > Duration::from_secs(55));

        // No Retry-After → the default cool-down.
        let defaulted = Breaker::default();
        defaulted.trip(None);
        assert!(defaulted.open_remaining().unwrap() > Duration::from_secs(500));
    }
}
