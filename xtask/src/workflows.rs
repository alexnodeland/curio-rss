//! Supply-chain lint for `.github/workflows` (`cargo run -p xtask -- workflows`).
//!
//! Holds the release pipeline to the shape issue #174 asked for, so a later
//! edit cannot quietly undo it:
//!
//! 1. **Pinned actions.** Every `uses:` is a local `./` path, a
//!    `docker://…@sha256:` digest, or `owner/repo@<40-hex commit SHA>` with a
//!    trailing `# vX.Y.Z` comment (what dependabot bumps).
//! 2. **Release gate.** `ci.yml` is callable (`workflow_call`), `release.yml`
//!    calls it, and every other release job transitively `needs` that call
//!    with no `always()` / `failure()` / `cancelled()` in its `if:` — so a
//!    tag on a commit with red CI never reaches a draft release.
//! 3. **Signing-key isolation.** The updater signing secrets are referenced
//!    from exactly one step's `env`, across all workflows. That step runs a
//!    `signer sign` command and no build tool; its job installs npm packages
//!    only with `--ignore-scripts`, compiles nothing, and uses only the
//!    first-party artifact plumbing actions. Ahead of the key step the job
//!    runs nothing but that install (no build tool, node or npx, no
//!    background process), and up to and including it nothing writes the
//!    runner's env/path/output files or sets a code-injecting env var
//!    (`BASH_ENV`, `NODE_OPTIONS`, …). No `secrets: inherit` or
//!    `toJSON(secrets)` anywhere (both hand every secret onward). The
//!    signing job deploys to the `updater-signing` environment (issue #187),
//!    the only place the key is to be stored, tag-restricted and
//!    reviewer-gated, whenever the key step runs: unconditionally, or (in a
//!    workflow that a `workflow_dispatch` dry run also triggers) under
//!    exactly the key step's own `if:`, so the dry run is never held at the
//!    tag-only rule.
//! 4. **Release assets.** The draft release step ships `SHA256SUMS`, a
//!    `CycloneDX` SBOM and the provenance attestation bundle, and its job mints
//!    that attestation first.
//! 5. **A nightly that runs** (issue #175). No job anywhere targets a retired
//!    runner image (such a leg queues until GitHub cancels it a day later —
//!    what `macos-13` did every night). `nightly.yml` keeps its schedule, is
//!    also triggered by PRs that edit it or the main-window creation its
//!    Windows smoke depends on (a path-filtered `pull_request`, so a broken
//!    nightly is caught before merge, not the next morning), sets no
//!    `continue-on-error` (it is a hard gate), and its unconditional
//!    artifact uploads fail on an empty bundle and cover Linux + Windows.
//! 6. **A bench the tracker can read** (issue #185). The nightly's bencher-
//!    format `cargo bench` runs criterion statelessly — `CRITERION_HOME` on a
//!    per-job `runner.temp` dir (never the cached `target/`) and
//!    `--discard-baseline` — under `pipefail`; `xtask bench-output --expect
//!    <bench-id>` checks the log before `github-action-benchmark` reads it;
//!    and the tracker
//!    keeps its `alert-threshold` with `fail-on-alert: true`.
//! 7. **Automatic release tagging** (issue #298). `release-tag.yml` runs on
//!    pushes to `main` only, never on a fork, holds no secret but
//!    `GITHUB_TOKEN` and deploys to no environment; its write grants sit on a
//!    job that checks nothing out and runs no build tool; it creates the tag
//!    through the API's create-ref call (which refuses an existing ref),
//!    never moves or deletes one, and dispatches `release.yml` on the tag
//!    ref. It is the only workflow that creates tags. `release.yml` has a
//!    `workflow_dispatch` trigger, its real-key step runs on exactly the tag
//!    ref ([`TAG_REF_CONDITION`]), and nothing in it reads
//!    `github.event_name` — a dispatch on the tag must release for real in
//!    every step, not in half of them.

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use crate::yaml::{self, Value};

mod tagging;

pub use tagging::releases_on_tag_dispatch;
use tagging::{real_release_by_ref, release_tagger, tag_creation};

/// Secret names (prefix match) that sign updates for every user. Each
/// reference must sit in the single signing step's `env`.
pub const SIGNING_SECRETS: &[&str] = &["TAURI_SIGNING_PRIVATE_KEY"];

/// The GitHub environment meant to hold the updater signing secrets (once
/// the runbook's one-time setup moves them there). Its deployment rule
/// admits only `v*` tags and it requires a reviewer, so a job on any other
/// ref cannot deploy to it, and one nobody approved never starts.
pub const SIGNING_ENVIRONMENT: &str = "updater-signing";

/// Commands that compile or run project code. None may appear in the
/// signing step; `cargo` may not appear anywhere in the signing job.
const BUILD_WORDS: &[&str] = &[
    "build", "bundle", "cargo", "rustc", "npm", "npx", "pnpm", "yarn", "bun", "just", "make",
    "vite", "node",
];

/// Env vars that make a shell or node load extra code. None may be set in
/// the signing job's `env`, at job level or at/before the key step.
const INJECTING_ENV: &[&str] = &["BASH_ENV", "ENV", "NODE_OPTIONS", "LD_PRELOAD"];

/// Runner command files a step writes to change later steps' env / PATH, or
/// to export values out of the step. None may be written at or before the
/// key step.
const RUNNER_FILES: &[&str] = &[
    "GITHUB_ENV",
    "GITHUB_PATH",
    "GITHUB_OUTPUT",
    "GITHUB_STATE",
    "BASH_ENV",
];

/// The only actions a signing job may run besides its own `run:` steps.
const SIGNING_JOB_ACTIONS: &[&str] = &[
    "actions/checkout@",
    "actions/setup-node@",
    "actions/download-artifact@",
    "actions/upload-artifact@",
];

/// The reusable gate the release must call.
const CI_WORKFLOW: &str = "ci.yml";
/// The tag-triggered release workflow.
const RELEASE_WORKFLOW: &str = "release.yml";
/// The heavy tier: 3-OS bundles, the e2e smoke, the cold-start bench.
const NIGHTLY_WORKFLOW: &str = "nightly.yml";
/// Tags a merged version bump and dispatches the release on the tag.
pub const RELEASE_TAG_WORKFLOW: &str = "release-tag.yml";

/// The condition (whitespace outside string literals removed, without
/// `${{ }}` or outer parentheses) under which `release.yml` signs with the
/// real key: the run is on a `v*` tag ref — a tag push and a dispatch on the
/// tag alike.
pub const TAG_REF_CONDITION: &str = "startsWith(github.ref,'refs/tags/v')";

/// Runner images GitHub has retired. A job on one is never picked up: it sits
/// queued until GitHub cancels it after 24h, failing the whole run.
pub const RETIRED_RUNNERS: &[&str] = &[
    "macos-11",
    "macos-12",
    "macos-13",
    "macos-14",
    "ubuntu-18.04",
    "ubuntu-20.04",
    "windows-2016",
    "windows-2019",
];

/// Files whose edits must trigger the nightly on their PR (its
/// `pull_request.paths` must cover each): the workflow itself, and the
/// main-window creation that forwards the `WebView2` automation flags in a
/// Windows debug build — without them every Windows smoke session fails.
pub const NIGHTLY_PR_PATHS: &[&str] = &[
    ".github/workflows/nightly.yml",
    "apps/desktop/src-tauri/src/lib.rs",
    "apps/desktop/src-tauri/src/browser_args.rs",
];

/// One workflow file: its file name (e.g. `release.yml`) and raw text.
#[derive(Debug, Clone, Copy)]
pub struct WorkflowFile<'a> {
    /// File name within `.github/workflows`.
    pub name: &'a str,
    /// The YAML source.
    pub source: &'a str,
}

/// A parsed workflow, kept next to its name for messages.
struct Parsed<'a> {
    name: &'a str,
    doc: Value,
}

/// Read every `*.yml` / `*.yaml` in `dir` (sorted by name) and [`lint`] them.
///
/// # Errors
/// Any I/O error reading the directory or a file.
pub fn lint_dir(dir: &Path) -> io::Result<Vec<String>> {
    let mut paths: Vec<_> = std::fs::read_dir(dir)?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<io::Result<_>>()?;
    paths.retain(|p| {
        p.extension()
            .is_some_and(|ext| ext == "yml" || ext == "yaml")
    });
    paths.sort();
    let mut loaded = Vec::with_capacity(paths.len());
    for path in &paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        loaded.push((name, std::fs::read_to_string(path)?));
    }
    let files: Vec<WorkflowFile<'_>> = loaded
        .iter()
        .map(|(name, source)| WorkflowFile { name, source })
        .collect();
    Ok(lint(&files))
}

