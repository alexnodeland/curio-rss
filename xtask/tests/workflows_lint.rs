//! The committed `.github/workflows` satisfy the supply-chain lint, so the
//! workspace suite goes red the moment an edit un-pins an action, ungates the
//! release, or widens the updater signing key's reach (issue #174) — or puts
//! the nightly back on a retired runner, softens its gate, or stops it from
//! running on the PRs that edit it (issue #175).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::PathBuf;

fn workflows_dir() -> PathBuf {
    xtask::paths::repo_root().unwrap().join(".github/workflows")
}

#[test]
fn committed_workflows_pass_the_supply_chain_lint() {
    let violations = xtask::workflows::lint_dir(&workflows_dir()).expect("read workflows");
    assert!(
        violations.is_empty(),
        "workflow lint violations:\n{}",
        violations.join("\n")
    );
}

/// The issue's literal acceptance check: `grep 'uses: .*@v'` over
/// `.github/workflows` finds nothing.
#[test]
fn no_workflow_uses_a_mutable_version_tag() {
    let mut hits = Vec::new();
    for entry in std::fs::read_dir(workflows_dir()).expect("list workflows") {
        let path = entry.expect("dir entry").path();
        let text = std::fs::read_to_string(&path).expect("read workflow");
        for (index, line) in text.lines().enumerate() {
            if let Some((_, after)) = line.split_once("uses: ")
                && after.split('#').next().unwrap_or_default().contains("@v")
            {
                hits.push(format!("{}:{}: {line}", path.display(), index + 1));
            }
        }
    }
    assert!(hits.is_empty(), "mutable action tags:\n{}", hits.join("\n"));
}

/// Dependabot keeps the pins fresh — one grouped config per ecosystem the
/// repo ships (actions, cargo, npm).
#[test]
fn dependabot_covers_actions_cargo_and_npm() {
    let path = xtask::paths::repo_root()
        .unwrap()
        .join(".github/dependabot.yml");
    let text = std::fs::read_to_string(&path).expect("read dependabot.yml");
    for ecosystem in ["github-actions", "cargo", "npm"] {
        assert!(
            text.contains(&format!("package-ecosystem: {ecosystem}")),
            "dependabot.yml has no `{ecosystem}` entry"
        );
    }
    assert!(
        text.contains("groups:"),
        "dependabot updates must be grouped"
    );
}

/// Lint the committed `ci.yml` beside an edited copy of the committed
/// `release.yml`.
fn lint_release_edit(from: &str, to: &str) -> Vec<String> {
    let dir = workflows_dir();
    let ci = std::fs::read_to_string(dir.join("ci.yml")).expect("read ci.yml");
    let release = std::fs::read_to_string(dir.join("release.yml")).expect("read release.yml");
    assert_eq!(
        release.matches(from).count(),
        1,
        "`{from}` must occur exactly once in release.yml"
    );
    let edited = release.replace(from, to);
    xtask::workflows::lint(&[
        xtask::workflows::WorkflowFile {
            name: "ci.yml",
            source: &ci,
        },
        xtask::workflows::WorkflowFile {
            name: "release.yml",
            source: &edited,
        },
    ])
}

/// A job that `needs` the gate but runs on `always()` would build off a red
/// tag — the regression the gate rule exists to block.
#[test]
fn an_always_build_job_in_the_real_release_is_flagged() {
    let violations = lint_release_edit(
        "    needs: gate\n    runs-on: macos",
        "    needs: gate\n    if: ${{ always() }}\n    runs-on: macos",
    );
    assert!(
        violations
            .iter()
            .any(|v| v.contains("job `build` has `if: … always()")),
        "{violations:#?}"
    );
}

/// A step ahead of the key in the real sign job that runs project code or
/// injects env must fail the lint.
#[test]
fn code_ahead_of_the_key_in_the_real_sign_job_is_flagged() {
    let anchor = "      - name: Collect the unsigned bundle\n";
    for (step, needle) in [
        ("npm run build", "runs `npm` before the signing step"),
        ("node ./dist/x.js", "runs `node` before the signing step"),
        (
            "echo NODE_OPTIONS=--require ./evil.js >> \"$GITHUB_ENV\"",
            "writes `GITHUB_ENV` at or before the signing step",
        ),
    ] {
        let violations = lint_release_edit(anchor, &format!("      - run: {step}\n{anchor}"));
        assert!(
            violations.iter().any(|v| v.contains(needle)),
            "`{step}`: {violations:#?}"
        );
    }
}

/// Dropping the real sign job's `environment:` (so the key would be read
/// outside the tag-restricted, reviewer-gated `updater-signing` environment)
/// fails the lint (issue #187).
#[test]
fn the_real_sign_job_must_keep_its_environment() {
    let line = "    environment: ${{ github.event_name == 'push' && 'updater-signing' || '' }}\n";
    assert_flagged(
        &lint_release_edit(line, ""),
        "job `sign` holds the signing key but declares no `environment:`",
    );
    // Selecting it unconditionally would hold the dispatch dry run at the
    // tag-only deployment rule.
    assert_flagged(
        &lint_release_edit(line, "    environment: updater-signing\n"),
        "always deploys to `updater-signing`",
    );
}

