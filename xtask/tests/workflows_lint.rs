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
