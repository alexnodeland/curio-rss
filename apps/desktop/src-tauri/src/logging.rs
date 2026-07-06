//! Local, zero-telemetry diagnostics: a panic hook and rotating logs.
//!
//! Tracing is written to day-rotated files under `<profile>/logs/`
//! (`curio.log.<date>`) — exactly the files [`crate::commands::platform`]'s
//! `copy_diagnostics_bundle` allowlists. Nothing is transmitted anywhere;
//! the user copies the redacted bundle by hand if they want to file a bug.
//!
//! The panic hook routes panics into the same log (and still calls the
//! previous hook so stderr keeps working in development).

use std::path::Path;

use tracing_subscriber::EnvFilter;

/// Installs the rotating-file tracing subscriber and the panic hook.
/// Idempotent-ish: a second global-default set (e.g. in a test) is a no-op,
/// and failure to create the log dir degrades to no file logging rather
/// than aborting startup.
pub fn init(logs_dir: &Path) {
    if let Err(error) = std::fs::create_dir_all(logs_dir) {
        eprintln!(
            "curio-desktop: no log dir at {} ({error}); continuing without file logs",
            logs_dir.display()
        );
        return;
    }

    // Daily rotation keeps any single file small; old days age out via the
    // event-log retention sweep's sibling housekeeping (a reader logs little).
    let appender = tracing_appender::rolling::daily(logs_dir, "curio.log");
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let installed = tracing_subscriber::fmt()
        .with_writer(appender)
        .with_ansi(false)
        .with_target(false)
        .with_env_filter(filter)
        .try_init()
        .is_ok();

    if installed {
        install_panic_hook();
    }
}

/// Routes panics into the log before deferring to the previous hook.
fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info.location().map_or_else(
            || "unknown".to_owned(),
            |loc| format!("{}:{}", loc.file(), loc.line()),
        );
        tracing::error!(target: "panic", %location, "panic: {}", payload(info));
        previous(info);
    }));
}

/// Best-effort human string for a panic payload.
fn payload(info: &std::panic::PanicHookInfo<'_>) -> String {
    let any = info.payload();
    if let Some(message) = any.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = any.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_owned()
    }
}
