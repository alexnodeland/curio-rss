//! `curio` — the CLI head over [`curio_core`].
//!
//! Thin by charter: every command parses arguments, calls one or two
//! [`curio_core::CoreHandle`](curio_core::CoreHandle) methods, and prints
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

fn main() -> ExitCode {
    init_tracing();
    let cli = cli::Cli::parse();
    match commands::run(cli) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
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
