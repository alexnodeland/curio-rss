//! Workspace automation tasks. Run via `cargo run -p xtask -- <task>`.
//!
//! Tasks:
//! - `boundary` — assert that `curio-core`'s normal dependency tree is
//!   webview-free (no tauri/wry/tao). `deny.toml` is the first line of
//!   defense; this is defense in depth, and it is what CI runs.

use std::process::{Command, ExitCode};

/// Crate names that must never appear in curio-core's dependency tree.
/// The webview boundary lives in `apps/desktop`, outside the workspace,
/// until Phase 4 deliberately moves it.
const BANNED: &[&str] = &["tauri", "wry", "tao"];

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("boundary") => boundary(),
        Some(other) => {
            eprintln!("xtask: unknown task `{other}`");
            usage();
            ExitCode::FAILURE
        }
        None => {
            usage();
            ExitCode::FAILURE
        }
    }
}

fn usage() {
    eprintln!("usage: cargo run -p xtask -- boundary");
}

fn boundary() -> ExitCode {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = match Command::new(cargo)
        .args([
            "tree",
            "-p",
            "curio-core",
            "-e",
            "normal",
            "--prefix",
            "none",
        ])
        .output()
    {
        Ok(output) => output,
        Err(err) => {
            eprintln!("xtask boundary: failed to run cargo tree: {err}");
            return ExitCode::FAILURE;
        }
    };

    if !output.status.success() {
        eprintln!("xtask boundary: cargo tree failed:");
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        return ExitCode::FAILURE;
    }

    let tree = String::from_utf8_lossy(&output.stdout);
    let violations: Vec<&str> = tree
        .lines()
        .filter_map(|line| line.split_whitespace().next())
        .filter(|name| {
            BANNED
                .iter()
                .any(|banned| name == banned || name.starts_with(&format!("{banned}-")))
        })
        .collect();

    if violations.is_empty() {
        println!("xtask boundary: OK — curio-core's dependency tree is webview-free");
        ExitCode::SUCCESS
    } else {
        eprintln!(
            "xtask boundary: VIOLATION — curio-core's dependency tree contains: {}",
            violations.join(", ")
        );
        eprintln!(
            "curio-core must stay Tauri-free; the webview boundary is apps/desktop (Phase 4)."
        );
        ExitCode::FAILURE
    }
}
