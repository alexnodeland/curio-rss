//! Background refresh scheduler — an on-launch pass plus an interval loop, so
//! the reader stays current without a click. It reuses the head's manual
//! refresh path (`refresh_all_and_emit`), so the same
//! `RefreshProgress`/`RefreshFinished`/`FeedsChanged`/`ArticlesChanged` events
//! fire and counts + status update for free.
//!
//! Config lives in `SQLite` settings and is read fresh each tick, so a change in
//! Settings → General takes effect within a minute:
//! - `ui.refresh.interval-minutes` — how often to refresh (`0` disables).
//! - `ui.refresh.on-launch` — whether to refresh once shortly after start.
//!
//! The scheduler's own bookkeeping lives there too: `ui.refresh.last-at` is
//! the wall-clock time (Unix seconds) of the last refresh, so a relaunch
//! remembers it instead of starting the interval clock from zero.
//!
//! Coordination with a manual refresh is the [`RefreshScheduler`]: a manual
//! refresh calls [`RefreshScheduler::record_manual_refresh`], resetting the
//! interval clock so a scheduled sweep never piles on right after one (unless
//! the refresh happened offline and fetched nothing); an in-flight flag
//! keeps two scheduled ticks from overlapping. Core runs one sweep at a time
//! anyway: a tick that lands during a manual sweep joins it (no feed is fetched
//! twice) and leaves the events and notification to that sweep.
//!
//! **Sleep/wake (#123).** The interval clock is wall-clock time
//! ([`SystemTime`]), not a monotonic [`Instant`](std::time::Instant): the
//! monotonic clock stops while a laptop is suspended on macOS and Linux, so an
//! overnight sleep used to count as no time at all. The loop also watches for a
//! wall-clock jump of more than two ticks between wake-ups — the machine slept
//! — and then waits `WAKE_SETTLE` (10 s) for the network to come back before
//! its due-check.
//!
//! **Offline (#123).** Before a sweep, a cheap connectivity probe (a DNS
//! lookup of hosts that refreshed fine before — see `DnsProbe`) decides
//! whether the network is there at all. Offline, the sweep is skipped
//! outright: no fetches, so no feed-health error rows and no "N feeds failed"
//! notification. The change is announced once as a
//! [`ConnectivityChanged`] event (the frontend's "Offline" badge), the
//! interval clock is left alone so the sweep stays due (a skipped on-launch
//! sweep is remembered as owed, whatever the interval), and every tick
//! re-probes until the network returns — then the overdue sweep runs.
//!
//! The same tick is also the event-emission retry timer (#118): a failed
//! JSONL flush (full disk, unwritable events dir) never fails the mutation
//! that staged it — core raises its "pending events" flag instead — so each
//! tick re-drains the staged intents while that flag is up.

use std::collections::HashSet;
use std::net::ToSocketAddrs as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use curio_core::CoreHandle;
use curio_core::model::{Feed, FeedStatus};
use tauri::{AppHandle, State};

use crate::commands::feeds::{Sweep, refresh_all_and_emit};
use crate::commands::{SharedCore, blocking};
use crate::dto::{FetchStatusDto, RefreshOutcomeDto};
use crate::error::CommandError;
use crate::events::{ConnectivityChanged, emit_or_log};

const INTERVAL_KEY: &str = "ui.refresh.interval-minutes";
const ON_LAUNCH_KEY: &str = "ui.refresh.on-launch";
/// The wall-clock time of the last refresh, as Unix seconds.
const LAST_REFRESH_KEY: &str = "ui.refresh.last-at";
const DEFAULT_INTERVAL_MINUTES: u64 = 30;
/// Let the window settle before the on-launch sweep, so first paint is snappy.
const LAUNCH_SETTLE: Duration = Duration::from_secs(5);
/// How often the loop wakes to check whether a refresh is due. Also the upper
/// bound on how long a settings change (interval / disable) takes to apply,
/// and how often the network is re-probed while offline.
const TICK: Duration = Duration::from_secs(60);
/// How long to wait after a detected wake before the due-check, so Wi-Fi has
/// re-associated and DNS answers again.
const WAKE_SETTLE: Duration = Duration::from_secs(10);
/// The most hosts one connectivity probe looks up.
const PROBE_HOSTS: usize = 3;
/// How long one probe lookup may take before it counts as failed (a
/// captive or uplink-less network makes the resolver hang, not refuse).
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Shared refresh coordination between the manual `refresh_all` command and the
/// background scheduler.
#[derive(Debug)]
pub struct RefreshScheduler {
    last_refresh: Mutex<SystemTime>,
    /// The on-launch sweep was skipped offline and has not run since: the
    /// next tick sweeps even when the interval is disabled or not yet due.
    launch_owed: AtomicBool,
    in_flight: AtomicBool,
    /// The last connectivity verdict; starts optimistic (online), so the
    /// badge only ever appears after a probe actually failed. A mutex, not
    /// an atomic: a change and its announcement happen under one lock, so
    /// two racing verdicts can't reach the frontend in the reverse order.
    online: Mutex<bool>,
}

impl Default for RefreshScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl RefreshScheduler {
    /// A scheduler whose interval clock starts now (so the first scheduled
    /// sweep is a full interval after launch, not immediately).
    #[must_use]
    pub fn new() -> Self {
        Self::starting_at(SystemTime::now())
    }

