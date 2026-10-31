//! Workspace automation tasks. Run via `cargo run -p xtask -- <task>`.
//!
//! Tasks:
//! - `boundary` — assert that `curio-core`'s normal dependency tree is
//!   webview-free (no tauri/wry/tao). `deny.toml` is the first line of
//!   defense; this is defense in depth, and it is what CI runs.
//! - `workflows` — the supply-chain lint over `.github/workflows`: SHA-pinned
//!   actions, the release gated on CI, the updater signing key confined to
//!   one build-free step of a job in the `updater-signing` environment,
//!   checksums/SBOM/attestation on the draft release,
//!   no retired runner images, a nightly that runs on the PRs editing it,
//!   and a cold-start bench the regression tracker can parse (see
//!   `xtask::workflows`).
//! - `bench-output <file> [--expect <bench-id>]…` — check a bencher-format
//!   `cargo bench` log before the nightly hands it to the regression tracker:
//!   no criterion error line, a result line the tracker can parse, and each
//!   `--expect`ed benchmark present (see `xtask::bench`).

use std::process::{Command, ExitCode};

use xtask::banned_crates_in_tree;

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    match task.as_deref() {
        Some("boundary") => boundary(),
        Some("workflows") => workflows(),
        Some("bench-output") => bench_output(std::env::args().skip(2).collect()),
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
    eprintln!(
        "usage: cargo run -p xtask -- <boundary|workflows|bench-output <file> [--expect <bench-id>]...>"
    );
}

fn bench_output(args: Vec<String>) -> ExitCode {
    let mut file = None;
    let mut expected = Vec::new();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        if arg == "--expect" {
            let Some(name) = args.next() else {
                eprintln!("xtask bench-output: `--expect` needs a benchmark id");
                return ExitCode::FAILURE;
            };
            expected.push(name);
        } else if file.is_none() {
            file = Some(arg);
        } else {
            eprintln!("xtask bench-output: unexpected argument `{arg}`");
            usage();
            return ExitCode::FAILURE;
        }
    }
    let Some(file) = file else {
        usage();
        return ExitCode::FAILURE;
    };
    let output = match std::fs::read_to_string(&file) {
        Ok(output) => output,
        Err(err) => {
            eprintln!("xtask bench-output: cannot read {file}: {err}");
            return ExitCode::FAILURE;
        }
    };
    let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
    match xtask::bench::check(&output, &expected) {
        Ok(results) => {
            for result in &results {
                println!(
                    "xtask bench-output: {} = {} {} (+/- {})",
                    result.name, result.value, result.unit, result.range
                );
            }
            println!("xtask bench-output: OK — {file} is parseable by the tracker");
            ExitCode::SUCCESS
        }
        Err(problems) => {
            eprintln!("xtask bench-output: {file} is not trackable:");
            for problem in &problems {
                eprintln!("  - {problem}");
            }
            ExitCode::FAILURE
        }
    }
}

fn workflows() -> ExitCode {
    // The repo root comes from the run-time CARGO_MANIFEST_DIR: a binary
    // reused from another worktree's shared target dir must lint this
    // worktree's workflows, not that one's (issue #183).
    let dir = match xtask::paths::repo_root() {
        Ok(root) => root.join(".github/workflows"),
        Err(err) => {
            eprintln!("xtask workflows: {err}");
            return ExitCode::FAILURE;
        }
    };
    match xtask::workflows::lint_dir(&dir) {
        Ok(violations) if violations.is_empty() => {
            println!(
                "xtask workflows: OK — actions SHA-pinned, release gated on CI, signing key isolated, release assets complete, no retired runners, nightly self-validating, bench trackable"
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
