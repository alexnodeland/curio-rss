//! The fuzz-coverage half of the `workflows` lint (issue #133): every
//! cargo-fuzz target in `fuzz/Cargo.toml` is actually fuzzed by the nightly,
//! in a bounded run that can fail the job.
//!
//! The rules, over `nightly.yml`:
//!
//! 1. Some job has a step whose `run:` invokes `cargo fuzz run`.
//! 2. That job's `strategy.matrix` names every `[[bin]]` target of the fuzz
//!    crate — a target added to `fuzz/` without a nightly leg is never run.
//! 3. The run is bounded (`-max_total_time=`): libFuzzer otherwise runs
//!    until the job's six-hour timeout.
//! 4. Neither the job nor the step carries an `if:` — the job runs on every
//!    trigger of the nightly, schedule included. (`continue-on-error` is
//!    already banned nightly-wide by the main lint, so a crash, which makes
//!    libFuzzer exit non-zero, fails the job.)

use std::io;
use std::path::Path;

use crate::yaml::{self, Value};

/// The fuzz crate's manifest, relative to the repo root.
pub const FUZZ_MANIFEST: &str = "fuzz/Cargo.toml";

/// The nightly workflow, relative to the repo root.
const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// The `[[bin]]` target names declared in a cargo-fuzz manifest, in order.
///
/// A line scan, not a TOML parse (xtask keeps saphyr-parser as its only
/// runtime dependency): each `name = "…"` that follows a `[[bin]]` header,
/// before the next table header.
#[must_use]
pub fn targets(manifest: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
            continue;
        }
        if !in_bin {
            continue;
        }
        if let Some(value) = line
            .strip_prefix("name")
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix('='))
        {
            out.push(value.trim().trim_matches('"').to_owned());
            in_bin = false;
        }
    }
    out
}

/// Check `nightly` (the workflow source) against the fuzz `manifest`; one
/// message per violation.
#[must_use]
pub fn lint(nightly: &str, manifest: &str) -> Vec<String> {
    let targets = targets(manifest);
    if targets.is_empty() {
        return vec![format!("{FUZZ_MANIFEST}: declares no [[bin]] fuzz targets")];
    }
    let doc = match yaml::from_str(nightly) {
        Ok(doc) => doc,
        Err(err) => return vec![format!("nightly.yml: not valid YAML: {err}")],
    };
    let jobs = doc
        .get("jobs")
        .and_then(Value::as_mapping)
        .map(|jobs| jobs.iter().collect::<Vec<_>>())
        .unwrap_or_default();

    let mut out = Vec::new();
    let mut found = false;
    for (job_id, job) in jobs {
        let job_id = job_id.as_str().unwrap_or("?");
        let steps = job
            .get("steps")
            .and_then(Value::as_sequence)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for step in steps {
            let Some(run) = step.get("run").and_then(Value::as_str) else {
                continue;
            };
            if !run.contains("cargo fuzz run") {
                continue;
            }
            found = true;
            if !run.contains("-max_total_time=") {
                out.push(format!(
                    "nightly.yml: job `{job_id}` runs `cargo fuzz run` without `-max_total_time=` — an unbounded run only ends at the job timeout"
                ));
            }
            if step.get("if").is_some() {
                out.push(format!(
                    "nightly.yml: job `{job_id}`'s `cargo fuzz run` step has an `if:` — the fuzz run must happen on every nightly"
                ));
            }
            if job.get("if").is_some() {
                out.push(format!(
                    "nightly.yml: job `{job_id}` has an `if:` — the fuzz job must run on every nightly trigger"
                ));
            }
            let mut matrix = Vec::new();
            if let Some(node) = job.get("strategy").and_then(|s| s.get("matrix")) {
                strings(node, &mut matrix);
            }
            for target in &targets {
                if !matrix.contains(&target.as_str()) {
                    out.push(format!(
                        "nightly.yml: fuzz target `{target}` ({FUZZ_MANIFEST}) has no leg in job `{job_id}`'s matrix — it is never fuzzed"
                    ));
                }
            }
        }
    }
    if !found {
        out.push(format!(
            "nightly.yml: no job runs `cargo fuzz run` — the {} target(s) in {FUZZ_MANIFEST} are never fuzzed",
            targets.len()
        ));
    }
    out
}