    /// A scheduler whose interval clock resumes from the last refresh
    /// persisted in `core`'s settings — so the first scheduled sweep after a
    /// relaunch is due when it would have been had the app kept running. With
    /// nothing persisted (first launch, or an unreadable value) the clock
    /// starts now, like [`Self::new`].
    #[must_use]
    pub fn load(core: &CoreHandle) -> Self {
        let persisted = core
            .get_setting(LAST_REFRESH_KEY)
            .ok()
            .flatten()
            .and_then(|value| value.parse::<u64>().ok())
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs));
        Self::starting_at(persisted.unwrap_or_else(SystemTime::now))
    }

    fn starting_at(last_refresh: SystemTime) -> Self {
        Self {
            last_refresh: Mutex::new(last_refresh),
            launch_owed: AtomicBool::new(false),
            in_flight: AtomicBool::new(false),
            online: Mutex::new(true),
        }
    }

    /// Books a manual refresh: any feed answering clears the "Offline"
    /// verdict right away (announced through `announce`), and the interval
    /// clock resets — unless the network is known down and nothing proved
    /// otherwise, because then nothing was fetched and the overdue sweep must
    /// still run once the network is back.
    pub async fn record_manual_refresh(
        &self,
        core: &SharedCore,
        outcomes: &[RefreshOutcomeDto],
        announce: impl FnOnce(bool),
    ) {
        let reached_network = outcomes_prove_online(outcomes);
        if reached_network {
            self.observe(true, announce);
        }
        if reached_network || self.is_online() {
            self.mark_refreshed(core).await;
        }
    }

    /// Resets the interval clock — called after a refresh (manual or
    /// scheduled) so the next scheduled sweep is a full interval later, and
    /// any owed launch sweep counts as done — and persists the time,
    /// best-effort and off the async runtime, so a relaunch remembers it.
    async fn mark_refreshed(&self, core: &SharedCore) {
        self.mark_refreshed_at(core, SystemTime::now()).await;
    }

    async fn mark_refreshed_at(&self, core: &SharedCore, at: SystemTime) {
        *self
            .last_refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = at;
        self.launch_owed.store(false, Ordering::SeqCst);
        let secs = at.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        let core = Arc::clone(core);
        let persisted =
            blocking(move || Ok(core.set_setting(LAST_REFRESH_KEY, &secs.to_string())?)).await;
        if let Err(error) = persisted {
            tracing::warn!(%error, "could not persist the last refresh time");
        }
    }

    /// The wall-clock time of the last refresh.
    fn last_refresh(&self) -> SystemTime {
        *self
            .last_refresh
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether a launch sweep skipped offline still has to run.
    fn launch_owed(&self) -> bool {
        self.launch_owed.load(Ordering::SeqCst)
    }

    /// The last connectivity verdict (`true` until a probe says otherwise).
    #[must_use]
    pub fn is_online(&self) -> bool {
        *self.online.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a connectivity verdict and, when it differs from the last
    /// one, calls `announce` with it — still holding the lock, so the
    /// announcements go out in the order the verdicts were recorded.
    /// Returns whether it changed.
    pub fn observe(&self, online: bool, announce: impl FnOnce(bool)) -> bool {
        let mut current = self.online.lock().unwrap_or_else(PoisonError::into_inner);
        if *current == online {
            return false;
        }
        *current = online;
        tracing::info!(online, "connectivity changed");
        announce(online);
        true
    }

    /// Claims the refresh slot; `false` if a scheduled sweep is already running.
    fn try_acquire(&self) -> bool {
        !self.in_flight.swap(true, Ordering::SeqCst)
    }

    fn release(&self) {
        self.in_flight.store(false, Ordering::SeqCst);
    }
}

/// Whether the network is up, as far as the frontend's "Offline" badge goes:
/// the scheduler's last connectivity verdict. Online until a probe fails.
///
/// # Errors
///
/// Never in practice — the `Result` keeps the generated wrapper on the same
/// typed `{ status, data | error }` shape as every other command.
#[tauri::command]
#[specta::specta]
// Async like every command here: Tauri runs a sync command on the main
// thread, and an async one with borrowed state must return a `Result`.
#[allow(clippy::unused_async)]
pub async fn connectivity_online(
    scheduler: State<'_, Arc<RefreshScheduler>>,
) -> Result<bool, CommandError> {
    Ok(scheduler.is_online())
}

/// Whether a refresh is due: enabled (interval > 0) and at least that long —
/// in wall-clock time, so a suspended laptop's sleep counts — since the last
/// one. A last refresh in the future means the clock was set back: the stamp
/// can't be trusted, so that counts as due too (one sweep re-anchors it).
/// Pure — the loop's decision, unit-tested without a clock.
#[must_use]
pub fn refresh_due(interval_minutes: u64, last_refresh: SystemTime, now: SystemTime) -> bool {
    if interval_minutes == 0 {
        return false;
    }
    let interval = Duration::from_secs(interval_minutes * 60);
    !now.duration_since(last_refresh)
        .is_ok_and(|elapsed| elapsed < interval)
}

/// Whether the wall clock moved much further than the loop slept: more than
/// two ticks between consecutive wake-ups means the machine was suspended
/// (tokio's timers run on the monotonic clock, which stops during sleep).
#[must_use]
pub fn woke_from_sleep(before: SystemTime, after: SystemTime, tick: Duration) -> bool {
    after
        .duration_since(before)
        .is_ok_and(|elapsed| elapsed > tick * 2)
}

/// Whether a refresh's outcomes prove the network works: any feed answered
/// (a fresh body, a 304, or even an HTTP error status — a server replied).
#[must_use]
pub fn outcomes_prove_online(outcomes: &[RefreshOutcomeDto]) -> bool {
    outcomes
        .iter()
        .any(|outcome| outcome.status != FetchStatusDto::Error || outcome.http_status.is_some())
}

/// A cheap "is the network there at all?" check, run before every scheduled
/// sweep. Must only say `false` when every fetch would fail anyway.
pub(crate) trait ConnectivityProbe {
    /// `false` only when the network is clearly unreachable.
    async fn is_online(&self) -> bool;
}

/// The side effects of a sweep — the real app emits events and shows OS
/// notifications; the tests count calls.
pub(crate) trait SweepEffects {
    /// Refreshes every active feed (or joins the sweep already running).
    async fn refresh(&self) -> Result<Sweep, CommandError>;
    /// Hands a background sweep's outcomes to the notifier.
    fn notify(&self, outcomes: &[RefreshOutcomeDto]);
    /// Announces a connectivity change (the "Offline" badge).
    fn connectivity_changed(&self, online: bool);
}

/// The production probe: resolve (DNS only — nothing is fetched or
/// connected to) up to [`PROBE_HOSTS`] feed hosts that refreshed fine before.
/// Every fetch resolves its host locally first, so when none of them
/// resolves, every fetch would fail too. With no such host to ask (no feeds
/// yet, only IP-literal or private-network feeds, none ever fetched) it
/// answers online — the probe never blocks a sweep it can't judge.
pub(crate) struct DnsProbe {
    core: SharedCore,
}

impl ConnectivityProbe for DnsProbe {
    async fn is_online(&self) -> bool {
        let core = Arc::clone(&self.core);
        let targets = blocking(move || Ok(probe_targets(&core.list_feeds()?))).await;
        match targets {
            Ok(targets) => any_resolves(&targets, resolves).await,
            Err(error) => {
                tracing::warn!(%error, "connectivity probe could not list feeds");
                true
            }
        }
    }
}

/// The hosts worth probing: active, public-network feeds that have fetched
/// successfully at least once (so a feed whose domain has lapsed never makes
/// the whole app look offline), currently healthy ones first, distinct by
/// host, at most [`PROBE_HOSTS`]. Domain names only — an IP literal has
/// nothing to resolve.
fn probe_targets(feeds: &[Feed]) -> Vec<(String, u16)> {
    let mut candidates: Vec<&Feed> = feeds
        .iter()
        .filter(|feed| {
            feed.status == FeedStatus::Active
                && !feed.allow_private_network
                && feed.last_ok_at.is_some()
        })
        .collect();
    // Stable: healthy feeds first, otherwise subscription order.
    candidates.sort_by_key(|feed| feed.last_error.is_some());
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter_map(|feed| {
            let url = url::Url::parse(&feed.url).ok()?;
            let host = url.domain()?.to_ascii_lowercase();
            let port = url.port_or_known_default()?;
            seen.insert(host.clone()).then_some((host, port))
        })
        .take(PROBE_HOSTS)
        .collect()
}

/// Online iff any target resolves (tried in order, stopping at the first
/// success); no targets at all means "can't tell", which is online.
async fn any_resolves<F, Fut>(targets: &[(String, u16)], resolve: F) -> bool
where
    F: Fn(String, u16) -> Fut,
    Fut: Future<Output = bool>,
{
    if targets.is_empty() {
        return true;
    }
    for (host, port) in targets {
        if resolve(host.clone(), *port).await {
            return true;
        }
    }
    false
}

/// One DNS lookup on the blocking pool, bounded by [`PROBE_TIMEOUT`].
async fn resolves(host: String, port: u16) -> bool {
    let lookup = blocking(move || {
        Ok((host.as_str(), port)
            .to_socket_addrs()
            .is_ok_and(|mut addrs| addrs.next().is_some()))
    });
    matches!(
        tokio::time::timeout(PROBE_TIMEOUT, lookup).await,
        Ok(Ok(true))
    )
}

/// The real app's sweep effects: the shared manual-refresh path, the OS
/// notifier, and the `ConnectivityChanged` event.
struct AppEffects {
    handle: AppHandle,
    core: SharedCore,
}

impl SweepEffects for AppEffects {
    async fn refresh(&self) -> Result<Sweep, CommandError> {
        refresh_all_and_emit(&self.handle, &self.core).await
    }

    fn notify(&self, outcomes: &[RefreshOutcomeDto]) {
        // A background sweep is the moment to notify (foreground refreshes are
        // reported in-app by the refresh status line); the notify layer decides
        // whether the user's prefs + OS permission actually surface anything.
        crate::notify::notify_refresh(&self.handle, &self.core, outcomes);
    }

    fn connectivity_changed(&self, online: bool) {
        emit_or_log(&self.handle, &ConnectivityChanged { online });
    }
}

/// The configured interval in minutes (`0` = disabled), defaulting to
/// `DEFAULT_INTERVAL_MINUTES` when unset or unparseable.
fn interval_minutes(core: &CoreHandle) -> u64 {
    core.get_setting(INTERVAL_KEY)
        .ok()
        .flatten()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_INTERVAL_MINUTES)
}

