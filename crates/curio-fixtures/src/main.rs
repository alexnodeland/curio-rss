//! `curio-fixtures` — the `just fixtures` entry point.
//!
//! Generates the deterministic performance fixture (default: 1000 feeds /
//! 50 000 articles) into `fixtures/generated/curio-50k` and prints its
//! SHA-256, size, and counts. The output directory is git-ignored
//! (`/fixtures/generated/`) so the multi-megabyte database never lands in
//! history and never trips the 1 MB blob guard.
//!
//! Usage:
//!   curio-fixtures [--out DIR] [--seed N] [--feeds N] [--articles N]

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use curio_fixtures::{FixtureSpec, generate};

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("curio-fixtures: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut spec = FixtureSpec::perf_50k();
    let mut out = PathBuf::from("fixtures/generated/curio-50k");

    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{flag} needs a value"));
        match flag.as_str() {
            "--out" => out = PathBuf::from(value()?),
            "--seed" => spec.seed = parse(&flag, &value()?)?,
            "--feeds" => spec.feeds = parse(&flag, &value()?)?,
            "--articles" => spec.articles = parse(&flag, &value()?)?,
            "-h" | "--help" => {
                println!("usage: curio-fixtures [--out DIR] [--seed N] [--feeds N] [--articles N]");
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    eprintln!(
        "generating {} articles across {} feeds (seed {:#018x}) into {}",
        spec.articles,
        spec.feeds,
        spec.seed,
        out.display(),
    );
    let started = Instant::now();
    let report = generate(&out, &spec).map_err(|e| e.to_string())?;
    let elapsed = started.elapsed();

    let mib = report.bytes / (1024 * 1024);
    let kib = (report.bytes % (1024 * 1024)) / 1024;
    println!("db      : {}", report.db_path.display());
    println!("feeds   : {}", report.feeds);
    println!("articles: {}", report.articles);
    println!("size    : {mib}.{kib:03} MiB ({} bytes)", report.bytes);
    println!("sha256  : {}", report.sha256);
    println!("elapsed : {:.2}s", elapsed.as_secs_f64());
    Ok(())
}

fn parse<T>(flag: &str, raw: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    raw.parse()
        .map_err(|e| format!("{flag}: invalid value {raw:?}: {e}"))
}
