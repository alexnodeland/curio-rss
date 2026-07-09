//! OS notifications for background refreshes (WP14).
//!
//! Fired from Rust, not JS: the trigger — the refresh outcome vector — is
//! already Rust-side (the scheduler holds it), the scheduler runs headless, and
//! a Rust notification works whether or not a webview is focused. The decision
//! is a pure function (`plan`) over the outcomes, the user's prefs (read fresh
//! from the same `SQLite` settings table the frontend writes), and the current
//! local time; [`notify_refresh`] wraps it with the impure checks (window focus,
//! OS permission) and shows the notification.
//!
//! Preferences (`ui.notify.*`, mirrored in the TS `SETTING_KEYS` by convention):
//! master `enabled` (opt-in, default off), per-event `new-articles` / `errors` /
//! `feed-dead` (default on once enabled), and a `quiet-start`/`quiet-end`
//! `"HH:MM"` window. The notification body is English — like the native menu, an
//! OS surface outside the in-app i18n layer.

use chrono::{Local, NaiveTime};
use curio_core::CoreHandle;
use tauri::{AppHandle, Manager as _};
use tauri_plugin_notification::{NotificationExt as _, PermissionState};

use crate::dto::{FetchStatusDto, RefreshOutcomeDto};
use crate::error::{CommandError, ErrorCode};

const ENABLED_KEY: &str = "ui.notify.enabled";
const NEW_ARTICLES_KEY: &str = "ui.notify.new-articles";
const ERRORS_KEY: &str = "ui.notify.errors";
const FEED_DEAD_KEY: &str = "ui.notify.feed-dead";
const QUIET_START_KEY: &str = "ui.notify.quiet-start";
const QUIET_END_KEY: &str = "ui.notify.quiet-end";

/// The HTTP status a feed returns when it is gone for good (auto-paused dead).
const GONE: u16 = 410;

/// The user's notification preferences, read fresh from settings each refresh.
// Four independent boolean prefs (master + three per-event categories); a
// bitflags struct would be less legible than the named fields.
#[allow(clippy::struct_excessive_bools)]
struct NotifyPrefs {
    enabled: bool,
    new_articles: bool,
    errors: bool,
    feed_dead: bool,
    quiet: Option<(NaiveTime, NaiveTime)>,
}

impl NotifyPrefs {
    fn read(core: &CoreHandle) -> Self {
        let get = |key: &str| core.storage().get_setting(key).ok().flatten();
        // Per-event flags default ON (once master is enabled); master defaults OFF.
        let flag = |key: &str| get(key).is_none_or(|value| value != "false");
        Self {
            enabled: get(ENABLED_KEY).is_some_and(|value| value == "true"),
            new_articles: flag(NEW_ARTICLES_KEY),
            errors: flag(ERRORS_KEY),
            feed_dead: flag(FEED_DEAD_KEY),
            quiet: parse_quiet(get(QUIET_START_KEY), get(QUIET_END_KEY)),
        }
    }
}

/// Parses the `"HH:MM"` quiet-hours window; `None` if either bound is unset or
/// malformed (so a half-configured window never suppresses notifications).
fn parse_quiet(start: Option<String>, end: Option<String>) -> Option<(NaiveTime, NaiveTime)> {
    let start = NaiveTime::parse_from_str(&start?, "%H:%M").ok()?;
    let end = NaiveTime::parse_from_str(&end?, "%H:%M").ok()?;
    Some((start, end))
}

/// Whether `now` falls inside the quiet window, handling a window that wraps
/// past midnight (e.g. 22:00 → 07:00). An empty (`start == end`) window is never
/// quiet.
fn in_quiet(now: NaiveTime, window: Option<(NaiveTime, NaiveTime)>) -> bool {
    match window {
        None => false,
        Some((start, end)) if start == end => false,
        Some((start, end)) if start < end => now >= start && now < end,
        Some((start, end)) => now >= start || now < end,
    }
}

/// Decides the notification body for a set of refresh outcomes, or `None` when
/// nothing qualifies (master off, quiet hours, all categories disabled or empty).
/// Pure — the impure focus/permission checks live in [`notify_refresh`].
fn plan(outcomes: &[RefreshOutcomeDto], prefs: &NotifyPrefs, now: NaiveTime) -> Option<String> {
    if !prefs.enabled || in_quiet(now, prefs.quiet) {
        return None;
    }
    let mut new_total: u64 = 0;
    let mut errored: usize = 0;
    let mut dead: usize = 0;
    for outcome in outcomes {
        new_total += outcome.new_articles;
        if outcome.status == FetchStatusDto::Error {
            if outcome.http_status == Some(GONE) {
                dead += 1;
            } else {
                errored += 1;
            }
        }
    }
    let mut lines: Vec<String> = Vec::new();
    if prefs.new_articles && new_total > 0 {
        lines.push(format!("{new_total} new {}", plural(new_total, "article")));
    }
    if prefs.errors && errored > 0 {
        lines.push(format!(
            "{errored} {} failed to refresh",
            plural(errored as u64, "feed")
        ));
    }
    if prefs.feed_dead && dead > 0 {
        lines.push(format!("{dead} {} went dead", plural(dead as u64, "feed")));
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join(" · "))
    }
}

/// `word` or `words`, for the count.
fn plural(count: u64, word: &str) -> String {
    if count == 1 {
        word.to_owned()
    } else {
        format!("{word}s")
    }
}

/// Whether the main window currently has OS focus (best-effort: unknown ⇒ not
/// focused, so a headless refresh still notifies).
fn window_focused(handle: &AppHandle) -> bool {
    handle
        .get_webview_window("main")
        .and_then(|window| window.is_focused().ok())
        .unwrap_or(false)
}