/// Whether to refresh once shortly after launch (defaults to on).
fn refresh_on_launch(core: &CoreHandle) -> bool {
    core.get_setting(ON_LAUNCH_KEY)
        .ok()
        .flatten()
        .is_none_or(|value| value != "false")
}

/// Spawns the background refresh loop onto the Tauri async runtime: an
/// on-launch sweep (if enabled) after a short settle, then — every `TICK` —
/// a wake check, the pending-events retry and a due-check against the
/// configured interval. The task lives for the process.
pub fn spawn(handle: AppHandle, core: SharedCore, scheduler: Arc<RefreshScheduler>) {
    tauri::async_runtime::spawn(async move {
        let probe = DnsProbe {
            core: Arc::clone(&core),
        };
        let effects = AppEffects {
            handle,
            core: Arc::clone(&core),
        };
        tokio::time::sleep(LAUNCH_SETTLE).await;
        if refresh_on_launch(&core) {
            launch_sweep(&scheduler, &core, &probe, &effects).await;
        }
        loop {
            let before = SystemTime::now();
            tokio::time::sleep(TICK).await;
            if woke_from_sleep(before, SystemTime::now(), TICK) {
                tracing::info!("woke from sleep; checking for a due refresh shortly");
                tokio::time::sleep(WAKE_SETTLE).await;
            }
            retry_pending_events(&core).await;
            tick(
                &scheduler,
                &core,
                &probe,
                &effects,
                interval_minutes(&core),
                SystemTime::now(),
            )
            .await;
        }
    });
}