/// Run every check over the given workflow files and return one message per
/// violation (empty = clean).
#[must_use]
pub fn lint(files: &[WorkflowFile<'_>]) -> Vec<String> {
    let mut out = Vec::new();
    let mut parsed = Vec::new();
    for file in files {
        match yaml::from_str(file.source) {
            Ok(doc) => parsed.push(Parsed {
                name: file.name,
                doc,
            }),
            Err(err) => out.push(format!("{}: not valid YAML: {err}", file.name)),
        }
        out.extend(unpinned_lines(file));
    }
    for wf in &parsed {
        out.extend(unpinned_uses(wf));
    }
    out.extend(signing_isolation(&parsed));
    if let Some(release) = parsed.iter().find(|wf| wf.name == RELEASE_WORKFLOW) {
        let ci = parsed.iter().find(|wf| wf.name == CI_WORKFLOW);
        out.extend(release_gate(release, ci));
        out.extend(release_assets(release));
        out.extend(real_release_by_ref(release));
    } else {
        out.push(format!("{RELEASE_WORKFLOW}: missing"));
    }
    out.extend(tag_creation(&parsed));
    if let Some(tagger) = parsed.iter().find(|wf| wf.name == RELEASE_TAG_WORKFLOW) {
        out.extend(release_tagger(tagger));
    } else {
        out.push(format!("{RELEASE_TAG_WORKFLOW}: missing"));
    }
    for wf in &parsed {
        out.extend(retired_runners(wf));
    }
    if let Some(nightly) = parsed.iter().find(|wf| wf.name == NIGHTLY_WORKFLOW) {
        out.extend(nightly_tier(nightly));
        out.extend(bench_tracking(nightly));
    } else {
        out.push(format!("{NIGHTLY_WORKFLOW}: missing"));
    }
    out
}

// ── 1. pinned actions ──────────────────────────────────────────────────────

/// Is `uses` an immutable reference?
fn is_pinned(uses: &str) -> bool {
    if uses.starts_with("./") {
        return true;
    }
    if let Some(image) = uses.strip_prefix("docker://") {
        return image.contains("@sha256:");
    }
    let Some((action, rev)) = uses.split_once('@') else {
        return false;
    };
    action.split('/').count() >= 2
        && rev.len() == 40
        && rev
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn unpinned_uses(wf: &Parsed<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for (job_id, job) in jobs(&wf.doc) {
        if let Some(uses) = job.get("uses").and_then(Value::as_str)
            && !is_pinned(uses)
        {
            out.push(format!(
                "{}: job `{job_id}` calls `{uses}` — pin it to a full commit SHA",
                wf.name
            ));
        }
        for (index, step) in steps(job).iter().enumerate() {
            if let Some(uses) = step.get("uses").and_then(Value::as_str)
                && !is_pinned(uses)
            {
                out.push(format!(
                    "{}: job `{job_id}` step {} uses `{uses}` — pin it to a full commit SHA",
                    wf.name,
                    index + 1
                ));
            }
        }
    }
    out
}

/// Line-level half of the pin rule: a SHA pin must say which version it is
/// (`@<sha> # v1.2.3`), or neither humans nor dependabot can tell.
fn unpinned_lines(file: &WorkflowFile<'_>) -> Vec<String> {
    file.source
        .lines()
        .enumerate()
        .filter_map(|(index, line)| {
            let trimmed = line.trim_start().trim_start_matches("- ").trim_start();
            let value = trimmed.strip_prefix("uses:")?.trim();
            let target = value.split('#').next().unwrap_or_default().trim();
            let local = target.starts_with("./") || target.starts_with("docker://");
            (!local && !value.contains(" # ")).then(|| {
                format!(
                    "{}:{}: `{target}` has no trailing `# <version>` comment",
                    file.name,
                    index + 1
                )
            })
        })
        .collect()
}

// ── 2. release gate ────────────────────────────────────────────────────────

fn release_gate(release: &Parsed<'_>, ci: Option<&Parsed<'_>>) -> Vec<String> {
    let mut out = Vec::new();
    match ci {
        Some(ci) if triggers(&ci.doc).contains("workflow_call") => {}
        Some(_) => out.push(format!(
            "{CI_WORKFLOW}: no `workflow_call` trigger — {RELEASE_WORKFLOW} cannot gate on it"
        )),
        None => out.push(format!("{CI_WORKFLOW}: missing")),
    }

    let gate_path = format!("./.github/workflows/{CI_WORKFLOW}");
    let all_jobs = jobs(&release.doc);
    let gates: BTreeSet<&str> = all_jobs
        .iter()
        .filter(|(_, job)| job.get("uses").and_then(Value::as_str) == Some(gate_path.as_str()))
        .map(|(id, _)| *id)
        .collect();
    if gates.is_empty() {
        out.push(format!(
            "{}: no job calls `{gate_path}` — releases must be gated on CI",
            release.name
        ));
        return out;
    }

    for (job_id, job) in &all_jobs {
        if gates.contains(job_id) {
            continue;
        }
        let mut seen = BTreeSet::new();
        if !reaches_gate(&all_jobs, job_id, &gates, &mut seen) {
            out.push(format!(
                "{}: job `{job_id}` does not (transitively) `needs` the CI gate ({})",
                release.name,
                gates.iter().copied().collect::<Vec<_>>().join(", ")
            ));
        }
        if let Some(function) = job_if(job).as_deref().and_then(status_override) {
            out.push(format!(
                "{}: job `{job_id}` has `if: … {function} …`, which runs it even when the CI gate (or a job it needs) failed — drop the status function",
                release.name
            ));
        }
    }
    out
}

/// A job's `if:` condition as text (a bare `true`/`false` stays as written).
fn job_if(job: &Value) -> Option<String> {
    match job.get("if")? {
        Value::String(text) => Some(text.clone()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// The status-check function in `condition` that lets a job run although a
/// job it `needs` failed or was cancelled: `always()`, `failure()` or
/// `cancelled()` (including `!cancelled()`). Without one, GitHub implies
/// `success()` — every needed job, the gate included, must have passed.
fn status_override(condition: &str) -> Option<&'static str> {
    let compact: String = condition
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_ascii_lowercase();
    ["always()", "failure()", "cancelled()"]
        .into_iter()
        .find(|function| compact.contains(function))
}

fn reaches_gate<'a>(
    all_jobs: &[(&'a str, &'a Value)],
    job_id: &'a str,
    gates: &BTreeSet<&str>,
    seen: &mut BTreeSet<&'a str>,
) -> bool {
    if !seen.insert(job_id) {
        return false;
    }
    let Some((_, job)) = all_jobs.iter().find(|(id, _)| *id == job_id) else {
        return false;
    };
    needs(job).into_iter().any(|dep| {
        gates.contains(dep)
            || all_jobs
                .iter()
                .find(|(id, _)| *id == dep)
                .is_some_and(|(id, _)| reaches_gate(all_jobs, id, gates, seen))
    })
}

// ── 3. signing-key isolation ───────────────────────────────────────────────

/// Where a signing-secret reference was found.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone)]
struct StepRef<'a> {
    workflow: &'a str,
    job: String,
    step: usize,
}

fn signing_isolation(parsed: &[Parsed<'_>]) -> Vec<String> {
    let mut out = Vec::new();
    let mut signing_steps: BTreeSet<StepRef<'_>> = BTreeSet::new();

    for wf in parsed {
        let mut hits = Vec::new();
        collect_strings(&wf.doc, &mut Vec::new(), &mut hits);
        for (path, text) in hits {
            let lowered = text.to_ascii_lowercase().replace(' ', "");
            if lowered.contains("tojson(secrets)") {
                out.push(format!(
                    "{}: `{}` serializes every secret — reference secrets by name",
                    wf.name,
                    path.join(".")
                ));
            }
            if path.last().map(String::as_str) == Some("secrets") && text.trim() == "inherit" {
                out.push(format!(
                    "{}: `{}: inherit` forwards every secret (the signing key included) — pass secrets by name",
                    wf.name,
                    path.join(".")
                ));
            }
            if !SIGNING_SECRETS
                .iter()
                .any(|name| references_secret(text, name))
            {
                continue;
            }
            match step_env_location(&path) {
                Some((job, step)) => {
                    signing_steps.insert(StepRef {
                        workflow: wf.name,
                        job,
                        step,
                    });
                }
                None => out.push(format!(
                    "{}: `{}` references the updater signing key outside a single step's `env` — scope it to the signing step only",
                    wf.name,
                    path.join(".")
                )),
            }
        }
    }

    match signing_steps.len() {
        0 => out.push(
            "no workflow step receives the updater signing key — the release must sign Curio.app.tar.gz"
                .to_owned(),
        ),
        1 => {
            if let Some(step_ref) = signing_steps.first()
                && let Some(wf) = parsed.iter().find(|wf| wf.name == step_ref.workflow)
            {
                out.extend(signing_step_is_minimal(wf, step_ref));
                out.extend(signing_environment(wf, step_ref));
            }
        }
        _ => out.push(format!(
            "the updater signing key reaches {} steps ({}) — exactly one may hold it",
            signing_steps.len(),
            signing_steps
                .iter()
                .map(|s| format!("{} job `{}` step {}", s.workflow, s.job, s.step + 1))
                .collect::<Vec<_>>()
                .join("; ")
        )),
    }
    out
}

/// Does `text` read the secret `name` (or one it prefixes, e.g. `…_PASSWORD`)
/// through the `secrets` context, in dot or index syntax?
fn references_secret(text: &str, name: &str) -> bool {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    ["secrets.", "secrets['", "secrets[\""]
        .iter()
        .any(|accessor| compact.contains(&format!("{accessor}{name}")))
}

/// `["jobs", <job>, "steps", <i>, "env", <KEY>]` → `(job, i)`.
fn step_env_location(path: &[String]) -> Option<(String, usize)> {
    match path {
        [jobs, job, steps, index, env, _key]
            if jobs == "jobs" && steps == "steps" && env == "env" =>
        {
            Some((job.clone(), index.parse().ok()?))
        }
        _ => None,
    }
}

fn signing_step_is_minimal(wf: &Parsed<'_>, at: &StepRef<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let Some(job) = wf
        .doc
        .get("jobs")
        .and_then(|jobs| jobs.get(at.job.as_str()))
    else {
        return out;
    };
    let job_steps = steps(job);
    let Some(step) = job_steps.get(at.step) else {
        return out;
    };
    let label = format!("{}: job `{}` step {}", wf.name, at.job, at.step + 1);

    if step.get("uses").is_some() {
        out.push(format!(
            "{label} hands the signing key to an action — sign with a plain `run:` step"
        ));
    }
    let run = step.get("run").and_then(Value::as_str).unwrap_or_default();
    if !run.contains("signer sign") {
        out.push(format!(
            "{label} holds the signing key but does not run `signer sign`"
        ));
    }
    let words = shell_words(run);
    for word in BUILD_WORDS {
        if words.contains(word) {
            out.push(format!(
                "{label} holds the signing key and runs `{word}` — build steps must never see it"
            ));
        }
    }

    for (index, other) in job_steps.iter().enumerate() {
        let other_label = format!("{}: job `{}` step {}", wf.name, at.job, index + 1);
        if let Some(uses) = other.get("uses").and_then(Value::as_str)
            && !SIGNING_JOB_ACTIONS.iter().any(|ok| uses.starts_with(ok))
        {
            out.push(format!(
                "{other_label} uses `{uses}` in the signing job — only checkout/setup-node/artifact actions may run beside the key"
            ));
        }
        let script = other.get("run").and_then(Value::as_str).unwrap_or_default();
        let script_words = shell_words(script);
        if script_words.contains(&"cargo") {
            out.push(format!(
                "{other_label} runs cargo in the signing job — build in another job"
            ));
        }
        let commands = shell_commands(script);
        for command in &commands {
            if is_npm_install(command) && !command.iter().any(|w| w == "--ignore-scripts") {
                out.push(format!(
                    "{other_label} installs npm packages without `--ignore-scripts` in the signing job"
                ));
            }
        }
        if index > at.step {
            // Runs after the key step: the key lived only in that step's
            // process env, so nothing here can reach it.
            continue;
        }
        for (key, _) in env_entries(other) {
            if INJECTING_ENV.contains(&key) {
                out.push(format!(
                    "{other_label} sets `{key}` in the signing job, which injects code into the key step's shell or node"
                ));
            }
        }
        for file in RUNNER_FILES {
            if script.contains(file) {
                out.push(format!(
                    "{other_label} writes `{file}` at or before the signing step — it could inject env into (or export from) the key step"
                ));
            }
        }
        if index == at.step {
            continue;
        }
        if backgrounds(script) {
            out.push(format!(
                "{other_label} leaves a background process running beside the signing step"
            ));
        }
        for command in commands.iter().filter(|c| !is_npm_install(c)) {
            for word in BUILD_WORDS {
                if *word != "cargo" && command.iter().any(|w| w == word) {
                    out.push(format!(
                        "{other_label} runs `{word}` before the signing step — only `npm ci --ignore-scripts` may run ahead of the key"
                    ));
                }
            }
        }
    }
    for (key, _) in env_entries(job) {
        if INJECTING_ENV.contains(&key) {
            out.push(format!(
                "{}: job `{}` sets `{key}` for every step of the signing job, the key step included",
                wf.name, at.job
            ));
        }
    }
    out
}

/// The signing job must deploy to [`SIGNING_ENVIRONMENT`] whenever its key
/// step runs: the secrets live only there, behind the tag-only deployment
/// rule and the required reviewer. A workflow with triggers besides `push`
/// (the `workflow_dispatch` dry run) must select the environment
/// conditionally — `${{ <cond> && 'updater-signing' || '' }}`, an empty name
/// meaning "no environment" — where `<cond>` is exactly the key step's
/// `if:`, so the dry run is not held at a rule it can never pass and the
/// real key step never runs outside the environment.
///
/// `&&` binds tighter than `||` in GitHub expressions, so a `<cond>` with a
/// top-level `||` must be parenthesised: `${{ A || B && 'updater-signing' ||
/// '' }}` reads as `A || (B && …)` and evaluates to `true` (not the
/// environment's name) whenever `A` holds. Such a `<cond>` is flagged;
/// `(A || B)` is accepted.
fn signing_environment(wf: &Parsed<'_>, at: &StepRef<'_>) -> Vec<String> {
    let Some(job) = wf
        .doc
        .get("jobs")
        .and_then(|jobs| jobs.get(at.job.as_str()))
    else {
        return Vec::new();
    };
    let label = format!("{}: job `{}`", wf.name, at.job);
    let declared = match job.get("environment") {
        Some(Value::String(name)) => Some(name.as_str()),
        Some(Value::Mapping(map)) => map.get("name").and_then(Value::as_str),
        _ => None,
    };
    let Some(declared) = declared else {
        return vec![format!(
            "{label} holds the signing key but declares no `environment:` — the key lives only in the `{SIGNING_ENVIRONMENT}` environment (tag-restricted, reviewer-gated)"
        )];
    };
    let compact = compact_expression(declared);
    if compact == SIGNING_ENVIRONMENT {
        let dry_run_triggers = triggers(&wf.doc).iter().any(|t| t != "push");
        return if dry_run_triggers {
            vec![format!(
                "{label} always deploys to `{SIGNING_ENVIRONMENT}`, so the `workflow_dispatch` dry run waits on (and fails) its tag-only rule — select it under the key step's `if:` (`${{{{ <cond> && '{SIGNING_ENVIRONMENT}' || '' }}}}`)"
            )]
        } else {
            Vec::new()
        };
    }
    let suffix = format!("&&'{SIGNING_ENVIRONMENT}'||''}}}}");
    let Some(condition) = compact
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix(suffix.as_str()))
    else {
        return vec![format!(
            "{label} deploys to `{declared}`, not `{SIGNING_ENVIRONMENT}` — the environment that holds the signing key"
        )];
    };
    let unwrapped = without_outer_parens(condition);
    if unwrapped == condition && has_top_level_or(condition) {
        return vec![format!(
            "{label} selects `{SIGNING_ENVIRONMENT}` under `{condition}`, whose top-level `||` binds looser than `&&` — the job would deploy to `true` (or nowhere) while the key step runs; parenthesise it: `${{{{ ({condition}) && '{SIGNING_ENVIRONMENT}' || '' }}}}`"
        )];
    }
    let key_step_if = steps(job)
        .get(at.step)
        .and_then(|step| step.get("if"))
        .and_then(Value::as_str)
        .map(|text| {
            let compact = compact_expression(text);
            let bare = compact
                .strip_prefix("${{")
                .and_then(|rest| rest.strip_suffix("}}"))
                .unwrap_or(&compact);
            without_outer_parens(bare).to_owned()
        });
    if key_step_if.as_deref() == Some(unwrapped) {
        Vec::new()
    } else {
        vec![format!(
            "{label} selects `{SIGNING_ENVIRONMENT}` only when `{unwrapped}`, but the key step runs on `{}` — the key step must never run outside the environment",
            key_step_if.as_deref().unwrap_or("every event")
        )]
    }
}

/// A GitHub expression with the whitespace outside its `'…'` string
/// literals removed (inside one it is significant: `'a b'` is not `'ab'`).
/// A `''` escape inside a literal flips the quote state twice, so it needs
/// no special case.
fn compact_expression(text: &str) -> String {
    let mut quoted = false;
    text.chars()
        .filter(|c| {
            if *c == '\'' {
                quoted = !quoted;
            }
            quoted || !c.is_whitespace()
        })
        .collect()
}

/// Walk `expr` (a compacted GitHub expression) outside its string literals,
/// calling `visit` with each byte offset, character and the parenthesis
/// depth before that character. Stops as soon as `visit` returns `true`, and
/// returns whether it did.
fn scan_expression(expr: &str, mut visit: impl FnMut(usize, char, usize) -> bool) -> bool {
    let mut quoted = false;
    let mut depth = 0_usize;
    for (offset, c) in expr.char_indices() {
        if c == '\'' {
            quoted = !quoted;
            continue;
        }
        if quoted {
            continue;
        }
        if visit(offset, c, depth) {
            return true;
        }
        match c {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    false
}

/// Does `expr` contain an `||` outside every parenthesis and string literal?
fn has_top_level_or(expr: &str) -> bool {
    let mut after_pipe = false;
    scan_expression(expr, |_, c, depth| {
        let pipe = depth == 0 && c == '|';
        let hit = pipe && after_pipe;
        after_pipe = pipe && !after_pipe;
        hit
    })
}

/// `expr` without the parentheses that wrap all of it (`((a||b))` →
/// `a||b`). `(a)&&(b)` comes back as is: its first `(` closes before the end.
fn without_outer_parens(expr: &str) -> &str {
    let mut current = expr;
    while let Some(inner) = current
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        let last = current.len() - 1;
        let closes_early = scan_expression(current, |offset, c, depth| {
            c == ')' && depth == 1 && offset < last
        });
        if closes_early {
            break;
        }
        current = inner;
    }
    current
}

/// The entries of a job's or step's `env:` mapping.
fn env_entries(node: &Value) -> Vec<(&str, &Value)> {
    node.get("env")
        .and_then(Value::as_mapping)
        .map(|env| {
            env.iter()
                .filter_map(|(key, value)| Some((key.as_str()?, value)))
                .collect()
        })
        .unwrap_or_default()
}

/// Is this simple command an `npm ci` / `npm install` / `npm i`?
fn is_npm_install(command: &[String]) -> bool {
    command
        .windows(2)
        .any(|pair| pair[0] == "npm" && matches!(pair[1].as_str(), "ci" | "install" | "i"))
}

/// Does `script` start a process that outlives its step (`cmd &`, `nohup`,
/// `setsid`, `disown`)? `&&`, `2>&1` and `&>` are not backgrounding.
fn backgrounds(script: &str) -> bool {
    if shell_words(script)
        .iter()
        .any(|w| matches!(*w, "nohup" | "setsid" | "disown"))
    {
        return true;
    }
    let chars: Vec<char> = script.chars().collect();
    chars.iter().enumerate().any(|(i, c)| {
        let before = i.checked_sub(1).and_then(|j| chars.get(j)).copied();
        let after = chars.get(i + 1).copied();
        *c == '&' && !matches!(before, Some('&' | '>' | '<')) && !matches!(after, Some('&' | '>'))
    })
}

/// Split shell text into simple commands (on newlines, `;`, `&`, `|`,
/// parentheses and backticks; `\`-continued lines are joined first), each as
/// its words.
fn shell_commands(script: &str) -> Vec<Vec<String>> {
    script
        .replace("\\\n", " ")
        .split(['\n', ';', '&', '|', '(', ')', '`'])
        .map(|command| {
            command
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|words| !words.is_empty())
        .collect()
}

/// Split shell text into words on whitespace and shell punctuation.
fn shell_words(script: &str) -> Vec<&str> {
    script
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '&' | '|' | '(' | ')' | '`'))
        .filter(|word| !word.is_empty())
        .collect()
}

// ── 4. release assets ──────────────────────────────────────────────────────

fn release_assets(release: &Parsed<'_>) -> Vec<String> {
    let mut out = Vec::new();
    let mut found_draft = false;
    for (job_id, job) in jobs(&release.doc) {
        let job_steps = steps(job);
        for (index, step) in job_steps.iter().enumerate() {
            let uses = step.get("uses").and_then(Value::as_str).unwrap_or_default();
            if !uses.starts_with("softprops/action-gh-release@") {
                continue;
            }
            found_draft = true;
            let label = format!("{}: job `{job_id}` step {}", release.name, index + 1);
            let with = step.get("with");
            if with.and_then(|w| w.get("draft")).and_then(Value::as_bool) != Some(true) {
                out.push(format!(
                    "{label} must create a DRAFT release (publishing is human-gated)"
                ));
            }
            let files = with
                .and_then(|w| w.get("files"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            for (needle, what) in [
                ("SHA256SUMS", "SHA256SUMS"),
                (".cdx.json", "a CycloneDX SBOM (*.cdx.json)"),
                (
                    ".sigstore.json",
                    "the provenance attestation bundle (*.sigstore.json)",
                ),
            ] {
                if !files.contains(needle) {
                    out.push(format!("{label} does not attach {what}"));
                }
            }
            let attested = job_steps[..index].iter().any(|earlier| {
                earlier
                    .get("uses")
                    .and_then(Value::as_str)
                    .is_some_and(|u| u.starts_with("actions/attest-build-provenance@"))
            });
            if !attested {
                out.push(format!(
                    "{label} is not preceded by an actions/attest-build-provenance step"
                ));
            }
        }
    }
    if !found_draft {
        out.push(format!(
            "{}: no softprops/action-gh-release step drafts the release",
            release.name
        ));
    }
    out
}

// ── 5. a nightly that runs ─────────────────────────────────────────────────

/// The runner labels a job can land on: a literal `runs-on` (a string, a
/// list, or the `labels` of its `{ group, labels }` form), and every `os` /
/// `runs-on` value in its matrix (where `runs-on: ${{ matrix.os }}` takes
/// them from) — except under `matrix.exclude`, which names the legs that do
/// *not* run.
fn runner_labels(job: &Value) -> Vec<&str> {
    let mut found = Vec::new();
    for key in ["runs-on", "strategy"] {
        if let Some(node) = job.get(key) {
            collect_strings(node, &mut vec![key.to_owned()], &mut found);
        }
    }
    found
        .into_iter()
        .filter(|(path, _)| {
            if path.iter().any(|segment| segment == "exclude") {
                return false;
            }
            // `os: [a, b]` / `runs-on: [self-hosted, x]` end in an index.
            path.iter()
                .rev()
                .find(|segment| !segment.bytes().all(|b| b.is_ascii_digit()))
                .is_some_and(|key| {
                    key == "os"
                        || key == "runs-on"
                        || (key == "labels" && path.first().is_some_and(|k| k == "runs-on"))
                })
        })
        .map(|(_, label)| label)
        .collect()
}

fn retired_runners(wf: &Parsed<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for (job_id, job) in jobs(&wf.doc) {
        for label in runner_labels(job) {
            if RETIRED_RUNNERS.contains(&label) {
                out.push(format!(
                    "{}: job `{job_id}` runs on `{label}`, a retired GitHub runner image — the leg queues until GitHub cancels it and the run fails",
                    wf.name
                ));
            }
        }
    }
    out
}

/// Is a `continue-on-error` value anything but a literal `false`?
fn tolerates_failure(node: Option<&Value>) -> bool {
    !matches!(node, None | Some(Value::Bool(false)))
}

fn nightly_tier(nightly: &Parsed<'_>) -> Vec<String> {
    let name = nightly.name;
    let mut out = Vec::new();
    let on = triggers(&nightly.doc);
    if !on.contains("schedule") {
        out.push(format!(
            "{name}: no `schedule` trigger — the heavy tier never runs on its own"
        ));
    }

    if on.contains("pull_request") {
        let paths: Vec<&str> = nightly
            .doc
            .get("on")
            .and_then(|on| on.get("pull_request"))
            .and_then(|pr| pr.get("paths"))
            .and_then(Value::as_sequence)
            .map(|paths| paths.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if paths.is_empty() {
            out.push(format!(
                "{name}: `pull_request` has no `paths` filter — the 3-OS matrix would run on every PR"
            ));
        } else {
            for required in NIGHTLY_PR_PATHS {
                if !paths_filter_covers(&paths, required) {
                    out.push(format!(
                        "{name}: `pull_request.paths` does not cover `{required}` — an edit to it would merge without the nightly ever running"
                    ));
                }
            }
        }
    } else {
        out.push(format!(
            "{name}: no path-filtered `pull_request` trigger — an edit to the nightly is first run the morning after it merges"
        ));
    }

    let mut upload_labels = Vec::new();
    for (job_id, job) in jobs(&nightly.doc) {
        if tolerates_failure(job.get("continue-on-error")) {
            out.push(format!(
                "{name}: job `{job_id}` sets `continue-on-error` — the nightly is a hard gate"
            ));
        }
        for (index, step) in steps(job).iter().enumerate() {
            if tolerates_failure(step.get("continue-on-error")) {
                out.push(format!(
                    "{name}: job `{job_id}` step {} sets `continue-on-error` — the nightly is a hard gate",
                    index + 1
                ));
            }
            let uploads = step
                .get("uses")
                .and_then(Value::as_str)
                .is_some_and(|uses| uses.starts_with("actions/upload-artifact@"));
            // Conditional uploads (failure screenshots) may legitimately be
            // empty; the unconditional ones are the bundles themselves.
            if !uploads || step.get("if").is_some() {
                continue;
            }
            upload_labels.extend(runner_labels(job));
            let on_empty = step
                .get("with")
                .and_then(|with| with.get("if-no-files-found"))
                .and_then(Value::as_str);
            if on_empty != Some("error") {
                out.push(format!(
                    "{name}: job `{job_id}` step {} uploads without `if-no-files-found: error` — a bundle job that uploads nothing must fail",
                    index + 1
                ));
            }
        }
    }
    for (os, prefix) in [("Linux", "ubuntu-"), ("Windows", "windows-")] {
        if !upload_labels.iter().any(|label| label.starts_with(prefix)) {
            out.push(format!(
                "{name}: no unconditional artifact upload runs on a {os} (`{prefix}*`) runner — the nightly must ship a {os} bundle"
            ));
        }
    }
    out
}

// ── 6. a bench the tracker can read ────────────────────────────────────────

/// The action that keeps the trailing baseline and raises the 130% alert.
const BENCH_TRACKER: &str = "benchmark-action/github-action-benchmark@";

/// Is this step the bencher-format criterion run the tracker consumes?
fn is_bencher_run(step: &Value) -> bool {
    step.get("run")
        .and_then(Value::as_str)
        .is_some_and(|run| run.contains("cargo bench") && run.contains("--output-format bencher"))
}

/// A step's or its job's `env` value for `key`, step first.
fn env_value<'v>(job: &'v Value, step: &'v Value, key: &str) -> Option<&'v str> {
    [step, job]
        .into_iter()
        .find_map(|node| node.get("env").and_then(|env| env.get(key)))
        .and_then(Value::as_str)
}

/// What one `set` command (or custom shell command line) does to `pipefail`:
/// `Some(true)` for `-o pipefail` / `-eo pipefail` / `-euo pipefail` (any
/// short-flag group ending in `o`, followed by `pipefail`), `Some(false)` for
/// `+o pipefail`, `None` when it leaves it alone. Stops at a comment or a
/// command separator.
fn pipefail_switch(words: &[&str]) -> Option<bool> {
    let words = words
        .iter()
        .position(|word| word.starts_with('#') || [";", "&&", "||", "|"].contains(word))
        .map_or(words, |end| &words[..end]);
    // The last `-o pipefail` / `+o pipefail` on the line wins.
    words.windows(2).rev().find_map(|pair| {
        if pair[1] != "pipefail" {
            return None;
        }
        let on = match pair[0].as_bytes().first() {
            Some(b'-') => true,
            Some(b'+') => false,
            _ => return None,
        };
        let letters = &pair[0][1..];
        (letters.ends_with('o') && letters.bytes().all(|b| b.is_ascii_alphabetic())).then_some(on)
    })
}

/// A shell line's words, with a `;` glued to a word's end split off as its
/// own separator (`set -euo pipefail; cargo bench …` → `pipefail`, `;`, …).
fn switch_words(line: &str) -> Vec<&str> {
    line.split_whitespace()
        .flat_map(|word| match word.strip_suffix(';') {
            Some(head) if !head.is_empty() => vec![head, ";"],
            _ => vec![word],
        })
        .collect()
}

/// What a script's own `set` lines leave `pipefail` at where `cargo bench`
/// starts: the last switch (`Some(true)` on, `Some(false)` off) on a line
/// that starts with `set`, up to and including the bench's own line (`set
/// -o pipefail; cargo bench …`); `None` when none touches it. A `set` after
/// the bench is too late to cover its pipe, and a comment or a string that
/// merely mentions pipefail does not count.
fn script_pipefail(run: &str) -> Option<bool> {
    let mut on = None;
    for line in run.lines() {
        let words = switch_words(line);
        if words.first() == Some(&"set")
            && let Some(switch) = pipefail_switch(&words[1..])
        {
            on = Some(switch);
        }
        if line.contains("cargo bench") {
            break;
        }
    }
    on
}

/// Does a `shell:` run scripts under `pipefail`? GitHub runs `shell: bash` as
/// `bash --noprofile --norc -eo pipefail {0}`; a custom command line counts
/// when it passes the option itself.
fn shell_has_pipefail(shell: &str) -> bool {
    let words = switch_words(shell);
    shell == "bash" || pipefail_switch(&words) == Some(true)
}

/// A job's or workflow's `defaults.run.shell`.
fn default_shell(node: &Value) -> Option<&Value> {
    node.get("defaults")
        .and_then(|defaults| defaults.get("run"))
        .and_then(|run| run.get("shell"))
}

/// Does a step's script fail when any stage of a pipe fails? The default
/// shell (no `shell:` anywhere) is plain `bash -e`, so a pipe needs either a
/// `set -o pipefail` in the script or a `shell:` with it — the step's own,
/// else its job's `defaults.run.shell`, else the workflow's.
fn fails_on_pipe(doc: &Value, job: &Value, step: &Value) -> bool {
    let run = step.get("run").and_then(Value::as_str).unwrap_or_default();
    if !run.contains('|') {
        return true;
    }
    let shell = step
        .get("shell")
        .or_else(|| default_shell(job))
        .or_else(|| default_shell(doc))
        .and_then(Value::as_str);
    script_pipefail(run).unwrap_or_else(|| shell.is_some_and(shell_has_pipefail))
}

/// Is a `CRITERION_HOME` value a path inside the job's `${{ runner.temp }}`
/// — the expression itself, or a relative path under it with no `..` to
/// climb back out (say into the cached `target/criterion`)?
fn is_under_runner_temp(home: &str) -> bool {
    home.trim()
        .strip_prefix("${{ runner.temp }}")
        .is_some_and(|rest| {
            (rest.is_empty() || rest.starts_with('/'))
                && !rest.split(['/', '\\']).any(|part| part == "..")
        })
}

/// Is a `with:` flag literally on (`true` or `"true"`)?
fn is_on(node: Option<&Value>) -> bool {
    match node {
        Some(Value::Bool(on)) => *on,
        Some(Value::String(text)) => text == "true",
        _ => false,
    }
}

fn bench_tracking(nightly: &Parsed<'_>) -> Vec<String> {
    let name = nightly.name;
    let mut out = Vec::new();
    let mut found = false;
    for (job_id, job) in jobs(&nightly.doc) {
        let steps = steps(job);
        let Some(bench_at) = steps.iter().position(is_bencher_run) else {
            continue;
        };
        found = true;
        let step = &steps[bench_at];
        let at = format!("{name}: job `{job_id}` step {}", bench_at + 1);
        let run = step.get("run").and_then(Value::as_str).unwrap_or_default();
        if !run.contains("--discard-baseline") {
            out.push(format!(
                "{at} runs `cargo bench` without `--discard-baseline` — criterion compares against (and saves) its own baseline, and a stale one breaks the line the tracker parses"
            ));
        }
        if !env_value(job, step, "CRITERION_HOME").is_some_and(is_under_runner_temp) {
            out.push(format!(
                "{at} runs `cargo bench` without `CRITERION_HOME` on a `runner.temp` dir — criterion's default home is the cached `target/criterion`, where a stale `base/` breaks the line the tracker parses"
            ));
        }
        if !fails_on_pipe(&nightly.doc, job, step) {
            out.push(format!(
                "{at} pipes `cargo bench` without `set -o pipefail` (or `shell: bash`) — a crashed bench passes through `tee`"
            ));
        }

        let later = &steps[bench_at + 1..];
        let tracker_at = later.iter().position(|later_step| {
            later_step
                .get("uses")
                .and_then(Value::as_str)
                .is_some_and(|uses| uses.starts_with(BENCH_TRACKER))
        });
        let check = later[..tracker_at.unwrap_or(later.len())]
            .iter()
            .filter_map(|check| check.get("run").and_then(Value::as_str))
            .find(|run| run.contains("xtask -- bench-output"));
        match check {
            None => out.push(format!(
                "{name}: job `{job_id}` has no `xtask -- bench-output` step between the bench and the tracker — an untrackable log fails as a bare \"No benchmark result was found\""
            )),
            Some(run) if !shell_words(run).contains(&"--expect") => out.push(format!(
                "{name}: job `{job_id}` checks the bench log without `--expect <bench-id>` — any one unrelated result would pass while the tracked benchmark is missing"
            )),
            Some(_) => {}
        }
        let Some(tracker_at) = tracker_at else {
            out.push(format!(
                "{name}: job `{job_id}` runs the bench but no later `{BENCH_TRACKER}…` step tracks it — no regression alert"
            ));
            continue;
        };
        let with = later[tracker_at].get("with");
        let flag = |key: &str| with.and_then(|with| with.get(key));
        if !is_on(flag("fail-on-alert")) {
            out.push(format!(
                "{name}: job `{job_id}` tracks the bench without `fail-on-alert: true` — a regression past the threshold would stay green"
            ));
        }
        if flag("alert-threshold").and_then(Value::as_str).is_none() {
            out.push(format!(
                "{name}: job `{job_id}` tracks the bench without an `alert-threshold`"
            ));
        }
    }
    if !found {
        out.push(format!(
            "{name}: no step runs `cargo bench … --output-format bencher` — the cold-start gate never runs"
        ));
    }
    out
}

/// Does a `pull_request.paths` filter trigger on `path`? GitHub's rule: the
/// last pattern that matches decides, and a `!pattern` excludes.
fn paths_filter_covers(patterns: &[&str], path: &str) -> bool {
    patterns
        .iter()
        .fold(false, |covered, pattern| match pattern.strip_prefix('!') {
            Some(negated) if glob_matches(negated.as_bytes(), path.as_bytes()) => false,
            Some(_) => covered,
            None => covered || glob_matches(pattern.as_bytes(), path.as_bytes()),
        })
}

/// GitHub filter-pattern glob: `**` matches anything (`/` included), `*`
/// anything but `/`, `?` one character but `/`; every other byte is literal.
fn glob_matches(pattern: &[u8], path: &[u8]) -> bool {
    match pattern {
        [] => path.is_empty(),
        [b'*', b'*', rest @ ..] => {
            // `**/x` also matches a bare `x` at this level.
            let rest_after_slash = rest.strip_prefix(b"/").unwrap_or(rest);
            (0..=path.len()).any(|skip| glob_matches(rest, &path[skip..]))
                || glob_matches(rest_after_slash, path)
        }
        [b'*', rest @ ..] => {
            let segment = path.iter().position(|&b| b == b'/').unwrap_or(path.len());
            (0..=segment).any(|skip| glob_matches(rest, &path[skip..]))
        }
        [b'?', rest @ ..] => {
            matches!(path.first(), Some(&b) if b != b'/') && glob_matches(rest, &path[1..])
        }
        [literal, rest @ ..] => path.first() == Some(literal) && glob_matches(rest, &path[1..]),
    }
}

// ── YAML helpers ───────────────────────────────────────────────────────────

fn jobs(doc: &Value) -> Vec<(&str, &Value)> {
    doc.get("jobs")
        .and_then(Value::as_mapping)
        .map(|jobs| {
            jobs.iter()
                .filter_map(|(id, job)| Some((id.as_str()?, job)))
                .collect()
        })
        .unwrap_or_default()
}

fn steps(job: &Value) -> &[Value] {
    job.get("steps")
        .and_then(Value::as_sequence)
        .map_or(&[], Vec::as_slice)
}

fn needs(job: &Value) -> Vec<&str> {
    match job.get("needs") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Sequence(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    }
}

fn triggers(doc: &Value) -> BTreeSet<String> {
    match doc.get("on") {
        Some(Value::String(one)) => BTreeSet::from([one.clone()]),
        Some(Value::Sequence(many)) => many
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::Mapping(map)) => map
            .keys()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// Every string scalar in the document with its key path.
fn collect_strings<'v>(
    value: &'v Value,
    path: &mut Vec<String>,
    out: &mut Vec<(Vec<String>, &'v str)>,
) {
    match value {
        Value::String(text) => out.push((path.clone(), text.as_str())),
        Value::Sequence(items) => {
            for (index, item) in items.iter().enumerate() {
                path.push(index.to_string());
                collect_strings(item, path, out);
                path.pop();
            }
        }
        Value::Mapping(map) => {
            for (key, item) in map {
                let key = match key {
                    Value::String(key) => key.clone(),
                    other => format!("{other:?}"),
                };
                path.push(key);
                collect_strings(item, path, out);
                path.pop();
            }
        }
        Value::Tagged(tagged) => collect_strings(&tagged.value, path, out),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA: &str = "0123456789abcdef0123456789abcdef01234567";

    const CI: &str = "on:\n  pull_request:\n  workflow_call:\njobs:\n  test:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567 # v4.4.0\n      - run: cargo test\n";

    /// The sign job's environment line in [`RELEASE`].
    const ENV_LINE: &str = "    environment: ${{ startsWith(github.ref, 'refs/tags/v') && 'updater-signing' || '' }}\n";
    /// The key step's head in [`RELEASE`].
    const KEY_STEP: &str =
        "      - name: Sign\n        if: startsWith(github.ref, 'refs/tags/v')\n";

    /// A minimal release.yml with the shape the lint demands.
    const RELEASE: &str = r#"on:
  push:
    tags: ["v*"]
  workflow_dispatch:
jobs:
  gate:
    uses: ./.github/workflows/ci.yml
  build:
    needs: gate
    runs-on: macos-15
    steps:
      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567 # v4.4.0
      - run: npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
  sign:
    needs: build
    runs-on: ubuntu-latest
    environment: ${{ startsWith(github.ref, 'refs/tags/v') && 'updater-signing' || '' }}
    steps:
      - uses: actions/setup-node@0123456789abcdef0123456789abcdef01234567 # v4.4.0
      - run: npm ci --ignore-scripts
      - name: Sign
        if: startsWith(github.ref, 'refs/tags/v')
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
        run: ./node_modules/.bin/tauri signer sign Curio.app.tar.gz
  release:
    needs: [sign]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/attest-build-provenance@0123456789abcdef0123456789abcdef01234567 # v4.2.2
        with:
          subject-checksums: dist/SHA256SUMS
      - uses: softprops/action-gh-release@0123456789abcdef0123456789abcdef01234567 # v2.6.2
        with:
          draft: true
          files: |
            dist/SHA256SUMS
            dist/Curio.cdx.json
            dist/Curio-provenance.sigstore.json
"#;

    /// A minimal nightly.yml with the shape the lint demands.
    const NIGHTLY: &str = r#"on:
  schedule:
    - cron: "0 7 * * *"
  workflow_dispatch:
  pull_request:
    paths:
      - .github/workflows/nightly.yml
      - apps/desktop/src-tauri/src/lib.rs
      - apps/desktop/src-tauri/src/browser_args.rs
      - apps/desktop/e2e/**
jobs:
  bundle:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - { name: linux-x64, os: ubuntu-22.04 }
          - { name: macos-universal, os: macos-15 }
          - { name: windows-x64, os: windows-latest }
    steps:
      - run: npm run tauri build
      - uses: actions/upload-artifact@0123456789abcdef0123456789abcdef01234567 # v4.6.2
        with:
          name: curio-${{ matrix.name }}
          if-no-files-found: error
  smoke:
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-22.04, windows-latest]
    steps:
      - run: npm test
      - if: failure()
        uses: actions/upload-artifact@0123456789abcdef0123456789abcdef01234567 # v4.6.2
        with:
          if-no-files-found: ignore
  bench:
    if: github.event_name != 'pull_request'
    runs-on: ubuntu-22.04
    steps:
      - env:
          CRITERION_HOME: ${{ runner.temp }}/criterion
        run: |
          set -o pipefail
          cargo bench -p curio-fixtures --bench cold_start -- \
            --output-format bencher --noplot --discard-baseline \
            | tee bench-output.txt
      - run: cargo run -p xtask -- bench-output bench-output.txt --expect cold_start/open_and_first_page_50k
      - uses: benchmark-action/github-action-benchmark@0123456789abcdef0123456789abcdef01234567 # v1.22.2
        with:
          tool: cargo
          output-file-path: bench-output.txt
          alert-threshold: "130%"
          fail-on-alert: true
"#;

    /// A minimal release-tag.yml with the shape the lint demands.
    const RELEASE_TAG: &str = r#"on:
  push:
    branches: [main]
permissions:
  contents: read
jobs:
  plan:
    if: ${{ !github.event.repository.fork }}
    runs-on: ubuntu-latest
    outputs:
      tag: ${{ steps.plan.outputs.tag }}
    steps:
      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567 # v4.4.0
      - id: plan
        run: cargo run -p xtask -- release-tag >> "$GITHUB_OUTPUT"
  tag:
    needs: plan
    if: needs.plan.outputs.tag != ''
    runs-on: ubuntu-latest
    permissions:
      contents: write
      actions: write
    env:
      GH_TOKEN: ${{ github.token }}
      TAG: ${{ needs.plan.outputs.tag }}
    steps:
      - name: Tag
        run: |
          object="$(gh api "repos/$GITHUB_REPOSITORY/git/tags" \
            -f tag="$TAG" -f object=abc -f type=commit --jq .sha)"
          gh api "repos/$GITHUB_REPOSITORY/git/refs" -f ref="refs/tags/$TAG" -f sha="$object"
      - name: Dispatch
        run: gh workflow run release.yml --repo "$GITHUB_REPOSITORY" --ref "$TAG"
"#;

    fn run(ci: &str, release: &str) -> Vec<String> {
        run_all(ci, release, NIGHTLY)
    }

    fn run_nightly(nightly: &str) -> Vec<String> {
        run_all(CI, RELEASE, nightly)
    }

    fn run_tagger(tagger: &str) -> Vec<String> {
        run_files(CI, RELEASE, NIGHTLY, tagger)
    }

    fn run_all(ci: &str, release: &str, nightly: &str) -> Vec<String> {
        run_files(ci, release, nightly, RELEASE_TAG)
    }

    fn run_files(ci: &str, release: &str, nightly: &str, tagger: &str) -> Vec<String> {
        lint(&[
            WorkflowFile {
                name: "ci.yml",
                source: ci,
            },
            WorkflowFile {
                name: "release.yml",
                source: release,
            },
            WorkflowFile {
                name: "nightly.yml",
                source: nightly,
            },
            WorkflowFile {
                name: "release-tag.yml",
                source: tagger,
            },
        ])
    }

    #[test]
    fn a_retired_runner_is_flagged_in_any_workflow_and_matrix_spelling() {
        let include = NIGHTLY.replace("os: macos-15", "os: macos-13");
        assert_flags(
            &run_nightly(&include),
            "job `bundle` runs on `macos-13`, a retired GitHub runner image",
        );
        // macos-14: brownouts from 2026-10, retired 2026-11-02
        // (actions/runner-images#13518).
        let brownout = NIGHTLY.replace("os: macos-15", "os: macos-14");
        assert_flags(
            &run_nightly(&brownout),
            "job `bundle` runs on `macos-14`, a retired GitHub runner image",
        );
        let list = NIGHTLY.replace(
            "[ubuntu-22.04, windows-latest]",
            "[ubuntu-20.04, windows-latest]",
        );
        assert_flags(&run_nightly(&list), "job `smoke` runs on `ubuntu-20.04`");
        let literal = CI.replace("runs-on: ubuntu-latest", "runs-on: windows-2019");
        assert_flags(
            &run(&literal, RELEASE),
            "ci.yml: job `test` runs on `windows-2019`",
        );
        // A matrix *name* that merely looks like a label is not a runner.
        let named = NIGHTLY.replace("name: macos-universal", "name: macos-13");
        assert!(!run_nightly(&named).iter().any(|v| v.contains("retired")));
        // Excluding a retired image is the opposite of running on it.
        let excluded = NIGHTLY.replace(
            "        os: [ubuntu-22.04, windows-latest]\n",
            "        os: [ubuntu-22.04, windows-latest]\n        exclude:\n          - os: macos-13\n",
        );
        assert!(!run_nightly(&excluded).iter().any(|v| v.contains("retired")));
        // The `{ group, labels }` form of `runs-on` names runners too.
        let grouped = CI.replace(
            "runs-on: ubuntu-latest",
            "runs-on:\n      group: big\n      labels: [ubuntu-20.04]",
        );
        assert_flags(
            &run(&grouped, RELEASE),
            "ci.yml: job `test` runs on `ubuntu-20.04`",
        );
    }

    #[test]
    fn path_filter_globs_follow_github_semantics() {
        let lib = "apps/desktop/src-tauri/src/lib.rs";
        assert!(paths_filter_covers(&[lib], lib));
        assert!(paths_filter_covers(
            &["apps/desktop/src-tauri/src/*.rs"],
            lib
        ));
        assert!(paths_filter_covers(&["apps/**"], lib));
        assert!(paths_filter_covers(&["**/lib.rs"], lib));
        assert!(paths_filter_covers(&["**/lib.?s"], lib));
        assert!(paths_filter_covers(
            &[".github/workflows/**"],
            ".github/workflows/nightly.yml"
        ));
        // `*` and `?` stop at a `/`.
        assert!(!paths_filter_covers(&["apps/*.rs"], lib));
        assert!(!paths_filter_covers(&["apps?desktop/**"], lib));
        // The last matching pattern decides; `!` excludes.
        assert!(!paths_filter_covers(&["apps/**", "!**/lib.rs"], lib));
        assert!(paths_filter_covers(
            &["apps/**", "!**/lib.rs", "**/src/lib.rs"],
            lib
        ));
        assert!(paths_filter_covers(&["!**/main.rs", "apps/**"], lib));
        assert!(!paths_filter_covers(&[], lib));
        assert!(!paths_filter_covers(
            &["apps/desktop/src-tauri/src/lib.rsx"],
            lib
        ));
    }

    #[test]
    fn the_nightly_must_keep_its_schedule_and_self_validate_on_prs() {
        let unscheduled = NIGHTLY.replace("  schedule:\n    - cron: \"0 7 * * *\"\n", "");
        assert_flags(&run_nightly(&unscheduled), "no `schedule` trigger");

        let no_pr = NIGHTLY.replace(
            "  pull_request:\n    paths:\n      - .github/workflows/nightly.yml\n      - apps/desktop/src-tauri/src/lib.rs\n      - apps/desktop/src-tauri/src/browser_args.rs\n      - apps/desktop/e2e/**\n",
            "",
        );
        assert_flags(
            &run_nightly(&no_pr),
            "no path-filtered `pull_request` trigger",
        );

        let unfiltered = NIGHTLY.replace(
            "    paths:\n      - .github/workflows/nightly.yml\n      - apps/desktop/src-tauri/src/lib.rs\n      - apps/desktop/src-tauri/src/browser_args.rs\n      - apps/desktop/e2e/**\n",
            "",
        );
        assert_flags(&run_nightly(&unfiltered), "has no `paths` filter");

        let not_self = NIGHTLY.replace("      - .github/workflows/nightly.yml\n", "");
        assert_flags(
            &run_nightly(&not_self),
            "does not cover `.github/workflows/nightly.yml`",
        );
        // An equivalent glob covers it just as well.
        let glob = NIGHTLY.replace(
            "      - .github/workflows/nightly.yml\n",
            "      - .github/workflows/**\n",
        );
        assert!(
            !run_nightly(&glob)
                .iter()
                .any(|v| v.contains("does not cover"))
        );

        // Main-window creation (the WebView2 automation flags) must also run
        // the nightly, so a refactor that drops them can't merge unrun.
        let no_window = NIGHTLY.replace(
            "      - apps/desktop/src-tauri/src/lib.rs\n      - apps/desktop/src-tauri/src/browser_args.rs\n",
            "",
        );
        let violations = run_nightly(&no_window);
        assert_flags(
            &violations,
            "does not cover `apps/desktop/src-tauri/src/lib.rs`",
        );
        assert_flags(
            &violations,
            "does not cover `apps/desktop/src-tauri/src/browser_args.rs`",
        );
    }

    #[test]
    fn the_nightly_is_a_hard_gate() {
        let job = NIGHTLY.replace("  smoke:\n", "  smoke:\n    continue-on-error: true\n");
        assert_flags(&run_nightly(&job), "job `smoke` sets `continue-on-error`");
        let step = NIGHTLY.replace(
            "      - run: npm test\n",
            "      - run: npm test\n        continue-on-error: ${{ matrix.os == 'windows-latest' }}\n",
        );
        assert_flags(
            &run_nightly(&step),
            "job `smoke` step 1 sets `continue-on-error`",
        );
        let explicit_off =
            NIGHTLY.replace("  smoke:\n", "  smoke:\n    continue-on-error: false\n");
        assert!(
            !run_nightly(&explicit_off)
                .iter()
                .any(|v| v.contains("continue-on-error"))
        );
    }

    #[test]
    fn bundle_uploads_must_fail_on_empty_and_cover_linux_and_windows() {
        let warn = NIGHTLY.replace("if-no-files-found: error", "if-no-files-found: warn");
        assert_flags(
            &run_nightly(&warn),
            "job `bundle` step 2 uploads without `if-no-files-found: error`",
        );
        let unset = NIGHTLY.replace("          if-no-files-found: error\n", "");
        assert_flags(
            &run_nightly(&unset),
            "uploads without `if-no-files-found: error`",
        );

        let no_windows = NIGHTLY.replace(
            "          - { name: windows-x64, os: windows-latest }\n",
            "",
        );
        assert_flags(
            &run_nightly(&no_windows),
            "no unconditional artifact upload runs on a Windows",
        );
        let no_linux = NIGHTLY.replace("          - { name: linux-x64, os: ubuntu-22.04 }\n", "");
        assert_flags(
            &run_nightly(&no_linux),
            "no unconditional artifact upload runs on a Linux",
        );
        // The smoke's failure-only screenshot upload neither needs `error`
        // nor counts as a bundle leg.
        let no_bundle_upload = NIGHTLY.replace(
            "      - uses: actions/upload-artifact@0123456789abcdef0123456789abcdef01234567 # v4.6.2\n        with:\n          name: curio-${{ matrix.name }}\n          if-no-files-found: error\n",
            "",
        );
        let violations = run_nightly(&no_bundle_upload);
        assert_flags(&violations, "on a Linux");
        assert!(!violations.iter().any(|v| v.contains("if-no-files-found")));
    }

    /// The reference bench step without its env block.
    const BENCH_ENV: &str =
        "      - env:\n          CRITERION_HOME: ${{ runner.temp }}/criterion\n        run: |\n";

    #[test]
    fn the_bench_must_run_criterion_statelessly() {
        // Issue #185's shape: criterion on its default (cached) home,
        // comparing against whatever baseline it finds there.
        let stateful = NIGHTLY
            .replace(BENCH_ENV, "      - run: |\n")
            .replace(" --noplot --discard-baseline", "");
        let violations = run_nightly(&stateful);
        assert_flags(
            &violations,
            "job `bench` step 1 runs `cargo bench` without `--discard-baseline`",
        );
        assert_flags(
            &violations,
            "job `bench` step 1 runs `cargo bench` without `CRITERION_HOME` on a `runner.temp` dir",
        );

        // A home inside the cached target dir is no fresher than the default.
        let cached_home = NIGHTLY.replace(
            "CRITERION_HOME: ${{ runner.temp }}/criterion",
            "CRITERION_HOME: target/criterion",
        );
        assert_flags(&run_nightly(&cached_home), "without `CRITERION_HOME`");
        // Naming `runner.temp` is not enough: the path must stay under it.
        for home in [
            "${{ runner.temp }}/../work/curio-rss/curio-rss/target/criterion",
            "${{ runner.temp }}/criterion/..",
            "${{ runner.temp }}criterion",
            "target/${{ runner.temp }}/criterion",
        ] {
            let escaped = NIGHTLY.replace(
                "CRITERION_HOME: ${{ runner.temp }}/criterion",
                &format!("CRITERION_HOME: \"{home}\""),
            );
            assert_flags(&run_nightly(&escaped), "without `CRITERION_HOME`");
        }
        for home in ["${{ runner.temp }}", "${{ runner.temp }}/bench/criterion"] {
            let fine = NIGHTLY.replace(
                "CRITERION_HOME: ${{ runner.temp }}/criterion",
                &format!("CRITERION_HOME: \"{home}\""),
            );
            assert_eq!(run_nightly(&fine), Vec::<String>::new(), "{home}");
        }

        // Job-level env counts.
        let job_env = NIGHTLY.replace(BENCH_ENV, "      - run: |\n").replace(
            "    runs-on: ubuntu-22.04\n    steps:\n      - run: |\n",
            "    runs-on: ubuntu-22.04\n    env:\n      CRITERION_HOME: ${{ runner.temp }}/criterion\n    steps:\n      - run: |\n",
        );
        assert_eq!(run_nightly(&job_env), Vec::<String>::new());
    }

    #[test]
    fn the_bench_pipe_must_fail_when_the_bench_does() {
        let no_pipefail = NIGHTLY.replace("          set -o pipefail\n", "");
        assert_flags(
            &run_nightly(&no_pipefail),
            "pipes `cargo bench` without `set -o pipefail`",
        );
        // `shell: bash` is `bash -eo pipefail` on GitHub.
        let bash = no_pipefail.replace(
            "      - env:\n          CRITERION_HOME",
            "      - shell: bash\n        env:\n          CRITERION_HOME",
        );
        assert_eq!(run_nightly(&bash), Vec::<String>::new());

        // So is a job- or workflow-level `defaults.run.shell: bash`, and a
        // custom shell that passes the option itself.
        let job_default = no_pipefail.replace(
            "    runs-on: ubuntu-22.04\n    steps:\n      - env:\n          CRITERION_HOME",
            "    runs-on: ubuntu-22.04\n    defaults:\n      run:\n        shell: bash\n    steps:\n      - env:\n          CRITERION_HOME",
        );
        assert_eq!(run_nightly(&job_default), Vec::<String>::new());
        let workflow_default =
            no_pipefail.replace("\njobs:\n", "\ndefaults:\n  run:\n    shell: bash\njobs:\n");
        assert_ne!(workflow_default, no_pipefail);
        assert_eq!(run_nightly(&workflow_default), Vec::<String>::new());
        let custom = no_pipefail.replace(
            "      - env:\n          CRITERION_HOME",
            "      - shell: bash --noprofile --norc -euo pipefail {0}\n        env:\n          CRITERION_HOME",
        );
        assert_eq!(run_nightly(&custom), Vec::<String>::new());
        // A step-level shell overrides the defaults: `sh` has no pipefail.
        let sh_over_default = job_default.replace(
            "      - env:\n          CRITERION_HOME",
            "      - shell: sh\n        env:\n          CRITERION_HOME",
        );
        assert_flags(
            &run_nightly(&sh_over_default),
            "pipes `cargo bench` without `set -o pipefail`",
        );
    }

    #[test]
    fn only_a_set_that_enables_pipefail_counts() {
        for script in [
            "          set -eo pipefail\n",
            "          set -euo pipefail\n",
            "          set -e -o pipefail # crashed bench stays red\n",
            // A `;` glued to the option is still a separator.
            "          set -euo pipefail; echo start\n",
            // Off, then back on before the bench.
            "          set +o pipefail\n          set -o pipefail\n",
        ] {
            let nightly = NIGHTLY.replace("          set -o pipefail\n", script);
            assert_eq!(run_nightly(&nightly), Vec::<String>::new(), "{script}");
        }
        for script in [
            // Mentioning it is not setting it.
            "          # set -o pipefail\n",
            "          echo pipefail\n",
            "          set -e # no pipefail\n",
            "          set -o errexit pipefail\n",
            // Turned back off before the pipe.
            "          set -o pipefail\n          set +o pipefail\n",
        ] {
            let nightly = NIGHTLY.replace("          set -o pipefail\n", script);
            assert_flags(
                &run_nightly(&nightly),
                "pipes `cargo bench` without `set -o pipefail`",
            );
        }
        // Even `shell: bash` loses it to an explicit `set +o pipefail`.
        let off_under_bash = NIGHTLY
            .replace("          set -o pipefail\n", "          set +o pipefail\n")
            .replace(
                "      - env:\n          CRITERION_HOME",
                "      - shell: bash\n        env:\n          CRITERION_HOME",
            );
        assert_flags(
            &run_nightly(&off_under_bash),
            "pipes `cargo bench` without `set -o pipefail`",
        );

        // On the bench's own line, ahead of it, counts…
        let same_line = NIGHTLY.replace(
            "          set -o pipefail\n          cargo bench",
            "          set -o pipefail; cargo bench",
        );
        assert_ne!(same_line, NIGHTLY);
        assert_eq!(run_nightly(&same_line), Vec::<String>::new());
        // …after the bench is too late to cover its pipe…
        let too_late = NIGHTLY.replace("          set -o pipefail\n", "").replace(
            "            | tee bench-output.txt\n",
            "            | tee bench-output.txt\n          set -o pipefail\n",
        );
        assert_flags(
            &run_nightly(&too_late),
            "pipes `cargo bench` without `set -o pipefail`",
        );
        // …and so is turning it off after the bench (it already ran under it).
        let off_after = NIGHTLY.replace(
            "            | tee bench-output.txt\n",
            "            | tee bench-output.txt\n          set +o pipefail\n",
        );
        assert_ne!(off_after, NIGHTLY);
        assert_eq!(run_nightly(&off_after), Vec::<String>::new());
    }

    #[test]
    fn the_bench_log_is_checked_and_tracked_with_a_hard_alert() {
        let unchecked = NIGHTLY.replace(
            "      - run: cargo run -p xtask -- bench-output bench-output.txt --expect cold_start/open_and_first_page_50k\n",
            "",
        );
        assert_flags(
            &run_nightly(&unchecked),
            "job `bench` has no `xtask -- bench-output` step between the bench and the tracker",
        );
        // The check must name the tracked benchmark: without `--expect`, any
        // one unrelated result would pass it.
        let unnamed = NIGHTLY.replace(" --expect cold_start/open_and_first_page_50k", "");
        assert_ne!(unnamed, NIGHTLY);
        assert_flags(
            &run_nightly(&unnamed),
            "job `bench` checks the bench log without `--expect <bench-id>`",
        );

        let soft = NIGHTLY.replace("fail-on-alert: true", "fail-on-alert: false");
        assert_flags(
            &run_nightly(&soft),
            "tracks the bench without `fail-on-alert: true`",
        );
        let quoted = NIGHTLY.replace("fail-on-alert: true", "fail-on-alert: \"true\"");
        assert_eq!(run_nightly(&quoted), Vec::<String>::new());

        let no_threshold = NIGHTLY.replace("          alert-threshold: \"130%\"\n", "");
        assert_flags(
            &run_nightly(&no_threshold),
            "tracks the bench without an `alert-threshold`",
        );

        let untracked = NIGHTLY.replace(
            "      - uses: benchmark-action/github-action-benchmark@0123456789abcdef0123456789abcdef01234567 # v1.22.2\n        with:\n          tool: cargo\n          output-file-path: bench-output.txt\n          alert-threshold: \"130%\"\n          fail-on-alert: true\n",
            "",
        );
        assert_flags(
            &run_nightly(&untracked),
            "no later `benchmark-action/github-action-benchmark@…` step",
        );

        let no_bench = NIGHTLY.replace("--output-format bencher", "--output-format criterion");
        assert_flags(
            &run_nightly(&no_bench),
            "no step runs `cargo bench … --output-format bencher`",
        );
    }

    #[test]
    fn a_missing_nightly_is_flagged() {
        let without = lint(&[
            WorkflowFile {
                name: "ci.yml",
                source: CI,
            },
            WorkflowFile {
                name: "release.yml",
                source: RELEASE,
            },
            WorkflowFile {
                name: "release-tag.yml",
                source: RELEASE_TAG,
            },
        ]);
        assert_eq!(without, vec!["nightly.yml: missing".to_owned()]);
    }

    fn assert_flags(violations: &[String], needle: &str) {
        assert!(
            violations.iter().any(|v| v.contains(needle)),
            "expected a violation containing `{needle}`, got {violations:#?}"
        );
    }

    #[test]
    fn the_reference_shape_is_clean() {
        assert_eq!(run(CI, RELEASE), Vec::<String>::new());
    }

    #[test]
    fn pin_rule_accepts_shas_local_paths_and_docker_digests_only() {
        assert!(is_pinned(&format!("actions/checkout@{SHA}")));
        assert!(is_pinned(&format!("github/codeql-action/init@{SHA}")));
        assert!(is_pinned("./.github/workflows/ci.yml"));
        assert!(is_pinned("docker://alpine@sha256:abc"));
        assert!(!is_pinned("actions/checkout@v4"));
        assert!(!is_pinned("dtolnay/rust-toolchain@stable"));
        assert!(!is_pinned("docker://alpine:3"));
        assert!(!is_pinned("actions/checkout"));
        assert!(!is_pinned(&format!("checkout@{SHA}")));
        // Uppercase / short SHAs are not what `git rev-parse` prints.
        assert!(!is_pinned(&format!(
            "actions/checkout@{}",
            SHA.to_ascii_uppercase()
        )));
        assert!(!is_pinned("actions/checkout@0123456"));
    }

    #[test]
    fn flags_mutable_tags_and_missing_version_comments() {
        let pinned = format!("actions/setup-node@{SHA} # v4.4.0");
        let violations = run(CI, &RELEASE.replace(&pinned, "actions/setup-node@v4"));
        assert_flags(&violations, "uses `actions/setup-node@v4` — pin it");
        assert_flags(&violations, "has no trailing `# <version>` comment");

        let bare = RELEASE.replace(&pinned, &format!("actions/setup-node@{SHA}"));
        assert_flags(&run(CI, &bare), "release.yml:19:");

        let job_level = RELEASE.replace(
            "uses: ./.github/workflows/ci.yml",
            "uses: someone/workflows/.github/workflows/ci.yml@main # main",
        );
        assert_flags(&run(CI, &job_level), "job `gate` calls");
    }

    #[test]
    fn release_without_the_ci_gate_is_flagged() {
        // The v0.4.0 shape: build runs straight off the tag.
        let ungated = RELEASE.replace("    needs: gate\n", "");
        assert_flags(
            &run(CI, &ungated),
            "job `build` does not (transitively) `needs` the CI gate",
        );

        let no_gate = RELEASE.replace("  gate:\n    uses: ./.github/workflows/ci.yml\n", "");
        assert_flags(
            &run(CI, &no_gate),
            "no job calls `./.github/workflows/ci.yml`",
        );

        let not_callable = CI.replace("  workflow_call:\n", "");
        assert_flags(&run(&not_callable, RELEASE), "no `workflow_call` trigger");
    }

    #[test]
    fn gate_reachability_is_transitive_and_cycle_safe() {
        // `release` → `sign` → `build` → `gate` is clean (above); a
        // needs-cycle that never reaches the gate must terminate and flag.
        let cyclic = RELEASE
            .replace("    needs: build\n", "    needs: release\n")
            .replace("    needs: gate\n", "    needs: sign\n");
        let violations = run(CI, &cyclic);
        assert_flags(&violations, "job `sign` does not");
        assert_flags(&violations, "job `build` does not");

        let dangling = RELEASE.replace("    needs: gate\n", "    needs: nowhere\n");
        assert_flags(&run(CI, &dangling), "job `build` does not");
    }

    #[test]
    fn a_status_function_that_outlives_a_red_gate_is_flagged() {
        // `needs` alone is not enough: `always()` & co. run the job anyway.
        for condition in [
            "${{ always() }}",
            "always()",
            "${{ !cancelled() }}",
            "${{ success() || failure() }}",
            "${{ ALWAYS ( ) && github.event_name == 'push' }}",
        ] {
            let bypass = RELEASE.replace(
                "    needs: gate\n",
                &format!("    needs: gate\n    if: {condition}\n"),
            );
            assert_flags(&run(CI, &bypass), "job `build` has `if: …");
        }

        // A downstream job, two hops from the gate, is held to it too.
        let downstream = RELEASE.replace(
            "    needs: [sign]\n",
            "    needs: [sign]\n    if: ${{ always() }}\n",
        );
        assert_flags(&run(CI, &downstream), "job `release` has `if: … always()");

        // Ordinary conditions keep the implied `success()`.
        for condition in [
            "${{ startsWith(github.ref, 'refs/tags/v') }}",
            "success()",
            "true",
        ] {
            let plain = RELEASE.replace(
                "    needs: gate\n",
                &format!("    needs: gate\n    if: {condition}\n"),
            );
            assert_eq!(run(CI, &plain), Vec::<String>::new(), "{condition}");
        }
    }

    #[test]
    fn the_signing_job_runs_nothing_but_the_install_before_the_key() {
        let before_key = |step: &str| {
            RELEASE.replace(
                "      - name: Sign\n",
                &format!("      - run: {step}\n      - name: Sign\n"),
            )
        };
        for (step, word) in [
            ("npm run build", "npm"),
            ("npx vite build", "npx"),
            ("node ./dist/x.js", "node"),
            ("npm ci --ignore-scripts && npm run build", "npm"),
            ("make sign", "make"),
        ] {
            assert_flags(
                &run(CI, &before_key(step)),
                &format!("runs `{word}` before the signing step"),
            );
        }

        let env_write = before_key("echo NODE_OPTIONS=--require ./evil.js >> \"$GITHUB_ENV\"");
        assert_flags(&run(CI, &env_write), "writes `GITHUB_ENV` at or before");
        let path_write = before_key("echo ./evil >> $GITHUB_PATH");
        assert_flags(&run(CI, &path_write), "writes `GITHUB_PATH` at or before");

        for step in [
            "python3 -m http.server &",
            "nohup ./watch.sh",
            "(sleep 600; cat /proc/*/environ) & disown",
        ] {
            assert_flags(&run(CI, &before_key(step)), "leaves a background process");
        }

        // The key step itself must not export the key to later steps.
        let exported = RELEASE.replace(
            "        run: ./node_modules/.bin/tauri signer sign Curio.app.tar.gz\n",
            "        run: |\n          ./node_modules/.bin/tauri signer sign Curio.app.tar.gz\n          echo \"k=$TAURI_SIGNING_PRIVATE_KEY\" >> \"$GITHUB_OUTPUT\"\n",
        );
        assert_flags(&run(CI, &exported), "writes `GITHUB_OUTPUT` at or before");

        // Code-injecting env vars, on a step or the whole job.
        let step_env = RELEASE.replace(
            "          TAURI_SIGNING_PRIVATE_KEY_PASSWORD:",
            "          NODE_OPTIONS: --require ./evil.js\n          TAURI_SIGNING_PRIVATE_KEY_PASSWORD:",
        );
        assert_flags(
            &run(CI, &step_env),
            "sets `NODE_OPTIONS` in the signing job",
        );
        let job_env = RELEASE.replace(
            "  sign:\n    needs: build\n",
            "  sign:\n    needs: build\n    env:\n      BASH_ENV: ./evil.sh\n",
        );
        assert_flags(&run(CI, &job_env), "sets `BASH_ENV` for every step");
    }

    #[test]
    fn the_signing_job_may_do_plain_work_around_the_key() {
        // Harmless shell ahead of the key, a line-continued install, and
        // node / output files AFTER the key step (the manifest writer) pass.
        let fine = RELEASE
            .replace(
                "      - run: npm ci --ignore-scripts\n",
                "      - run: |\n          set -euo pipefail\n          npm ci \\\n            --ignore-scripts 2>&1 | tee install.log\n          ls -l dist && echo ok\n",
            )
            .replace(
                "  release:\n",
                "      - run: node -e 'require(\"fs\")' && echo \"v=1\" >> \"$GITHUB_OUTPUT\"\n  release:\n",
            );
        assert_eq!(run(CI, &fine), Vec::<String>::new());
    }

    #[test]
    fn signing_key_in_the_build_step_is_flagged() {
        // The v0.4.0 shape: the key sits on `npm run tauri build`.
        let leaked = RELEASE.replace(
            "      - run: npm run tauri build",
            "      - env:\n          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}\n        run: npm run tauri build",
        );
        assert_flags(&run(CI, &leaked), "reaches 2 steps");
    }

    #[test]
    fn signing_key_at_job_or_workflow_scope_is_flagged() {
        let job_env = RELEASE.replace(
            "  sign:\n    needs: build\n",
            "  sign:\n    needs: build\n    env:\n      K: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}\n",
        );
        assert_flags(
            &run(CI, &job_env),
            "`jobs.sign.env.K` references the updater signing key",
        );

        let workflow_env =
            format!("env:\n  K: ${{{{ secrets['TAURI_SIGNING_PRIVATE_KEY'] }}}}\n{RELEASE}");
        assert_flags(
            &run(CI, &workflow_env),
            "`env.K` references the updater signing key",
        );
    }

    #[test]
    fn mentioning_the_env_name_without_the_secret_is_fine() {
        // A dry-run step may set the variable from a throwaway key.
        let dry_run = RELEASE.replace(
            "      - run: npm ci --ignore-scripts\n",
            "      - run: npm ci --ignore-scripts\n      - run: TAURI_SIGNING_PRIVATE_KEY=\"$(cat throwaway.key)\" echo dry\n",
        );
        assert_eq!(run(CI, &dry_run), Vec::<String>::new());
    }

    #[test]
    fn secret_forwarding_wholesale_is_flagged() {
        let inherit = RELEASE.replace(
            "    uses: ./.github/workflows/ci.yml\n",
            "    uses: ./.github/workflows/ci.yml\n    secrets: inherit\n",
        );
        assert_flags(
            &run(CI, &inherit),
            "`jobs.gate.secrets: inherit` forwards every secret",
        );

        let dumped = RELEASE.replace(
            "      - run: npm ci --ignore-scripts\n",
            "      - run: npm ci --ignore-scripts\n      - env:\n          ALL: ${{ toJSON(secrets) }}\n        run: echo hi\n",
        );
        assert_flags(&run(CI, &dumped), "serializes every secret");
    }

    #[test]
    fn the_signing_step_must_be_a_build_free_signer_run() {
        let builds = RELEASE.replace(
            "run: ./node_modules/.bin/tauri signer sign Curio.app.tar.gz",
            "run: npx tauri build && ./node_modules/.bin/tauri signer sign Curio.app.tar.gz",
        );
        let violations = run(CI, &builds);
        assert_flags(&violations, "runs `build`");
        assert_flags(&violations, "runs `npx`");

        let not_signing = RELEASE.replace("signer sign", "signer generate");
        assert_flags(&run(CI, &not_signing), "does not run `signer sign`");

        let action = RELEASE.replace(
            "        run: ./node_modules/.bin/tauri signer sign Curio.app.tar.gz\n",
            &format!("        uses: tauri-apps/tauri-action@{SHA} # v0\n"),
        );
        let violations = run(CI, &action);
        assert_flags(&violations, "hands the signing key to an action");
        assert_flags(&violations, "only checkout/setup-node/artifact actions");
    }

    #[test]
    fn the_signing_job_installs_without_scripts_and_compiles_nothing() {
        let scripts = RELEASE.replace("npm ci --ignore-scripts", "npm ci");
        assert_flags(&run(CI, &scripts), "without `--ignore-scripts`");

        let compiles = RELEASE.replace("npm ci --ignore-scripts", "cargo build -p curio-desktop");
        assert_flags(&run(CI, &compiles), "runs cargo in the signing job");
    }

    #[test]
    fn no_signing_step_at_all_is_flagged() {
        let unsigned = RELEASE.replace("secrets.TAURI_SIGNING_PRIVATE_KEY", "secrets.OTHER");
        assert_flags(
            &run(CI, &unsigned),
            "no workflow step receives the updater signing key",
        );
    }

    /// The `RELEASE` fixture with its sign job selecting `environment` (YAML
    /// text) and its key step guarded by `key_if` (`None` = no `if:`).
    fn with_sign(environment: &str, key_if: Option<&str>) -> String {
        let key_step = key_if.map_or_else(
            || "      - name: Sign\n".to_owned(),
            |condition| format!("      - name: Sign\n        if: {condition}\n"),
        );
        RELEASE
            .replace(ENV_LINE, &format!("    environment: {environment}\n"))
            .replace(KEY_STEP, &key_step)
    }

    /// `violations` without the tag-ref rule's own message, for the
    /// environment checks that hold for any key-step condition.
    fn besides_tag_ref_rule(violations: Vec<String>) -> Vec<String> {
        violations
            .into_iter()
            .filter(|v| !v.contains("signs with the real key on"))
            .collect()
    }

    #[test]
    fn the_signing_job_must_deploy_to_the_signing_environment() {
        let dropped = RELEASE.replace(ENV_LINE, "");
        assert_flags(
            &run(CI, &dropped),
            "job `sign` holds the signing key but declares no `environment:`",
        );
        let renamed = RELEASE.replace(ENV_LINE, "    environment: production\n");
        assert_flags(
            &run(CI, &renamed),
            "job `sign` deploys to `production`, not `updater-signing`",
        );
        let mapping = RELEASE.replace(
            ENV_LINE,
            "    environment:\n      name: ${{ startsWith(github.ref, 'refs/tags/v') && 'updater-signing' || '' }}\n      url: https://example.invalid\n",
        );
        assert_eq!(run(CI, &mapping), Vec::<String>::new());
        let empty_mapping = RELEASE.replace(
            ENV_LINE,
            "    environment:\n      url: https://example.invalid\n",
        );
        assert_flags(&run(CI, &empty_mapping), "declares no `environment:`");

        // A tag-push-only release may deploy unconditionally (there is no
        // dry run to keep out of the tag-only rule); what fails it is the
        // missing dispatch trigger the tagger needs, not the environment.
        let push_only = with_sign("updater-signing", None).replace("  workflow_dispatch:\n", "");
        let violations = run(CI, &push_only);
        assert!(
            !violations.iter().any(|v| v.contains("environment")),
            "{violations:#?}"
        );
        assert_flags(&violations, "release.yml: no `workflow_dispatch` trigger");
    }

    #[test]
    fn the_dry_run_selects_the_environment_exactly_when_the_key_step_runs() {
        let tag_ref = "startsWith(github.ref, 'refs/tags/v')";
        let conditional = "${{ startsWith(github.ref, 'refs/tags/v') && 'updater-signing' || '' }}";

        // The committed shape: environment and key step share the condition,
        // in either `if:` spelling.
        assert_eq!(
            run(CI, &with_sign(conditional, Some(tag_ref))),
            Vec::<String>::new()
        );
        assert_eq!(
            run(
                CI,
                &with_sign(conditional, Some(&format!("${{{{ {tag_ref} }}}}")))
            ),
            Vec::<String>::new()
        );

        // Unconditional: the dispatch run would wait on the tag-only rule.
        assert_flags(
            &run(CI, &with_sign("updater-signing", Some(tag_ref))),
            "job `sign` always deploys to `updater-signing`, so the `workflow_dispatch` dry run",
        );

        // The key step outruns the environment: unguarded, or guarded by a
        // different condition.
        assert_flags(
            &run(CI, &with_sign(conditional, None)),
            "but the key step runs on `every event`",
        );
        assert_flags(
            &run(CI, &with_sign(conditional, Some("always()"))),
            "but the key step runs on `always()`",
        );

        // A conditional that never names the signing environment.
        assert_flags(
            &run(
                CI,
                &with_sign(
                    "${{ startsWith(github.ref, 'refs/tags/v') && 'production' || '' }}",
                    Some(tag_ref),
                ),
            ),
            "not `updater-signing`",
        );
    }

    /// `&&` binds tighter than `||`: a key-step condition with a top-level
    /// `||` must be parenthesised in the environment expression, or the job
    /// deploys to `true` while the real key step runs.
    #[test]
    fn a_top_level_or_in_the_environment_condition_must_be_parenthesised() {
        let either = "startsWith(github.ref, 'refs/tags/v') || github.ref_type == 'tag'";
        let unparenthesised = format!("${{{{ {either} && 'updater-signing' || '' }}}}");
        assert_flags(
            &run(CI, &with_sign(&unparenthesised, Some(either))),
            "whose top-level `||` binds looser than `&&`",
        );
        let parenthesised = format!("${{{{ ({either}) && 'updater-signing' || '' }}}}");
        assert_eq!(
            besides_tag_ref_rule(run(CI, &with_sign(&parenthesised, Some(either)))),
            Vec::<String>::new()
        );
        // The key step's own `if:` may carry the same parentheses.
        assert_eq!(
            besides_tag_ref_rule(run(
                CI,
                &with_sign(&parenthesised, Some(&format!("(({either}))")))
            )),
            Vec::<String>::new()
        );
        // An `||` inside parentheses or a string literal is not top-level.
        let nested = "startsWith(github.ref, 'refs/tags/v') && (github.ref_type == 'tag' || github.ref_type == 'branch')";
        assert_eq!(
            besides_tag_ref_rule(run(
                CI,
                &with_sign(
                    &format!("${{{{ {nested} && 'updater-signing' || '' }}}}"),
                    Some(nested)
                )
            )),
            Vec::<String>::new()
        );
        let literal = "github.ref_name != 'a || b'";
        assert_eq!(
            besides_tag_ref_rule(run(
                CI,
                &with_sign(
                    &format!("${{{{ {literal} && 'updater-signing' || '' }}}}"),
                    Some(literal)
                )
            )),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_real_release_is_decided_by_the_tag_ref_alone() {
        // The v0.5.0 shape: key step and environment on the event name — a
        // dispatch on the tag would be a dry run.
        let by_event = with_sign(
            "${{ github.event_name == 'push' && 'updater-signing' || '' }}",
            Some("github.event_name == 'push'"),
        );
        let violations = run(CI, &by_event);
        assert_flags(
            &violations,
            "release.yml: job `sign` step 3 signs with the real key on `github.event_name=='push'`, not on the tag ref",
        );
        assert_flags(
            &violations,
            "`jobs.sign.environment` reads `github.event_name`",
        );

        // Any other step deciding by the event splits a dispatch on the tag.
        let draft_by_event = RELEASE.replace(
            "      - uses: softprops/action-gh-release@",
            "      - if: github.event_name == 'push'\n        uses: softprops/action-gh-release@",
        );
        assert_flags(
            &run(CI, &draft_by_event),
            "`jobs.release.steps.1.if` reads `github.event_name`",
        );

        // No dispatch trigger: the tagger's dispatch has nothing to run.
        let push_only = RELEASE.replace("  workflow_dispatch:\n", "");
        assert_flags(
            &run(CI, &push_only),
            "release.yml: no `workflow_dispatch` trigger — release-tag.yml cannot dispatch",
        );
    }

    #[test]
    fn the_tagger_is_required_and_runs_on_main_pushes_alone() {
        let missing = lint(&[
            WorkflowFile {
                name: "ci.yml",
                source: CI,
            },
            WorkflowFile {
                name: "release.yml",
                source: RELEASE,
            },
            WorkflowFile {
                name: "nightly.yml",
                source: NIGHTLY,
            },
        ]);
        assert_eq!(missing, vec!["release-tag.yml: missing".to_owned()]);

        for (from, to) in [
            ("on:\n  push:\n", "on:\n  pull_request:\n  push:\n"),
            ("on:\n  push:\n", "on:\n  workflow_dispatch:\n  push:\n"),
            ("    branches: [main]\n", "    branches: [main, release]\n"),
            (
                "    branches: [main]\n",
                "    branches: [main]\n    tags: [\"v*\"]\n",
            ),
            ("    branches: [main]\n", ""),
        ] {
            assert_flags(
                &run_tagger(&RELEASE_TAG.replace(from, to)),
                "must run on `push: branches: [main]` alone",
            );
        }

        let forks = RELEASE_TAG.replace("    if: ${{ !github.event.repository.fork }}\n", "");
        assert_flags(
            &run_tagger(&forks),
            "job `plan` has no `if: ${{ !github.event.repository.fork }}`",
        );
    }

    #[test]
    fn the_tagger_holds_no_secret_and_writes_from_a_codeless_job() {
        for secret in [
            "${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}",
            "${{ secrets.RELEASE_PAT }}",
            "${{ secrets['GH_PAT'] }}",
        ] {
            let leaked = RELEASE_TAG.replace(
                "GH_TOKEN: ${{ github.token }}",
                &format!("GH_TOKEN: {secret}"),
            );
            assert_flags(
                &run_tagger(&leaked),
                "`jobs.tag.env.GH_TOKEN` reads a secret",
            );
        }
        let token = RELEASE_TAG.replace("${{ github.token }}", "${{ secrets.GITHUB_TOKEN }}");
        assert_eq!(run_tagger(&token), Vec::<String>::new());

        let environment = RELEASE_TAG.replace(
            "    needs: plan\n",
            "    needs: plan\n    environment: updater-signing\n",
        );
        assert_flags(
            &run_tagger(&environment),
            "job `tag` deploys to an environment",
        );

        let workflow_write = RELEASE_TAG.replace(
            "permissions:\n  contents: read\n",
            "permissions: write-all\n",
        );
        assert_flags(
            &run_tagger(&workflow_write),
            "workflow-level `permissions` grant write",
        );

        let checkout = RELEASE_TAG.replace(
            "      - name: Tag\n",
            &format!("      - uses: actions/checkout@{SHA} # v4.4.0\n      - name: Tag\n"),
        );
        assert_flags(
            &run_tagger(&checkout),
            "job `tag` step 1 uses `actions/checkout@",
        );
        let builds = RELEASE_TAG.replace(
            "      - name: Dispatch\n",
            "      - run: cargo run -p xtask -- release-tag\n      - name: Dispatch\n",
        );
        assert_flags(
            &run_tagger(&builds),
            "job `tag` step 2 runs `cargo` beside a write token",
        );
        let reusable = RELEASE_TAG.replace(
            "  tag:\n    needs: plan\n",
            "  call:\n    needs: plan\n    permissions:\n      contents: write\n    uses: ./.github/workflows/ci.yml\n  tag:\n    needs: plan\n",
        );
        assert_flags(
            &run_tagger(&reusable),
            "job `call` grants write to a reusable workflow",
        );

        // Write grants on the read-only planner would put the token beside
        // cargo and the checkout.
        let planner_writes = RELEASE_TAG.replace(
            "    runs-on: ubuntu-latest\n    outputs:\n",
            "    runs-on: ubuntu-latest\n    permissions:\n      contents: write\n    outputs:\n",
        );
        let violations = run_tagger(&planner_writes);
        assert_flags(&violations, "job `plan` step 1 uses `actions/checkout@");
        assert_flags(
            &violations,
            "job `plan` step 2 runs `cargo` beside a write token",
        );
    }

    #[test]
    fn the_tagger_creates_by_api_never_moves_and_dispatches_on_the_tag() {
        let no_ref = RELEASE_TAG.replace(
            "          gh api \"repos/$GITHUB_REPOSITORY/git/refs\" -f ref=\"refs/tags/$TAG\" -f sha=\"$object\"\n",
            "",
        );
        assert_flags(
            &run_tagger(&no_ref),
            "no `gh api …/git/refs` step creates the tag",
        );

        let no_dispatch = RELEASE_TAG.replace(
            "        run: gh workflow run release.yml --repo \"$GITHUB_REPOSITORY\" --ref \"$TAG\"\n",
            "        run: gh workflow run release.yml --repo \"$GITHUB_REPOSITORY\"\n",
        );
        assert_flags(
            &run_tagger(&no_dispatch),
            "no step runs `gh workflow run release.yml --ref <tag>`",
        );

        for mover in [
            "gh api -X PATCH \"repos/$GITHUB_REPOSITORY/git/refs/tags/$TAG\" -f sha=\"$object\" -F force=true",
            "gh api --method DELETE \"repos/$GITHUB_REPOSITORY/git/refs/tags/$TAG\"",
        ] {
            let moved = RELEASE_TAG.replace(
                "      - name: Dispatch\n",
                &format!("      - run: {mover}\n      - name: Dispatch\n"),
            );
            assert_flags(&run_tagger(&moved), "which moves or deletes a ref");
        }
    }

    #[test]
    fn only_the_tagger_creates_tags() {
        for step in [
            "git tag -a v1.0.0 -m release && git push origin refs/tags/v1.0.0",
            "gh release create v1.0.0 --draft",
            "gh api repos/o/r/git/refs -f ref=refs/tags/v1.0.0 -f sha=abc",
        ] {
            let tagging_ci = CI.replace(
                "      - run: cargo test\n",
                &format!("      - run: {step}\n"),
            );
            let violations = run(&tagging_ci, RELEASE);
            assert_flags(&violations, "ci.yml: job `test` step 2 runs `");
            assert_flags(
                &violations,
                "which creates a tag — only release-tag.yml creates tags",
            );
        }
        let listing = CI.replace(
            "      - run: cargo test\n",
            "      - run: git tag --list && git describe --tags\n",
        );
        assert_eq!(run(&listing, RELEASE), Vec::<String>::new());

        let named = RELEASE.replace(
            "          draft: true\n",
            "          draft: true\n          tag_name: v9.9.9\n",
        );
        assert_flags(
            &run(CI, &named),
            "sets `tag_name`, which creates a missing tag",
        );
    }

    #[test]
    fn expression_helpers_respect_literals_and_parentheses() {
        // Whitespace inside a string literal is significant.
        assert_eq!(compact_expression("a == 'x y'"), "a=='x y'");
        assert_ne!(
            compact_expression("a == 'x y'"),
            compact_expression("a == 'xy'")
        );
        assert_eq!(compact_expression("a == 'it''s  b'"), "a=='it''s  b'");

        assert!(has_top_level_or("a||b"));
        assert!(has_top_level_or("(a)||(b)"));
        assert!(!has_top_level_or("(a||b)"));
        assert!(!has_top_level_or("a&&'x||y'"));
        assert!(!has_top_level_or("a&&b"));

        assert_eq!(without_outer_parens("((a||b))"), "a||b");
        assert_eq!(without_outer_parens("(a)&&(b)"), "(a)&&(b)");
        assert_eq!(without_outer_parens("(a==')')"), "a==')'");
        assert_eq!(without_outer_parens("a"), "a");
    }

    #[test]
    fn release_assets_and_attestation_are_required() {
        for (asset, message) in [
            (
                "            dist/SHA256SUMS\n",
                "does not attach SHA256SUMS",
            ),
            ("            dist/Curio.cdx.json\n", "a CycloneDX SBOM"),
            (
                "            dist/Curio-provenance.sigstore.json\n",
                "the provenance attestation bundle",
            ),
        ] {
            assert_flags(&run(CI, &RELEASE.replace(asset, "")), message);
        }

        let unattested = RELEASE.replace(
            &format!(
                "      - uses: actions/attest-build-provenance@{SHA} # v4.2.2\n        with:\n          subject-checksums: dist/SHA256SUMS\n"
            ),
            "",
        );
        assert_flags(
            &run(CI, &unattested),
            "not preceded by an actions/attest-build-provenance",
        );

        let public = RELEASE.replace("draft: true", "draft: false");
        assert_flags(&run(CI, &public), "must create a DRAFT release");

        let none = RELEASE.replace("softprops/action-gh-release", "someone/else");
        assert_flags(&run(CI, &none), "no softprops/action-gh-release step");
    }

    #[test]
    fn broken_or_missing_files_are_reported() {
        assert_flags(&run(CI, "jobs: [unclosed"), "release.yml: not valid YAML");
        assert_flags(&lint(&[]), "release.yml: missing");
        let only_release = lint(&[WorkflowFile {
            name: "release.yml",
            source: RELEASE,
        }]);
        assert_flags(&only_release, "ci.yml: missing");
    }

    #[test]
    fn triggers_and_needs_accept_every_yaml_spelling() {
        let doc = |src: &str| yaml::from_str(src).unwrap_or(Value::Null);
        assert!(triggers(&doc("on: workflow_call")).contains("workflow_call"));
        assert!(triggers(&doc("on: [push, workflow_call]")).contains("workflow_call"));
        assert!(triggers(&doc("on:\n  workflow_call:\n")).contains("workflow_call"));
        assert_eq!(triggers(&doc("name: x")), BTreeSet::new());
        assert_eq!(needs(&doc("needs: a")), vec!["a"]);
        assert_eq!(needs(&doc("needs: [a, b]")), vec!["a", "b"]);
        assert_eq!(needs(&doc("runs-on: x")), Vec::<&str>::new());
    }
}
