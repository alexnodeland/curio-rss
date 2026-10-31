//! The committed `.github/workflows` satisfy the supply-chain lint, so the
//! workspace suite goes red the moment an edit un-pins an action, ungates the
//! release, or widens the updater signing key's reach (issue #174).

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::path::{Path, PathBuf};

fn workflows_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/workflows")
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
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.github/dependabot.yml");
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
