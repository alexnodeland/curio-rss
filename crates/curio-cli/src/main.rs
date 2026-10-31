//! `curio` — the CLI head over [`curio_core`].
//!
//! Thin by charter: every command parses arguments, calls one or two
//! [`curio_core::CoreHandle`] methods, and prints
//! either human lines or (with `--json`) exactly one JSON document on
//! stdout. Diagnostics go to stderr, driven by `RUST_LOG`.

mod app;
mod cli;
mod commands;
mod config;
mod output;
mod resolve;

use std::process::ExitCode;

use clap::Parser as _;

/// Exit status when another Curio process (the desktop app, another cron
/// job) holds the profile past the wait — `EX_TEMPFAIL` from sysexits(3):
/// nothing is wrong, retry later. Scripts can tell it apart from a real
/// failure (status 1).
const EXIT_PROFILE_BUSY: u8 = 75;

fn main() -> ExitCode {
    init_tracing();
    let cli = cli::Cli::parse();
    match commands::run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::from(failure_status(&err))
        }
    }
}

/// The exit status for a failed command: [`EXIT_PROFILE_BUSY`] for
/// contention with another process on the profile, 1 for everything else.
fn failure_status(err: &anyhow::Error) -> u8 {
    let busy = err.chain().any(|cause| {
        cause
            .downcast_ref::<curio_core::CoreError>()
            .is_some_and(curio_core::CoreError::is_profile_busy)
            || cause
                .downcast_ref::<curio_core::storage::StorageError>()
                .is_some_and(curio_core::storage::StorageError::is_busy)
    });
    if busy { EXIT_PROFILE_BUSY } else { 1 }
}

/// stderr diagnostics, `RUST_LOG`-driven (`warn` when unset).
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contention_gets_its_own_exit_status() {
        let locked = curio_core::CoreError::Events(curio_core::events::EventsError::Log(
            curio_core::events::EventLogError::ProfileLocked {
                path: "events.lock".into(),
                waited_ms: 15_000,
            },
        ));
        let wrapped = anyhow::Error::new(locked).context("starring 42");
        assert_eq!(failure_status(&wrapped), EXIT_PROFILE_BUSY);
        let other = anyhow::Error::new(curio_core::CoreError::NotFound { entity: "article" });
        assert_eq!(failure_status(&other), 1);
        assert_eq!(failure_status(&anyhow::anyhow!("plain")), 1);
    }
}