/// The on-launch sweep. Skipped because the app started offline, it is
/// remembered as owed, so the first tick back online still runs it — even
/// with the interval disabled ("refresh on launch only") — instead of
/// waiting out a full interval or never running at all.
pub(crate) async fn launch_sweep<P: ConnectivityProbe, E: SweepEffects>(
    scheduler: &RefreshScheduler,
    core: &SharedCore,
    probe: &P,
    effects: &E,
) -> SweepOutcome {
    let outcome = sweep(scheduler, core, probe, effects).await;
    if outcome == SweepOutcome::Offline {
        scheduler.launch_owed.store(true, Ordering::SeqCst);
    }
    outcome
}

/// What one tick did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TickOutcome {
    /// Nothing due, and the network isn't known to be down.
    Idle,
    /// Nothing due; the network was known down, so it was re-probed.
    Rechecked,
    /// A sweep was due — see its outcome.
    Swept(SweepOutcome),
}

/// One loop iteration after the sleep: sweep when due (or a skipped launch
/// sweep is owed); otherwise, while the last probe said offline, re-probe so
/// the badge clears within a tick of the network coming back (even with the
/// interval disabled).
pub(crate) async fn tick<P: ConnectivityProbe, E: SweepEffects>(
    scheduler: &RefreshScheduler,
    core: &SharedCore,
    probe: &P,
    effects: &E,
    interval_minutes: u64,
    now: SystemTime,
) -> TickOutcome {
    if scheduler.launch_owed() || refresh_due(interval_minutes, scheduler.last_refresh(), now) {
        return TickOutcome::Swept(sweep(scheduler, core, probe, effects).await);
    }
    if scheduler.is_online() {
        return TickOutcome::Idle;
    }
    observe(scheduler, effects, probe.is_online().await);
    TickOutcome::Rechecked
}

/// What one scheduled sweep did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SweepOutcome {
    /// Another scheduled sweep held the slot.
    Busy,
    /// The probe said offline: nothing was fetched.
    Offline,
    /// The feeds were refreshed (or the attempt failed as a whole — logged).
    Refreshed,
}

/// One scheduled sweep, guarded so two never overlap: probe first and skip
/// everything while offline (no fetches, no health rows, no notification,
/// and the interval clock untouched so the sweep stays due); otherwise the
/// same events a manual refresh emits, then the notifier. A fetch/parse
/// failure is an outcome (logged in the feed-health log), never a panic.
pub(crate) async fn sweep<P: ConnectivityProbe, E: SweepEffects>(
    scheduler: &RefreshScheduler,
    core: &SharedCore,
    probe: &P,
    effects: &E,
) -> SweepOutcome {
    if !scheduler.try_acquire() {
        return SweepOutcome::Busy;
    }
    let online = probe.is_online().await;
    observe(scheduler, effects, online);
    if !online {
        tracing::info!("offline; skipping the scheduled refresh");
        scheduler.release();
        return SweepOutcome::Offline;
    }
    match effects.refresh().await {
        // A tick that found a manual sweep running joined it: that sweep is
        // the user's, reported in-app — nothing to notify.
        Ok(sweep) if sweep.joined => {}
        Ok(sweep) => effects.notify(&sweep.finished.outcomes),
        Err(error) => tracing::warn!(%error, "scheduled refresh failed"),
    }
    scheduler.mark_refreshed(core).await;
    scheduler.release();
    SweepOutcome::Refreshed
}

/// Records a probe verdict and announces it when it changed.
fn observe<E: SweepEffects>(scheduler: &RefreshScheduler, effects: &E, online: bool) {
    scheduler.observe(online, |online| effects.connectivity_changed(online));
}

/// Re-drains event intents a failed flush left staged. A no-op (one atomic
/// load) while core's pending-events flag is down.
async fn retry_pending_events(core: &SharedCore) {
    if !core.has_pending_events() {
        return;
    }
    let core = Arc::clone(core);
    if let Err(error) = blocking(move || Ok(flush_pending_events_impl(&core))).await {
        tracing::warn!(%error, "pending-events retry did not run");
    }
}