/// [`lint`] over the files under `root`.
///
/// # Errors
///
/// Reading either file fails (a missing `fuzz/Cargo.toml` included — the
/// fuzz targets are part of the gate).
pub fn lint_repo(root: &Path) -> io::Result<Vec<String>> {
    let nightly = std::fs::read_to_string(root.join(NIGHTLY))?;
    let manifest = std::fs::read_to_string(root.join(FUZZ_MANIFEST))?;
    Ok(lint(&nightly, &manifest))
}

/// Every string scalar under `node`.
fn strings<'v>(node: &'v Value, out: &mut Vec<&'v str>) {
    match node {
        Value::String(text) => out.push(text),
        Value::Sequence(items) => items.iter().for_each(|item| strings(item, out)),
        Value::Mapping(map) => map.values().for_each(|item| strings(item, out)),
        Value::Tagged(tagged) => strings(&tagged.value, out),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const MANIFEST: &str = r#"[package]
name = "curio-fuzz"

[dependencies]
libfuzzer-sys = "0.4"

[[bin]]
name = "feed_parse"
path = "fuzz_targets/feed_parse.rs"

[[bin]]
path = "fuzz_targets/opml.rs"
name="opml"
"#;

    const NIGHTLY: &str = r#"on:
  schedule:
    - cron: "0 7 * * *"
jobs:
  fuzz:
    runs-on: ubuntu-22.04
    strategy:
      matrix:
        include:
          - { target: feed_parse, seeds: fixtures/feeds }
          - { target: opml, seeds: fixtures/opml }
    steps:
      - run: |
          cargo fuzz run "${{ matrix.target }}" -- -max_total_time=600
"#;

    #[test]
    fn targets_reads_every_bin_name_and_nothing_else() {
        assert_eq!(targets(MANIFEST), ["feed_parse", "opml"]);
    }

    #[test]
    fn a_nightly_that_fuzzes_every_target_is_clean() {
        assert_eq!(lint(NIGHTLY, MANIFEST), Vec::<String>::new());
    }

    #[test]
    fn a_target_without_a_matrix_leg_is_flagged() {
        let nightly = NIGHTLY.replace("          - { target: opml, seeds: fixtures/opml }\n", "");
        let violations = lint(&nightly, MANIFEST);
        assert_eq!(violations.len(), 1, "{violations:?}");
        assert!(
            violations[0].contains("fuzz target `opml`"),
            "{violations:?}"
        );
    }

    #[test]
    fn a_nightly_without_a_fuzz_run_is_flagged() {
        let nightly = NIGHTLY.replace("cargo fuzz run", "cargo build");
        let violations = lint(&nightly, MANIFEST);
        assert!(
            violations[0].contains("no job runs `cargo fuzz run`"),
            "{violations:?}"
        );
    }

    #[test]
    fn an_unbounded_or_conditional_fuzz_run_is_flagged() {
        let unbounded = NIGHTLY.replace(" -- -max_total_time=600", "");
        assert!(lint(&unbounded, MANIFEST)[0].contains("without `-max_total_time=`"));

        let gated = NIGHTLY.replace(
            "    runs-on: ubuntu-22.04\n",
            "    if: github.event_name == 'pull_request'\n    runs-on: ubuntu-22.04\n",
        );
        assert!(lint(&gated, MANIFEST)[0].contains("has an `if:`"));

        let step_gated = NIGHTLY.replace("      - run: |", "      - if: false\n        run: |");
        assert!(lint(&step_gated, MANIFEST)[0].contains("step has an `if:`"));
    }

    #[test]
    fn a_manifest_without_targets_is_flagged() {
        assert!(lint(NIGHTLY, "[package]\nname = \"x\"\n")[0].contains("declares no [[bin]]"));
    }

    /// The real repo: every target in fuzz/ has a bounded, unconditional
    /// nightly leg. (Paths resolved at run time — issue #183.)
    #[test]
    fn the_repo_fuzzes_every_target_nightly() {
        let root = crate::paths::repo_root().unwrap();
        let violations = lint_repo(&root).unwrap();
        assert!(violations.is_empty(), "{violations:#?}");
        let manifest = std::fs::read_to_string(root.join(FUZZ_MANIFEST)).unwrap();
        assert!(
            targets(&manifest).len() >= 4,
            "feed parse, OPML, content, URL screening"
        );
    }
}