/// Pins the committed sign job's text: its environment expression names
/// `updater-signing` only on a tag push (empty, i.e. no environment,
/// otherwise), and the key step and the throwaway-key step split on that same
/// condition. A pin, not a proof — the lint rule itself is exercised in
/// `xtask/src/workflows.rs`, and that GitHub really runs the dispatch dry run
/// with no environment is checked by the post-merge dry run (runbook §2).
#[test]
fn the_real_dry_run_runs_outside_the_signing_environment() {
    let dir = workflows_dir();
    let release = std::fs::read_to_string(dir.join("release.yml")).expect("read release.yml");
    let doc: serde_yaml::Value = serde_yaml::from_str(&release).expect("parse release.yml");
    let sign = &doc["jobs"]["sign"];
    assert_eq!(
        sign["environment"].as_str(),
        Some("${{ github.event_name == 'push' && 'updater-signing' || '' }}")
    );
    let conditions: Vec<(&str, &str)> = sign["steps"]
        .as_sequence()
        .expect("sign steps")
        .iter()
        .filter_map(|step| Some((step["name"].as_str()?, step["if"].as_str()?)))
        .collect();
    assert!(
        conditions.contains(&("Sign the updater archive", "github.event_name == 'push'")),
        "{conditions:#?}"
    );
    assert!(
        conditions.contains(&(
            "Sign with a throwaway key (dry run)",
            "github.event_name != 'push'"
        )),
        "{conditions:#?}"
    );
}

/// Lint the committed `ci.yml` + `release.yml` beside an edited copy of the
/// committed `nightly.yml` (issue #175).
fn lint_nightly_edit(from: &str, to: &str) -> Vec<String> {
    let dir = workflows_dir();
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).expect("read workflow");
    let (ci, release, nightly) = (read("ci.yml"), read("release.yml"), read("nightly.yml"));
    assert_eq!(
        nightly.matches(from).count(),
        1,
        "`{from}` must occur exactly once in nightly.yml"
    );
    let edited = nightly.replace(from, to);
    xtask::workflows::lint(&[
        xtask::workflows::WorkflowFile {
            name: "ci.yml",
            source: &ci,
        },
        xtask::workflows::WorkflowFile {
            name: "release.yml",
            source: &release,
        },
        xtask::workflows::WorkflowFile {
            name: "nightly.yml",
            source: &edited,
        },
    ])
}

fn assert_flagged(violations: &[String], needle: &str) {
    assert!(
        violations.iter().any(|v| v.contains(needle)),
        "expected `{needle}` in {violations:#?}"
    );
}

/// The real nightly runs on a PR that edits it — dropping itself from the
/// PR path filter (so the next nightly change merges unrun) fails the lint.
#[test]
fn the_real_nightly_validates_its_own_edits() {
    assert_flagged(
        &lint_nightly_edit("      - .github/workflows/nightly.yml\n", ""),
        "does not cover `.github/workflows/nightly.yml`",
    );
}

/// The real nightly also runs on a PR that edits main-window creation — the
/// code that hands `WebView2` its automation flags on Windows (without them
/// every Windows smoke session dies).
#[test]
fn the_real_nightly_validates_main_window_edits() {
    for file in ["lib.rs", "browser_args.rs"] {
        let line = format!("      - apps/desktop/src-tauri/src/{file}\n");
        assert_flagged(
            &lint_nightly_edit(&line, ""),
            &format!("does not cover `apps/desktop/src-tauri/src/{file}`"),
        );
    }
}

/// Putting the retired Intel image back (its leg queued 24h, then was
/// cancelled, every night) fails the lint.
#[test]
fn a_retired_macos_leg_in_the_real_nightly_is_flagged() {
    let windows =
        "          - { name: windows-x64, os: windows-latest, bundle: target/release/bundle }\n";
    assert_flagged(
        &lint_nightly_edit(
            windows,
            &format!("          - {{ name: macos-x64, os: macos-13 }}\n{windows}"),
        ),
        "job `bundle` runs on `macos-13`",
    );
}

/// The real smoke is a hard gate and the real bundle upload fails on empty.
#[test]
fn softening_the_real_nightly_is_flagged() {
    assert_flagged(
        &lint_nightly_edit(
            "    name: e2e smoke (${{ matrix.name }})\n",
            "    name: e2e smoke (${{ matrix.name }})\n    continue-on-error: true\n",
        ),
        "job `smoke` sets `continue-on-error`",
    );
    assert_flagged(
        &lint_nightly_edit(
            "          if-no-files-found: error\n",
            "          if-no-files-found: warn\n",
        ),
        "uploads without `if-no-files-found: error`",
    );
}
