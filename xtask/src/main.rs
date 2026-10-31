//! Workspace automation tasks. Run via `cargo run -p xtask -- <task>`.
//!
//! Tasks:
//! - `boundary` — assert that `curio-core`'s normal dependency tree is
//!   webview-free (no tauri/wry/tao). `deny.toml` is the first line of
//!   defense; this is defense in depth, and it is what CI runs.
//! - `workflows` — the supply-chain lint over `.github/workflows`: SHA-pinned
//!   actions, the release gated on CI, the updater signing key confined to
//!   one build-free step, checksums/SBOM/attestation on the draft release,
//!   no retired runner images, and a nightly that runs on the PRs editing it
//!   (see `xtask::workflows`).

use std::path::Path;
use std::process::{Command, ExitCode};

use xtask::banned_crates_in_tree;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("boundary") => boundary(),
        Some("workflows") => workflows(),
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
    eprintln!("usage: cargo run -p xtask -- <boundary|workflows>");
}

fn workflows() -> ExitCode {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows");
    match xtask::workflows::lint_dir(&dir) {
        Ok(violations) if violations.is_empty() => {
            println!(
                "xtask workflows: OK — actions SHA-pinned, release gated on CI, signing key isolated, release assets complete, no retired runners, nightly self-validating"
            );
            ExitCode::SUCCESS
        }
        Ok(violations) => {
            eprintln!("xtask workflows: {} violation(s):", violations.len());
            for violation in &violations {
                eprintln!("  - {violation}");
            }
            ExitCode::FAILURE
        }
        Err(err) => {
            eprintln!("xtask workflows: cannot read {}: {err}", dir.display());
            ExitCode::FAILURE
        }
    }
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
    let violations = banned_crates_in_tree(&tree);

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
