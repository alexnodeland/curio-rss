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
//!    `toJSON(secrets)` anywhere (both hand every secret onward).
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

use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use serde_yaml::Value;

/// Secret names (prefix match) that sign updates for every user. Each
/// reference must sit in the single signing step's `env`.
pub const SIGNING_SECRETS: &[&str] = &["TAURI_SIGNING_PRIVATE_KEY"];

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

/// Runner images GitHub has retired. A job on one is never picked up: it sits
/// queued until GitHub cancels it after 24h, failing the whole run.
pub const RETIRED_RUNNERS: &[&str] = &[
    "macos-11",
    "macos-12",
    "macos-13",
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
        match serde_yaml::from_str::<Value>(file.source) {
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
    } else {
        out.push(format!("{RELEASE_WORKFLOW}: missing"));
    }
    for wf in &parsed {
        out.extend(retired_runners(wf));
    }
    if let Some(nightly) = parsed.iter().find(|wf| wf.name == NIGHTLY_WORKFLOW) {
        out.extend(nightly_tier(nightly));
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

    /// A minimal release.yml with the shape the lint demands.
    const RELEASE: &str = r#"on:
  push:
    tags: ["v*"]
jobs:
  gate:
    uses: ./.github/workflows/ci.yml
  build:
    needs: gate
    runs-on: macos-14
    steps:
      - uses: actions/checkout@0123456789abcdef0123456789abcdef01234567 # v4.4.0
      - run: npm run tauri build -- --config '{"bundle":{"createUpdaterArtifacts":false}}'
  sign:
    needs: build
    runs-on: ubuntu-latest
    steps:
      - uses: actions/setup-node@0123456789abcdef0123456789abcdef01234567 # v4.4.0
      - run: npm ci --ignore-scripts
      - name: Sign
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
          - { name: macos-universal, os: macos-14 }
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
"#;

    fn run(ci: &str, release: &str) -> Vec<String> {
        run_all(ci, release, NIGHTLY)
    }

    fn run_nightly(nightly: &str) -> Vec<String> {
        run_all(CI, RELEASE, nightly)
    }

    fn run_all(ci: &str, release: &str, nightly: &str) -> Vec<String> {
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
        ])
    }

    #[test]
    fn a_retired_runner_is_flagged_in_any_workflow_and_matrix_spelling() {
        let include = NIGHTLY.replace("os: macos-14", "os: macos-13");
        assert_flags(
            &run_nightly(&include),
            "job `bundle` runs on `macos-13`, a retired GitHub runner image",
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
        assert_flags(&run(CI, &bare), "release.yml:17:");

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
        for condition in ["${{ github.event_name == 'push' }}", "success()", "true"] {
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
        let doc = |src: &str| serde_yaml::from_str::<Value>(src).unwrap_or(Value::Null);
        assert!(triggers(&doc("on: workflow_call")).contains("workflow_call"));
        assert!(triggers(&doc("on: [push, workflow_call]")).contains("workflow_call"));
        assert!(triggers(&doc("on:\n  workflow_call:\n")).contains("workflow_call"));
        assert_eq!(triggers(&doc("name: x")), BTreeSet::new());
        assert_eq!(needs(&doc("needs: a")), vec!["a"]);
        assert_eq!(needs(&doc("needs: [a, b]")), vec!["a", "b"]);
        assert_eq!(needs(&doc("runs-on: x")), Vec::<&str>::new());
    }
}
