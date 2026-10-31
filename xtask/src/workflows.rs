//! Supply-chain lint for `.github/workflows` (`cargo run -p xtask -- workflows`).
//!
//! Holds the release pipeline to the shape issue #174 asked for, so a later
//! edit cannot quietly undo it:
//!
//! 1. **Pinned actions.** Every `uses:` is a local `./` path, a
//!    `docker://…@sha256:` digest, or `owner/repo@<40-hex commit SHA>` with a
//!    trailing `# vX.Y.Z` comment (what dependabot bumps).
//! 2. **Release gate.** `ci.yml` is callable (`workflow_call`), `release.yml`
//!    calls it, and every other release job transitively `needs` that call —
//!    so a tag on a commit with red CI never reaches a draft release.
//! 3. **Signing-key isolation.** The updater signing secrets are referenced
//!    from exactly one step's `env`, across all workflows. That step runs a
//!    `signer sign` command and no build tool; its job installs npm packages
//!    only with `--ignore-scripts`, compiles nothing, and uses only the
//!    first-party artifact plumbing actions. No `secrets: inherit` or
//!    `toJSON(secrets)` anywhere (both hand every secret onward).
//! 4. **Release assets.** The draft release step ships `SHA256SUMS`, a
//!    `CycloneDX` SBOM and the provenance attestation bundle, and its job mints
//!    that attestation first.

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

    for (job_id, _) in &all_jobs {
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
    }
    out
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
        for command in script.lines() {
            let command_words = shell_words(command);
            let installs = command_words
                .windows(2)
                .any(|pair| pair[0] == "npm" && matches!(pair[1], "ci" | "install" | "i"));
            if installs && !command_words.contains(&"--ignore-scripts") {
                out.push(format!(
                    "{other_label} installs npm packages without `--ignore-scripts` in the signing job"
                ));
            }
        }
    }
    out
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

    fn run(ci: &str, release: &str) -> Vec<String> {
        lint(&[
            WorkflowFile {
                name: "ci.yml",
                source: ci,
            },
            WorkflowFile {
                name: "release.yml",
                source: release,
            },
        ])
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