/// The blocking half of [`retry_pending_events`]: drain and log the outcome.
/// Returns how many staged events reached the log.
fn flush_pending_events_impl(core: &CoreHandle) -> usize {
    match core.flush_pending_events() {
        Ok(emitted) => {
            tracing::info!(emitted, "pending events flushed to the event log");
            emitted
        }
        Err(error) => {
            tracing::warn!(%error, "pending events still cannot be flushed; retrying next tick");
            0
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test assertions
mod tests {
    use std::sync::atomic::AtomicUsize;

    use curio_core::model::FeedId;
    use curio_types::Timestamp;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::commands::feeds::refresh_all_impl;
    use crate::commands::testutil::{seed_article, temp_core, temp_core_trusting_localhost};

    const HOUR: Duration = Duration::from_hours(1);

    /// A probe with a fixed verdict that counts how often it was asked.
    struct FakeProbe {
        online: AtomicBool,
        calls: AtomicUsize,
    }

    impl FakeProbe {
        fn new(online: bool) -> Self {
            Self {
                online: AtomicBool::new(online),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl ConnectivityProbe for FakeProbe {
        fn is_online(&self) -> impl Future<Output = bool> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            std::future::ready(self.online.load(Ordering::SeqCst))
        }
    }

    /// The real refresh path (no app handle: no events), with the notifier
    /// and the connectivity announcements recorded instead of shown.
    struct RecordingEffects {
        core: SharedCore,
        notified: AtomicUsize,
        connectivity: Mutex<Vec<bool>>,
    }

    impl RecordingEffects {
        fn new(core: &SharedCore) -> Self {
            Self {
                core: Arc::clone(core),
                notified: AtomicUsize::new(0),
                connectivity: Mutex::new(Vec::new()),
            }
        }

        fn connectivity(&self) -> Vec<bool> {
            self.connectivity.lock().unwrap().clone()
        }
    }

    impl SweepEffects for RecordingEffects {
        async fn refresh(&self) -> Result<Sweep, CommandError> {
            refresh_all_impl(&self.core, |_| {}).await
        }

        fn notify(&self, _outcomes: &[RefreshOutcomeDto]) {
            self.notified.fetch_add(1, Ordering::SeqCst);
        }

        fn connectivity_changed(&self, online: bool) {
            self.connectivity.lock().unwrap().push(online);
        }
    }

    const RSS: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel>
        <title>T</title><link>https://example.test/</link>
        <item><title>One</title><link>https://example.test/1</link><guid>1</guid></item>
        </channel></rss>"#;

    /// A trusting core subscribed to one feed on a local mock server, which
    /// answers every GET with `RSS`.
    async fn core_with_feed() -> (tempfile::TempDir, SharedCore, MockServer, FeedId) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(RSS))
            .mount(&server)
            .await;
        let (dir, core) = temp_core_trusting_localhost();
        let feed = core
            .add_feed(curio_core::model::NewFeed {
                url: format!("{}/feed.xml", server.uri()),
                title: None,
                tags: Vec::new(),
            })
            .unwrap();
        (dir, core, server, feed.id)
    }

    #[tokio::test]
    async fn offline_skips_the_sweep_no_fetch_no_health_row_no_notification() {
        let (_dir, core, server, feed) = core_with_feed().await;
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        let probe = FakeProbe::new(false);
        let effects = RecordingEffects::new(&core);

        let outcome = tick(&scheduler, &core, &probe, &effects, 30, SystemTime::now()).await;

        assert_eq!(outcome, TickOutcome::Swept(SweepOutcome::Offline));
        assert!(
            server.received_requests().await.unwrap().is_empty(),
            "no fetch"
        );
        assert!(
            core.recent_fetches(feed, 10).unwrap().is_empty(),
            "no feed-health row"
        );
        assert_eq!(
            effects.notified.load(Ordering::SeqCst),
            0,
            "no notification"
        );
        assert_eq!(
            effects.connectivity(),
            vec![false],
            "the badge goes up once"
        );
        assert!(!scheduler.is_online());
        assert_eq!(
            scheduler.last_refresh(),
            UNIX_EPOCH,
            "the clock is untouched, so the sweep stays due"
        );
        assert_eq!(core.get_setting(LAST_REFRESH_KEY).unwrap(), None);
        assert!(scheduler.try_acquire(), "the in-flight slot was released");
    }

    #[tokio::test]
    async fn the_overdue_sweep_runs_once_the_network_is_back() {
        let (_dir, core, server, feed) = core_with_feed().await;
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        let probe = FakeProbe::new(false);
        let effects = RecordingEffects::new(&core);

        // Two offline ticks: one announcement, not one per tick.
        for _ in 0..2 {
            tick(&scheduler, &core, &probe, &effects, 30, SystemTime::now()).await;
        }
        assert_eq!(effects.connectivity(), vec![false]);

        probe.online.store(true, Ordering::SeqCst);
        let outcome = tick(&scheduler, &core, &probe, &effects, 30, SystemTime::now()).await;

        assert_eq!(outcome, TickOutcome::Swept(SweepOutcome::Refreshed));
        assert_eq!(
            effects.connectivity(),
            vec![false, true],
            "the badge clears"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(core.recent_fetches(feed, 10).unwrap().len(), 1);
        assert_eq!(effects.notified.load(Ordering::SeqCst), 1);
        assert!(scheduler.last_refresh() > UNIX_EPOCH, "the clock moved on");
        assert!(
            core.get_setting(LAST_REFRESH_KEY).unwrap().is_some(),
            "and was persisted"
        );
    }

    #[tokio::test]
    async fn a_launch_sweep_skipped_offline_runs_once_the_network_is_back() {
        let (_dir, core, server, _feed) = core_with_feed().await;
        // A fresh clock: the interval alone would not be due for 30 minutes.
        let scheduler = RefreshScheduler::new();
        let probe = FakeProbe::new(false);
        let effects = RecordingEffects::new(&core);

        assert_eq!(
            launch_sweep(&scheduler, &core, &probe, &effects).await,
            SweepOutcome::Offline
        );
        assert!(
            scheduler.launch_owed(),
            "the skipped launch sweep stays owed"
        );
        assert_eq!(
            core.get_setting(LAST_REFRESH_KEY).unwrap(),
            None,
            "nothing pretends a refresh happened"
        );

        probe.online.store(true, Ordering::SeqCst);
        let outcome = tick(&scheduler, &core, &probe, &effects, 30, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Swept(SweepOutcome::Refreshed));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(!scheduler.launch_owed(), "and is paid off");
    }

    #[tokio::test]
    async fn a_launch_only_setup_still_runs_the_skipped_launch_sweep() {
        // "Refresh on launch only": the interval is disabled, so only the
        // owed launch sweep can bring the feeds up to date.
        let (_dir, core, server, _feed) = core_with_feed().await;
        let scheduler = RefreshScheduler::new();
        let probe = FakeProbe::new(false);
        let effects = RecordingEffects::new(&core);

        assert_eq!(
            launch_sweep(&scheduler, &core, &probe, &effects).await,
            SweepOutcome::Offline
        );
        // Still offline: each tick re-probes through the owed sweep, fetching
        // nothing.
        let outcome = tick(&scheduler, &core, &probe, &effects, 0, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Swept(SweepOutcome::Offline));
        assert!(server.received_requests().await.unwrap().is_empty());

        probe.online.store(true, Ordering::SeqCst);
        let outcome = tick(&scheduler, &core, &probe, &effects, 0, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Swept(SweepOutcome::Refreshed));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert_eq!(effects.connectivity(), vec![false, true]);

        // Paid off: with the interval disabled, nothing else runs.
        let outcome = tick(&scheduler, &core, &probe, &effects, 0, SystemTime::now()).await;
        assert_eq!(outcome, TickOutcome::Idle);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_manual_refresh_offline_that_reached_nothing_leaves_the_sweep_due() {
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        scheduler.observe(false, |_| {});
        let announced = Mutex::new(Vec::new());

        let failed = [outcome(FetchStatusDto::Error, None)];
        scheduler
            .record_manual_refresh(&core, &failed, |online| {
                announced.lock().unwrap().push(online);
            })
            .await;

        assert!(!scheduler.is_online(), "the badge stays up");
        assert!(announced.lock().unwrap().is_empty());
        assert_eq!(
            scheduler.last_refresh(),
            UNIX_EPOCH,
            "the clock is untouched"
        );
        assert_eq!(core.get_setting(LAST_REFRESH_KEY).unwrap(), None);
        assert!(refresh_due(30, scheduler.last_refresh(), SystemTime::now()));
    }

    #[tokio::test]
    async fn a_manual_refresh_that_reached_a_server_clears_the_badge_and_the_clock() {
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        scheduler.observe(false, |_| {});
        scheduler.launch_owed.store(true, Ordering::SeqCst);
        let announced = Mutex::new(Vec::new());

        let answered = [
            outcome(FetchStatusDto::Error, None),
            outcome(FetchStatusDto::Ok, Some(200)),
        ];
        scheduler
            .record_manual_refresh(&core, &answered, |online| {
                announced.lock().unwrap().push(online);
            })
            .await;

        assert!(scheduler.is_online());
        assert_eq!(*announced.lock().unwrap(), vec![true], "announced once");
        assert!(scheduler.last_refresh() > UNIX_EPOCH, "the clock reset");
        assert!(core.get_setting(LAST_REFRESH_KEY).unwrap().is_some());
        assert!(!scheduler.launch_owed(), "an owed launch sweep is done too");
    }

    #[tokio::test]
    async fn a_manual_refresh_while_online_always_resets_the_clock() {
        // Not known offline: even a refresh where every feed failed counts.
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        let announced = Mutex::new(Vec::new());

        let failed = [outcome(FetchStatusDto::Error, None)];
        scheduler
            .record_manual_refresh(&core, &failed, |online| {
                announced.lock().unwrap().push(online);
            })
            .await;

        assert!(scheduler.last_refresh() > UNIX_EPOCH);
        assert!(scheduler.is_online());
        assert!(announced.lock().unwrap().is_empty(), "nothing changed");
    }

    #[tokio::test]
    async fn an_online_launch_sweep_refreshes_and_restarts_the_clock() {
        let (_dir, core, server, _feed) = core_with_feed().await;
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        let probe = FakeProbe::new(true);
        let effects = RecordingEffects::new(&core);

        assert_eq!(
            launch_sweep(&scheduler, &core, &probe, &effects).await,
            SweepOutcome::Refreshed
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        assert!(!refresh_due(
            30,
            scheduler.last_refresh(),
            SystemTime::now()
        ));
        assert_eq!(
            effects.connectivity(),
            Vec::<bool>::new(),
            "no change to announce"
        );
    }

    #[tokio::test]
    async fn an_offline_badge_clears_even_when_nothing_is_due() {
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::new();
        scheduler.observe(false, |_| {});
        let probe = FakeProbe::new(true);
        let effects = RecordingEffects::new(&core);

        // Interval disabled: never due, but the network is still re-probed.
        let outcome = tick(&scheduler, &core, &probe, &effects, 0, SystemTime::now()).await;

        assert_eq!(outcome, TickOutcome::Rechecked);
        assert_eq!(effects.connectivity(), vec![true]);
        assert!(scheduler.is_online());
    }

    #[tokio::test]
    async fn an_idle_online_tick_never_probes() {
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::new();
        let probe = FakeProbe::new(true);
        let effects = RecordingEffects::new(&core);

        let outcome = tick(&scheduler, &core, &probe, &effects, 30, SystemTime::now()).await;

        assert_eq!(outcome, TickOutcome::Idle);
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
        assert_eq!(effects.connectivity(), Vec::<bool>::new());
    }

    #[tokio::test]
    async fn a_sweep_already_in_flight_is_not_doubled() {
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::new();
        assert!(scheduler.try_acquire());
        let probe = FakeProbe::new(true);
        let effects = RecordingEffects::new(&core);

        assert_eq!(
            sweep(&scheduler, &core, &probe, &effects).await,
            SweepOutcome::Busy
        );
        assert_eq!(probe.calls.load(Ordering::SeqCst), 0);
    }

    /// The timer's drain: nothing while the events dir is unwritable, the
    /// whole backlog once it is back (#118).
    #[test]
    fn the_retry_drains_the_backlog_once_the_log_is_writable_again() {
        let (_dir, core) = temp_core();
        let id = curio_core::model::ArticleId(seed_article(&core, "starred"));
        let events_dir = core.profile_dir().join(".curio").join("events");
        std::fs::remove_dir_all(&events_dir).unwrap();
        std::fs::write(&events_dir, b"not a directory").unwrap();

        assert!(core.set_starred(id, true).unwrap(), "the mutation succeeds");
        assert!(core.has_pending_events());
        assert_eq!(flush_pending_events_impl(&core), 0);
        assert!(core.has_pending_events());

        std::fs::remove_file(&events_dir).unwrap();
        std::fs::create_dir(&events_dir).unwrap();
        assert_eq!(flush_pending_events_impl(&core), 1);
        assert!(!core.has_pending_events());
        assert_eq!(core.pending_intent_count().unwrap(), 0);
    }

    #[test]
    fn a_zero_interval_is_never_due() {
        let now = SystemTime::now();
        assert!(!refresh_due(0, now, now));
        assert!(!refresh_due(0, now - Duration::from_hours(24), now));
    }

    #[test]
    fn a_refresh_is_due_only_once_the_interval_has_elapsed() {
        // 30-minute interval: not due at 29 minutes, due at exactly 30.
        let last = UNIX_EPOCH + Duration::from_hours(500_000);
        assert!(!refresh_due(30, last, last + Duration::from_mins(29)));
        assert!(refresh_due(30, last, last + Duration::from_mins(30)));
        assert!(refresh_due(30, last, last + Duration::from_mins(31)));
    }

    #[test]
    fn an_overnight_sleep_counts_as_due() {
        // Refreshed, then the lid closed for 8 hours: wall-clock time moved on
        // even though the monotonic clock (and the tick) did not.
        let last = UNIX_EPOCH + Duration::from_hours(500_000);
        assert!(refresh_due(30, last, last + 8 * HOUR));
        assert!(refresh_due(180, last, last + 8 * HOUR));
    }

    #[test]
    fn a_clock_set_back_counts_as_due() {
        let last = UNIX_EPOCH + Duration::from_hours(500_000);
        assert!(refresh_due(30, last, last - HOUR));
    }

    #[test]
    fn a_wall_clock_jump_of_more_than_two_ticks_is_a_wake() {
        let before = UNIX_EPOCH + Duration::from_hours(500_000);
        assert!(
            !woke_from_sleep(before, before + TICK, TICK),
            "a normal tick"
        );
        assert!(
            !woke_from_sleep(before, before + TICK * 2, TICK),
            "a slow tick"
        );
        assert!(woke_from_sleep(before, before + 8 * HOUR, TICK), "a night");
        assert!(
            !woke_from_sleep(before, before - HOUR, TICK),
            "a clock set back is not a wake"
        );
    }

    #[tokio::test]
    async fn mark_refreshed_resets_and_persists_the_clock() {
        let (_dir, core) = temp_core();
        let scheduler = RefreshScheduler::starting_at(UNIX_EPOCH);
        let at = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        scheduler.mark_refreshed_at(&core, at).await;
        assert_eq!(scheduler.last_refresh(), at);
        assert_eq!(
            core.get_setting(LAST_REFRESH_KEY).unwrap().as_deref(),
            Some("1700000000")
        );

        // A relaunch resumes from the persisted time, not from "now".
        assert_eq!(RefreshScheduler::load(&core).last_refresh(), at);

        scheduler.mark_refreshed(&core).await;
        assert!(scheduler.last_refresh() > at);
    }

    #[test]
    fn with_nothing_persisted_the_clock_starts_at_launch() {
        let (_dir, core) = temp_core();
        let before = SystemTime::now();
        assert!(RefreshScheduler::load(&core).last_refresh() >= before);

        core.set_setting(LAST_REFRESH_KEY, "not a number").unwrap();
        assert!(RefreshScheduler::load(&core).last_refresh() >= before);
    }

    #[test]
    fn observe_announces_only_changes() {
        let scheduler = RefreshScheduler::new();
        let announced = Mutex::new(Vec::new());
        let record = |online| announced.lock().unwrap().push(online);
        assert!(scheduler.is_online(), "optimistic until a probe fails");
        assert!(!scheduler.observe(true, record));
        assert!(scheduler.observe(false, record));
        assert!(!scheduler.observe(false, record));
        assert!(!scheduler.is_online());
        assert!(scheduler.observe(true, record));
        assert_eq!(*announced.lock().unwrap(), vec![false, true]);
    }

    #[test]
    fn the_in_flight_flag_admits_one_holder_at_a_time() {
        let scheduler = RefreshScheduler::new();
        assert!(scheduler.try_acquire(), "first acquire succeeds");
        assert!(
            !scheduler.try_acquire(),
            "a second is refused while in flight"
        );
        scheduler.release();
        assert!(
            scheduler.try_acquire(),
            "released, so it can be claimed again"
        );
    }

    fn outcome(status: FetchStatusDto, http_status: Option<u16>) -> RefreshOutcomeDto {
        RefreshOutcomeDto {
            feed_id: 1,
            status,
            http_status,
            new_articles: 0,
            updated_articles: 0,
            error: None,
            error_kind: None,
        }
    }

    #[test]
    fn any_answer_from_a_server_proves_online() {
        assert!(!outcomes_prove_online(&[]));
        assert!(!outcomes_prove_online(&[outcome(
            FetchStatusDto::Error,
            None
        )]));
        assert!(outcomes_prove_online(&[
            outcome(FetchStatusDto::Error, None),
            outcome(FetchStatusDto::NotModified, Some(304)),
        ]));
        assert!(outcomes_prove_online(&[outcome(
            FetchStatusDto::Error,
            Some(500)
        )]));
        assert!(outcomes_prove_online(&[outcome(
            FetchStatusDto::Ok,
            Some(200)
        )]));
    }

    fn feed_at(id: i64, url: &str) -> Feed {
        Feed {
            id: FeedId(id),
            url: url.to_owned(),
            title: None,
            site_url: None,
            description: None,
            etag: None,
            last_modified: None,
            status: FeedStatus::Active,
            allow_private_network: false,
            added_at: Timestamp::now(),
            last_fetched_at: Some(Timestamp::now()),
            last_error: None,
            last_error_kind: None,
            last_ok_at: Some(Timestamp::now()),
            tags: Vec::new(),
            fetch_full_text: false,
            consecutive_failures: 0,
            next_attempt_at: None,
            refresh_interval_min: None,
            failing_since: None,
            auto_paused_at: None,
            refresh_hint_secs: None,
        }
    }

    #[test]
    fn probe_targets_are_distinct_hosts_that_worked_before_healthy_first() {
        let erroring = Feed {
            last_error: Some("timed out".to_owned()),
            ..feed_at(1, "https://flaky.example/rss")
        };
        let never_ok = Feed {
            last_ok_at: None,
            ..feed_at(2, "https://lapsed.example/rss")
        };
        let paused = Feed {
            status: FeedStatus::Paused,
            ..feed_at(3, "https://paused.example/rss")
        };
        let private = Feed {
            allow_private_network: true,
            ..feed_at(4, "http://nas.local/rss")
        };
        let feeds = vec![
            erroring,
            never_ok,
            paused,
            private,
            feed_at(5, "http://127.0.0.1:8080/rss"),
            feed_at(6, "https://A.example/one"),
            feed_at(7, "https://a.example/two"),
            feed_at(8, "http://b.example:8080/rss"),
            feed_at(9, "https://c.example/rss"),
        ];

        assert_eq!(
            probe_targets(&feeds),
            vec![
                ("a.example".to_owned(), 443),
                ("b.example".to_owned(), 8080),
                ("c.example".to_owned(), 443),
            ]
        );
        // With only the erroring one left, it still gets probed.
        assert_eq!(
            probe_targets(&feeds[..1]),
            vec![("flaky.example".to_owned(), 443)]
        );
        assert_eq!(probe_targets(&feeds[1..5]), Vec::new());
    }

    #[tokio::test]
    async fn the_probe_is_online_if_any_target_resolves() {
        let targets = vec![("a.example".to_owned(), 443), ("b.example".to_owned(), 443)];
        let asked = Mutex::new(Vec::new());
        let only_b = |host: String, _port| {
            asked.lock().unwrap().push(host.clone());
            async move { host == "b.example" }
        };
        assert!(any_resolves(&targets, only_b).await);
        assert_eq!(*asked.lock().unwrap(), vec!["a.example", "b.example"]);

        assert!(!any_resolves(&targets, |_, _| async { false }).await);
        assert!(
            any_resolves(&[], |_, _| async { false }).await,
            "nothing to ask means can't tell, which is online"
        );
    }

    #[tokio::test]
    async fn the_real_lookup_resolves_localhost() {
        // `localhost` resolves from the hosts file — no network involved.
        assert!(resolves("localhost".to_owned(), 80).await);
    }

    #[tokio::test]
    async fn the_dns_probe_cannot_judge_an_empty_library_so_it_is_online() {
        let (_dir, core) = temp_core();
        assert!(DnsProbe { core }.is_online().await);
    }
}
