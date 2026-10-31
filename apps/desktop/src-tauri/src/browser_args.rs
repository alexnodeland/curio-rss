//! `WebView2` browser arguments for the main window (Windows only at runtime).
//!
//! `WebView2` automation — `msedgedriver`, which `tauri-driver` fronts for the
//! nightly e2e smoke — launches the app with its `--remote-debugging-port` /
//! `--user-data-dir` flags in the `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS` env
//! var, then waits for the browser's `DevToolsActivePort` file. But wry always
//! hands `WebView2` an explicit `AdditionalBrowserArguments` (its defaults
//! below), and the runtime then ignores the env var: the browser starts
//! without the debugging port and every session fails with "`DevToolsActivePort`
//! file doesn't exist" (the nightly's Windows smoke, issue #175). So debug
//! builds — the ones the smoke drives — forward the env var's flags
//! themselves, after wry's defaults. Release builds never take injected
//! browser flags.

/// The env var `WebView2` automation injects its browser flags through.
pub const AUTOMATION_ENV: &str = "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS";

/// What wry passes when a window sets no browser arguments (wry
/// `src/webview2/mod.rs`): the selection mini-menu, PDF mini-menu and
/// `SmartScreen` features off, and gesture-free autoplay (wry's default).
/// Setting any arguments replaces these, so they are restated here.
pub const WRY_DEFAULT_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required";

/// The main window's browser arguments given the [`AUTOMATION_ENV`] value:
/// `None` (keep wry's defaults untouched) when nothing is injected, else
/// wry's defaults followed by the injected flags — later flags win in
/// Chromium, so the driver's `--user-data-dir` is where the browser writes
/// `DevToolsActivePort`.
#[must_use]
pub fn with_injected(injected: Option<&str>) -> Option<String> {
    let injected = injected.map(str::trim).filter(|args| !args.is_empty())?;
    Some(format!("{WRY_DEFAULT_ARGS} {injected}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_injected_keeps_wry_defaults() {
        assert_eq!(with_injected(None), None);
        assert_eq!(with_injected(Some("")), None);
        assert_eq!(with_injected(Some("   ")), None);
    }

    #[test]
    fn injected_flags_follow_wry_defaults() {
        let args = with_injected(Some(" --remote-debugging-port=0 --user-data-dir=C:\\t\\d "));
        assert_eq!(
            args.as_deref(),
            Some(
                "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection \
                 --autoplay-policy=no-user-gesture-required \
                 --remote-debugging-port=0 --user-data-dir=C:\\t\\d"
            )
        );
    }

    #[test]
    fn the_defaults_keep_every_feature_wry_turns_off() {
        for feature in ["msWebOOUI", "msPdfOOUI", "msSmartScreenProtection"] {
            assert!(WRY_DEFAULT_ARGS.contains(feature), "{feature}");
        }
    }
}