/// Shows an OS notification summarizing a background refresh, if the user's
/// prefs and the OS permission allow it. Best-effort: any failure is logged, not
/// raised. Skipped when the window is focused — the in-app refresh status line
/// already reports foreground refreshes, so this is the *background* channel.
pub fn notify_refresh(handle: &AppHandle, core: &CoreHandle, outcomes: &[RefreshOutcomeDto]) {
    let prefs = NotifyPrefs::read(core);
    if !prefs.enabled || window_focused(handle) {
        return;
    }
    let Some(body) = plan(outcomes, &prefs, Local::now().time()) else {
        return;
    };
    if !matches!(
        handle.notification().permission_state(),
        Ok(PermissionState::Granted)
    ) {
        return;
    }
    if let Err(error) = handle
        .notification()
        .builder()
        .title("Curio")
        .body(body)
        .show()
    {
        tracing::warn!(%error, "failed to show refresh notification");
    }
}

/// Requests OS notification permission (the OS shows its prompt on first ask)
/// and returns whether it is granted. The Settings notification toggle calls
/// this when the user turns notifications on, so the prompt lands at that moment
/// rather than at some later background refresh.
///
/// # Errors
///
/// Returns an internal error if the permission request itself fails.
#[tauri::command]
#[specta::specta]
pub async fn request_notification_permission(app: AppHandle) -> Result<bool, CommandError> {
    let state = app
        .notification()
        .request_permission()
        .map_err(|error| CommandError::internal(ErrorCode::Internal, error.to_string()))?;
    Ok(state == PermissionState::Granted)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn prefs(enabled: bool) -> NotifyPrefs {
        NotifyPrefs {
            enabled,
            new_articles: true,
            errors: true,
            feed_dead: true,
            quiet: None,
        }
    }

    fn outcome(
        new_articles: u64,
        status: FetchStatusDto,
        http_status: Option<u16>,
    ) -> RefreshOutcomeDto {
        RefreshOutcomeDto {
            feed_id: 1,
            status,
            http_status,
            new_articles,
            updated_articles: 0,
            error: None,
        }
    }

    fn noon() -> NaiveTime {
        NaiveTime::from_hms_opt(12, 0, 0).unwrap()
    }

    #[test]
    fn master_off_plans_nothing() {
        let out = [outcome(5, FetchStatusDto::Ok, Some(200))];
        assert_eq!(plan(&out, &prefs(false), noon()), None);
    }

    #[test]
    fn summarizes_new_articles_across_feeds() {
        let out = [
            outcome(3, FetchStatusDto::Ok, Some(200)),
            outcome(1, FetchStatusDto::Ok, Some(200)),
            outcome(0, FetchStatusDto::NotModified, Some(304)),
        ];
        assert_eq!(
            plan(&out, &prefs(true), noon()),
            Some("4 new articles".to_owned())
        );
    }

    #[test]
    fn singular_grammar_for_one_article() {
        let out = [outcome(1, FetchStatusDto::Ok, Some(200))];
        assert_eq!(
            plan(&out, &prefs(true), noon()),
            Some("1 new article".to_owned())
        );
    }

    #[test]
    fn separates_errors_from_dead_feeds() {
        let out = [
            outcome(0, FetchStatusDto::Error, Some(500)),
            outcome(0, FetchStatusDto::Error, Some(GONE)),
            outcome(2, FetchStatusDto::Ok, Some(200)),
        ];
        assert_eq!(
            plan(&out, &prefs(true), noon()),
            Some("2 new articles · 1 feed failed to refresh · 1 feed went dead".to_owned())
        );
    }

    #[test]
    fn a_disabled_category_is_omitted() {
        let mut p = prefs(true);
        p.new_articles = false;
        let out = [
            outcome(5, FetchStatusDto::Ok, Some(200)),
            outcome(0, FetchStatusDto::Error, Some(GONE)),
        ];
        assert_eq!(plan(&out, &p, noon()), Some("1 feed went dead".to_owned()));
    }

    #[test]
    fn nothing_notable_plans_nothing() {
        let out = [outcome(0, FetchStatusDto::NotModified, Some(304))];
        assert_eq!(plan(&out, &prefs(true), noon()), None);
    }

    #[test]
    fn quiet_hours_suppress() {
        let mut p = prefs(true);
        p.quiet = Some((
            NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
        ));
        let out = [outcome(5, FetchStatusDto::Ok, Some(200))];
        // 23:00 is inside the wrap-around window → suppressed.
        assert_eq!(
            plan(&out, &p, NaiveTime::from_hms_opt(23, 0, 0).unwrap()),
            None
        );
        // 12:00 is outside → allowed.
        assert!(plan(&out, &p, noon()).is_some());
    }

    #[test]
    fn quiet_window_wrap_and_same_day() {
        let wrap = Some((
            NaiveTime::from_hms_opt(22, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(7, 0, 0).unwrap(),
        ));
        assert!(in_quiet(NaiveTime::from_hms_opt(2, 0, 0).unwrap(), wrap));
        assert!(!in_quiet(NaiveTime::from_hms_opt(8, 0, 0).unwrap(), wrap));

        let same_day = Some((
            NaiveTime::from_hms_opt(9, 0, 0).unwrap(),
            NaiveTime::from_hms_opt(17, 0, 0).unwrap(),
        ));
        assert!(in_quiet(noon(), same_day));
        assert!(!in_quiet(
            NaiveTime::from_hms_opt(20, 0, 0).unwrap(),
            same_day
        ));

        // An empty window (start == end) is never quiet.
        let empty = Some((noon(), noon()));
        assert!(!in_quiet(noon(), empty));
    }
}
