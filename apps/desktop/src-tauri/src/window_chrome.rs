//! Native-feeling window chrome (#148).
//!
//! On macOS the main window's `tauri.conf.json` entry asks for an overlay
//! title bar (`titleBarStyle: "Overlay"`, `hiddenTitle`, a tuned
//! `trafficLightPosition`): the three pane headers become the title bar, so
//! there is no stacked OS title bar above them. Linux and Windows ignore
//! those keys and keep their native decorations.
//!
//! The frontend has to know which chrome it got — it insets the leading
//! header past the traffic lights and turns the headers into drag regions
//! only when the title bar is overlaid — and when the window goes
//! fullscreen, where macOS hides the traffic lights and the inset drops to
//! zero. [`window_chrome`] answers both at startup (and after a webview
//! reload); [`on_window_event`] emits [`FullscreenChanged`] whenever the
//! fullscreen state flips.

use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use tauri::utils::config::WindowConfig;
use tauri::{Manager as _, TitleBarStyle};
use tauri_specta::Event;

use crate::error::CommandError;
use crate::events::emit_or_log;

/// What chrome the main window was built with, for the frontend's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct WindowChromeDto {
    /// The OS title bar is overlaid on the webview (macOS): the traffic
    /// lights sit inside the leading pane header, and the headers must act
    /// as the window's drag regions. `false` = native decorations.
    pub overlay_title_bar: bool,
    /// The window is fullscreen (macOS hides the traffic lights there).
    pub fullscreen: bool,
}

/// The main window entered or left fullscreen. Fires on change only;
/// [`window_chrome`] answers the current state.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, specta::Type, Event)]
pub struct FullscreenChanged {
    /// Whether the window is now fullscreen.
    pub fullscreen: bool,
}

/// Whether a window built from `config` gets an overlay title bar. Only
/// macOS honours `titleBarStyle`, so elsewhere it is always `false` —
/// `macos` is the build target, a parameter so both answers are testable.
#[must_use]
pub fn overlay_title_bar(config: &WindowConfig, macos: bool) -> bool {
    macos && config.decorations && config.title_bar_style == TitleBarStyle::Overlay
}

/// The last fullscreen state seen, so a resize only emits on a real flip.
#[derive(Debug, Default)]
pub struct FullscreenTracker(AtomicBool);

impl FullscreenTracker {
    /// Records `fullscreen`; `true` when it differs from the last state.
    pub fn observe(&self, fullscreen: bool) -> bool {
        self.0.swap(fullscreen, Ordering::AcqRel) != fullscreen
    }
}

/// The main window's chrome: overlay title bar or native, and fullscreen.
///
/// # Errors
///
/// Never — a fullscreen state the OS will not report reads as windowed
/// (logged). The `Result` keeps the generated wrapper on the same typed
/// `{ status, data | error }` shape as every other command.
#[tauri::command]
#[specta::specta]
// Async like every command here: Tauri runs a sync command on the main
// thread, and nothing here awaits.
#[allow(clippy::unused_async)]
pub async fn window_chrome(window: tauri::WebviewWindow) -> Result<WindowChromeDto, CommandError> {
    let overlay = window
        .config()
        .app
        .windows
        .iter()
        .find(|config| config.label == window.label())
        .is_some_and(|config| overlay_title_bar(config, cfg!(target_os = "macos")));
    let fullscreen = fullscreen_or_windowed(&window);
    Ok(WindowChromeDto {
        overlay_title_bar: overlay,
        fullscreen,
    })
}

/// The window's fullscreen state, recorded as the tracker's baseline (a
/// reloaded webview re-seeds it). One the OS will not report reads as
/// windowed (logged) — the inset stays, the safe side.
fn fullscreen_or_windowed(window: &tauri::WebviewWindow) -> bool {
    let fullscreen = window.is_fullscreen().unwrap_or_else(|error| {
        tracing::warn!(%error, "could not read the window's fullscreen state");
        false
    });
    if let Some(tracker) = window.try_state::<FullscreenTracker>() {
        tracker.observe(fullscreen);
    }
    fullscreen
}

/// Watches the main window's resizes (entering or leaving fullscreen is
/// one) and emits [`FullscreenChanged`] when the state flips.
pub fn on_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if !matches!(event, tauri::WindowEvent::Resized(_)) || window.label() != crate::MAIN_WINDOW {
        return;
    }
    let Some(tracker) = window.try_state::<FullscreenTracker>() else {
        return;
    };
    match window.is_fullscreen() {
        Ok(fullscreen) => {
            if tracker.observe(fullscreen) {
                emit_or_log(window.app_handle(), &FullscreenChanged { fullscreen });
            }
        }
        Err(error) => tracing::warn!(%error, "could not read the window's fullscreen state"),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)] // test assertions
mod tests {
    use super::*;

    /// The checked-in config, read at run time (never `env!`, see #183).
    fn read(relative: &str) -> serde_json::Value {
        let dir = std::env::var_os("CARGO_MANIFEST_DIR").unwrap();
        let text = std::fs::read_to_string(std::path::Path::new(&dir).join(relative)).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn main_window() -> WindowConfig {
        let config = read("tauri.conf.json");
        let main = config["app"]["windows"]
            .as_array()
            .unwrap()
            .iter()
            .find(|window| window["label"] == crate::MAIN_WINDOW)
            .cloned()
            .unwrap();
        serde_json::from_value(main).unwrap()
    }

    #[test]
    fn the_main_window_overlays_its_title_bar_on_macos_only() {
        let main = main_window();
        assert!(overlay_title_bar(&main, true), "macOS: overlay title bar");
        assert!(
            !overlay_title_bar(&main, false),
            "Linux/Windows: native decorations"
        );
        assert!(main.hidden_title, "no title text over the pane headers");
        assert!(main.decorations, "the traffic lights need decorations on");
    }

    #[test]
    fn the_traffic_lights_sit_centred_in_the_pane_header() {
        // `y` is the buttons' vertical centre for the 52 px `--header-height`
        // (app.css); `x` matches the header's 16 px leading padding, which
        // the frontend's `--titlebar-inset` clears.
        let position = main_window().traffic_light_position.unwrap();
        assert!((position.x - 16.0).abs() < f64::EPSILON);
        assert!((position.y - 26.0).abs() < f64::EPSILON);
    }

    #[test]
    fn a_native_title_bar_is_never_reported_as_overlaid() {
        let mut main = main_window();
        main.title_bar_style = TitleBarStyle::Visible;
        assert!(!overlay_title_bar(&main, true));
        let mut undecorated = main_window();
        undecorated.decorations = false;
        assert!(!overlay_title_bar(&undecorated, true));
    }

    /// The drag regions call `start_dragging`, which `core:default` does
    /// not grant; double-click zoom's `internal_toggle_maximize` it does.
    #[test]
    fn the_capability_lets_drag_regions_move_the_window() {
        let capability = read("capabilities/main.json");
        let permissions = capability["permissions"].as_array().unwrap();
        assert!(
            permissions
                .iter()
                .any(|permission| permission == "core:window:allow-start-dragging"),
            "main.json grants core:window:allow-start-dragging"
        );
    }

    #[test]
    fn the_tracker_reports_only_real_flips() {
        let tracker = FullscreenTracker::default();
        assert!(!tracker.observe(false), "starts windowed");
        assert!(tracker.observe(true), "entered fullscreen");
        assert!(!tracker.observe(true), "a resize while fullscreen");
        assert!(tracker.observe(false), "left fullscreen");
    }
}
