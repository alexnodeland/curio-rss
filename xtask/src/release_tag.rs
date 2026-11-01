//! `cargo run -p xtask -- release-tag` — decide what `release-tag.yml` tags
//! on a push to `main` (issue #298).
//!
//! Merging the version-bump PR is the release decision, so the tag goes on
//! the commit that introduced the workspace version (the bump PR's
//! squash-merge), not on the tip of `main`. The plan is:
//!
//! - **Nothing** when tag `v<version>` already exists — the usual push, and
//!   every re-run: an existing tag is never moved.
//! - **Tag** `v<version>` on the oldest commit of the newest unbroken run of
//!   first-parent commits whose `Cargo.toml` carries that version (a later
//!   commit that edits `Cargo.toml` for another reason keeps the bump
//!   commit), with the version's CHANGELOG heading as the tag message.
//! - **Refuse** (an error) when the CHANGELOG at that commit does not lead
//!   with the version's section (release.yml would fail on it), or when that
//!   commit's own release.yml would not release a dispatch on the tag ref for
//!   real ([`crate::workflows::releases_on_tag_dispatch`]): GitHub runs the
//!   workflow file *at the tag*, and an older release.yml that decides by the
//!   event name would dry-run the dispatch — while the tag, once made, would
//!   keep a hand-pushed tag from ever triggering the real release.
//!
//! The decision is pure ([`plan`] over a [`Repo`]); [`Git`] is the `git`
//! CLI behind it.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The workspace manifest whose `[workspace.package] version` is released.
const MANIFEST: &str = "Cargo.toml";
/// The release notes; release.yml ships its first `## ` section.
const CHANGELOG: &str = "CHANGELOG.md";
/// The release workflow, read at the commit that would be tagged.
const RELEASE_WORKFLOW: &str = ".github/workflows/release.yml";

/// What `release-tag.yml` should do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// `tag` exists already: nothing to do.
    Tagged {
        /// The existing tag, e.g. `v0.5.0`.
        tag: String,
    },
    /// Create the annotated `tag` on `sha` with `message`, then release it.
    Tag {
        /// The tag to create, e.g. `v0.5.0`.
        tag: String,
        /// The full SHA of the commit that introduced the version.
        sha: String,
        /// The tag message: the version's CHANGELOG heading.
        message: String,
    },
}

impl Plan {
    /// The plan as `key=value` lines for `$GITHUB_OUTPUT` — empty when
    /// there is nothing to tag.
    #[must_use]
    pub fn outputs(&self) -> String {
        match self {
            Self::Tagged { .. } => String::new(),
            Self::Tag { tag, sha, message } => {
                format!("tag={tag}\nsha={sha}\nmessage={message}\n")
            }
        }
    }
}

/// The repository reads the plan needs.
pub trait Repo {
    /// The text of `path` at `rev`, `None` when the path is absent there.
    ///
    /// # Errors
    /// The repository could not be read.
    fn show(&self, rev: &str, path: &str) -> Result<Option<String>, String>;

    /// Does tag `tag` exist?
    ///
    /// # Errors
    /// The repository could not be read.
    fn tag_exists(&self, tag: &str) -> Result<bool, String>;

    /// Full SHAs of the first-parent commits from `HEAD` back that change
    /// `path`, newest first.
    ///
    /// # Errors
    /// The repository could not be read.
    fn first_parent_commits(&self, path: &str) -> Result<Vec<String>, String>;
}

/// The `version` of a manifest's `[workspace.package]` table.
#[must_use]
pub fn workspace_version(manifest: &str) -> Option<&str> {
    let mut in_table = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_table = line == "[workspace.package]";
            continue;
        }
        if !in_table {
            continue;
        }
        let Some(value) = line
            .strip_prefix("version")
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix('='))
        else {
            continue;
        };
        let value = value.trim();
        return value
            .strip_prefix('"')
            .and_then(|rest| rest.split_once('"'))
            .map(|(version, _)| version);
    }
    None
}

/// The tag message from a CHANGELOG whose first `## ` heading is
/// `## [<version>]…`: `Curio <version>` plus the rest of that heading
/// (`## [0.5.0] — 2026-11-01` → `Curio 0.5.0 — 2026-11-01`).
///
/// # Errors
/// The CHANGELOG has no `## ` section, or its first one is another version.
pub fn tag_message(changelog: &str, version: &str) -> Result<String, String> {
    let Some(heading) = changelog.lines().find(|line| line.starts_with("## ")) else {
        return Err(format!(
            "{CHANGELOG} has no `## [{version}]` section — add the release notes before tagging"
        ));
    };
    let marker = format!("## [{version}]");
    let Some(rest) = heading.strip_prefix(&marker) else {
        return Err(format!(
            "{CHANGELOG}'s first section is `{heading}`, not `{marker}` — release.yml ships the first section as the notes, so add `{marker}` on top"
        ));
    };
    Ok(format!("Curio {version}{}", rest.trim_end()))
}

/// Decide what to tag.
///
/// # Errors
/// The repository could not be read, the workspace version is missing, or
/// the version must not be tagged automatically (see the module docs).
pub fn plan(repo: &dyn Repo) -> Result<Plan, String> {
    let manifest = repo
        .show("HEAD", MANIFEST)?
        .ok_or_else(|| format!("no {MANIFEST} at HEAD"))?;
    let version = workspace_version(&manifest)
        .ok_or_else(|| format!("{MANIFEST} has no `[workspace.package] version`"))?
        .to_owned();
    let tag = format!("v{version}");
    if repo.tag_exists(&tag)? {
        return Ok(Plan::Tagged { tag });
    }

    let mut bump = None;
    for sha in repo.first_parent_commits(MANIFEST)? {
        let at = repo.show(&sha, MANIFEST)?;
        if at.as_deref().and_then(workspace_version) != Some(version.as_str()) {
            break;
        }
        bump = Some(sha);
    }
    let sha = bump.ok_or_else(|| {
        format!("no first-parent commit of HEAD sets the workspace version to {version}")
    })?;

    let changelog = repo.show(&sha, CHANGELOG)?.unwrap_or_default();
    let message = tag_message(&changelog, &version).map_err(|err| {
        format!(
            "{sha}: {err}. The tag goes on the commit that introduced {version}, so a later commit that only fixes {CHANGELOG} leaves this refusal in place: either bump to a new version with its section on top, or push {tag} by hand on the commit that fixes {CHANGELOG} (docs/release/runbook.md §2, Fallback)"
        )
    })?;

    let release = repo.show(&sha, RELEASE_WORKFLOW)?.unwrap_or_default();
    if let Err(err) = crate::workflows::releases_on_tag_dispatch(&release) {
        return Err(format!(
            "{tag} would go on {sha}, but the release.yml there would not release a dispatch on the tag: {err}. GitHub runs the workflow file at the tag, so tagging it from here would only dry-run, and the tag would then keep a hand-pushed one from triggering the real release. Push this tag by hand instead (docs/release/runbook.md §2): git tag -a {tag} {sha} -m {} && git push origin {tag}",
            shell_quote(&message)
        ));
    }
    Ok(Plan::Tag { tag, sha, message })
}

/// `text` as one POSIX shell word: single-quoted, each `'` spelled `'\''`,
/// so a printed command pastes as-is whatever the CHANGELOG heading holds
/// (`"`, `$`, a backtick).
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The `git` CLI on a checkout.
#[derive(Debug, Clone)]
pub struct Git {
    root: PathBuf,
}

impl Git {
    /// A repository rooted at (or containing) `root`.
    #[must_use]
    pub fn new(root: &Path) -> Self {
        Self {
            root: root.to_path_buf(),
        }
    }

    fn run(&self, args: &[&str]) -> Result<std::process::Output, String> {
        git_at(&self.root)
            .args(args)
            .output()
            .map_err(|err| format!("cannot run git {}: {err}", args.join(" ")))
    }
}

/// Git's repository-locating environment (`git rev-parse --local-env-vars`).
/// Git exports `GIT_DIR` & co. to the hooks it runs, so a `git -C <root>`
/// started from inside a hook (a pre-push test run) would otherwise read —
/// and write — the hook's repository instead of `root`.
const GIT_LOCAL_ENV: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// `git -C <root>`, with the repository found from `root` alone.
fn git_at(root: &Path) -> Command {
    let mut command = Command::new("git");
    for key in GIT_LOCAL_ENV {
        command.env_remove(key);
    }
    command.arg("-C").arg(root);
    command
}

impl Repo for Git {
    fn show(&self, rev: &str, path: &str) -> Result<Option<String>, String> {
        let spec = format!("{rev}:{path}");
        // Ask first, so a path missing at `rev` is `None` and any other
        // failure (not a repository, an unknown rev) is an error.
        let exists = self.run(&["cat-file", "-e", &spec])?;
        if !exists.status.success() {
            let commit = self.run(&["cat-file", "-e", &format!("{rev}^{{commit}}")])?;
            return if commit.status.success() {
                Ok(None)
            } else {
                Err(format!(
                    "git: no commit `{rev}`: {}",
                    String::from_utf8_lossy(&commit.stderr).trim()
                ))
            };
        }
        let output = self.run(&["show", &spec])?;
        if output.status.success() {
            String::from_utf8(output.stdout)
                .map(Some)
                .map_err(|err| format!("{spec} is not UTF-8: {err}"))
        } else {
            Err(format!(
                "git show {spec}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ))
        }
    }

    fn tag_exists(&self, tag: &str) -> Result<bool, String> {
        let output = self.run(&["rev-parse", "-q", "--verify", &format!("refs/tags/{tag}")])?;
        Ok(output.status.success())
    }

    fn first_parent_commits(&self, path: &str) -> Result<Vec<String>, String> {
        let output = self.run(&["log", "--first-parent", "--format=%H", "HEAD", "--", path])?;
        if !output.status.success() {
            return Err(format!(
                "git log -- {path}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::to_owned)
            .collect())
    }
}

#[cfg(test)]
mod tests {
    // Tests drive real git and assert on the result; a panic is the failure.
    #![allow(clippy::unwrap_used)]

    use super::*;
    use std::collections::{BTreeMap, BTreeSet};

    /// A release.yml that releases a dispatch on the tag for real.
    const RELEASE_BY_REF: &str = "on:\n  push:\n    tags: [\"v*\"]\n  workflow_dispatch:\njobs:\n  sign:\n    runs-on: ubuntu-latest\n    steps:\n      - if: startsWith(github.ref, 'refs/tags/v')\n        env:\n          K: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}\n        run: tauri signer sign x\n";

    /// An in-memory history: newest-first first-parent commits touching the
    /// manifest, each with its files.
    #[derive(Default)]
    struct Fake {
        tags: BTreeSet<String>,
        commits: Vec<(String, BTreeMap<String, String>)>,
    }

    impl Fake {
        /// Push an older commit (call newest first).
        fn commit(mut self, sha: &str, version: &str, changelog: &str, release: &str) -> Self {
            let files = BTreeMap::from([
                (
                    MANIFEST.to_owned(),
                    format!(
                        "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{version}\"\n"
                    ),
                ),
                (CHANGELOG.to_owned(), changelog.to_owned()),
                (RELEASE_WORKFLOW.to_owned(), release.to_owned()),
            ]);
            self.commits.push((sha.to_owned(), files));
            self
        }

        fn tagged(mut self, tag: &str) -> Self {
            self.tags.insert(tag.to_owned());
            self
        }
    }

    impl Repo for Fake {
        fn show(&self, rev: &str, path: &str) -> Result<Option<String>, String> {
            let rev = if rev == "HEAD" {
                self.commits.first().map(|(sha, _)| sha.as_str())
            } else {
                Some(rev)
            };
            let files = self
                .commits
                .iter()
                .find(|(sha, _)| Some(sha.as_str()) == rev)
                .map(|(_, files)| files)
                .ok_or_else(|| format!("no commit {rev:?}"))?;
            Ok(files.get(path).cloned())
        }

        fn tag_exists(&self, tag: &str) -> Result<bool, String> {
            Ok(self.tags.contains(tag))
        }

        fn first_parent_commits(&self, _path: &str) -> Result<Vec<String>, String> {
            Ok(self.commits.iter().map(|(sha, _)| sha.clone()).collect())
        }
    }

    const NOTES_050: &str =
        "# Changelog\n\n## [0.5.0] — 2026-11-01\n\nStuff.\n\n## [0.4.0] — 2026-07-13\n";
    const NOTES_040: &str = "# Changelog\n\n## [0.4.0] — 2026-07-13\n";

    #[test]
    fn a_merged_bump_tags_the_commit_that_introduced_the_version() {
        // A later commit edits Cargo.toml (a dependency) without changing the
        // version: the tag still goes on the bump.
        let repo = Fake::default()
            .commit("ccc", "0.5.0", NOTES_050, RELEASE_BY_REF)
            .commit("bbb", "0.5.0", NOTES_050, RELEASE_BY_REF)
            .commit("aaa", "0.4.0", NOTES_040, RELEASE_BY_REF)
            .tagged("v0.4.0");
        let planned = plan(&repo);
        assert_eq!(
            planned,
            Ok(Plan::Tag {
                tag: "v0.5.0".to_owned(),
                sha: "bbb".to_owned(),
                message: "Curio 0.5.0 — 2026-11-01".to_owned(),
            })
        );
        assert_eq!(
            planned.map(|plan| plan.outputs()),
            Ok("tag=v0.5.0\nsha=bbb\nmessage=Curio 0.5.0 — 2026-11-01\n".to_owned())
        );
    }

    #[test]
    fn an_existing_tag_is_a_no_op() {
        let repo = Fake::default()
            .commit("bbb", "0.5.0", NOTES_050, RELEASE_BY_REF)
            .commit("aaa", "0.4.0", NOTES_040, RELEASE_BY_REF)
            .tagged("v0.5.0");
        let planned = plan(&repo);
        assert_eq!(
            planned,
            Ok(Plan::Tagged {
                tag: "v0.5.0".to_owned()
            })
        );
        assert_eq!(planned.map(|plan| plan.outputs()), Ok(String::new()));
    }

    #[test]
    fn a_version_reverted_and_bumped_again_tags_the_latest_bump() {
        let repo = Fake::default()
            .commit("ddd", "0.5.0", NOTES_050, RELEASE_BY_REF)
            .commit("ccc", "0.4.0", NOTES_040, RELEASE_BY_REF)
            .commit("bbb", "0.5.0", NOTES_050, RELEASE_BY_REF)
            .commit("aaa", "0.4.0", NOTES_040, RELEASE_BY_REF);
        assert!(matches!(plan(&repo), Ok(Plan::Tag { sha, .. }) if sha == "ddd"));
    }

    #[test]
    fn a_bump_without_its_changelog_section_is_refused() {
        let repo = Fake::default()
            .commit("bbb", "0.5.0", NOTES_040, RELEASE_BY_REF)
            .commit("aaa", "0.4.0", NOTES_040, RELEASE_BY_REF);
        let err = plan(&repo).unwrap_err();
        assert!(
            err.starts_with(
                "bbb: CHANGELOG.md's first section is `## [0.4.0] — 2026-07-13`, not `## [0.5.0]`"
            ),
            "{err}"
        );
        // A CHANGELOG-only fix keeps the version, so the tag stays on `bbb`:
        // the error names the two ways out that work.
        assert!(
            err.contains("either bump to a new version with its section on top, or push v0.5.0 by hand on the commit that fixes CHANGELOG.md"),
            "{err}"
        );
        let fixed_later = Fake::default()
            .commit("ccc", "0.5.0", NOTES_050, RELEASE_BY_REF)
            .commit("bbb", "0.5.0", NOTES_040, RELEASE_BY_REF)
            .commit("aaa", "0.4.0", NOTES_040, RELEASE_BY_REF);
        assert!(
            plan(&fixed_later)
                .unwrap_err()
                .starts_with("bbb: CHANGELOG.md's first section is `## [0.4.0]")
        );
        let empty = Fake::default().commit("bbb", "0.5.0", "# Changelog\n", RELEASE_BY_REF);
        assert!(
            plan(&empty)
                .unwrap_err()
                .contains("has no `## [0.5.0]` section")
        );
    }

    /// The v0.5.0 case: its bump commit carries a release.yml that decides
    /// by the event name, so a dispatch on the tag would only dry-run.
    #[test]
    fn a_bump_whose_release_yml_dry_runs_a_dispatch_is_refused() {
        let by_event = RELEASE_BY_REF.replace(
            "startsWith(github.ref, 'refs/tags/v')",
            "github.event_name == 'push'",
        );
        let repo = Fake::default()
            .commit("67cc218", "0.5.0", NOTES_050, &by_event)
            .commit("8dd5292", "0.4.0", NOTES_040, &by_event)
            .tagged("v0.4.0");
        let err = plan(&repo).unwrap_err();
        assert!(err.starts_with("v0.5.0 would go on 67cc218, but the release.yml there would not release a dispatch on the tag"), "{err}");
        assert!(
            err.contains(
                "git tag -a v0.5.0 67cc218 -m 'Curio 0.5.0 — 2026-11-01' && git push origin v0.5.0"
            ),
            "{err}"
        );
    }

    /// The printed fallback pastes as-is: a heading holding quotes, `$` or a
    /// backtick stays one literal shell word.
    #[test]
    fn the_fallback_command_quotes_the_tag_message() {
        assert_eq!(shell_quote("Curio 0.5.0"), "'Curio 0.5.0'");
        assert_eq!(
            shell_quote("Curio 0.5.0 — \"$HOME\" `id` it's"),
            r#"'Curio 0.5.0 — "$HOME" `id` it'\''s'"#
        );
        let by_event = RELEASE_BY_REF.replace(
            "startsWith(github.ref, 'refs/tags/v')",
            "github.event_name == 'push'",
        );
        let notes = "# Changelog\n\n## [0.5.0] — the \"$x\" release\n";
        let repo = Fake::default().commit("67cc218", "0.5.0", notes, &by_event);
        let err = plan(&repo).unwrap_err();
        assert!(
            err.contains(r#"-m 'Curio 0.5.0 — the "$x" release' && git push origin v0.5.0"#),
            "{err}"
        );
    }

    #[test]
    fn a_manifest_without_a_workspace_version_is_an_error() {
        assert_eq!(workspace_version("[package]\nversion = \"1.0.0\"\n"), None);
        assert_eq!(
            workspace_version(
                "[workspace.package]\nedition = \"2024\"\nversion=\"1.2.3\" # x\n[dependencies]\nversion = \"9\"\n"
            ),
            Some("1.2.3")
        );
        assert_eq!(
            workspace_version("[workspace.package]\nversion.workspace = true\n"),
            None
        );
        let mut repo = Fake::default().commit("aaa", "0.5.0", NOTES_050, RELEASE_BY_REF);
        if let Some((_, files)) = repo.commits.first_mut() {
            files.insert(MANIFEST.to_owned(), "[workspace]\n".to_owned());
        }
        assert_eq!(
            plan(&repo),
            Err("Cargo.toml has no `[workspace.package] version`".to_owned())
        );
    }

    /// A scratch git repository under the system temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("xtask-release-tag-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join(".github/workflows")).unwrap();
            let scratch = Self(dir);
            scratch.git(&["init", "-q", "-b", "main"]);
            // Every later call writes: make sure it is this repository.
            let top = scratch.git(&["rev-parse", "--absolute-git-dir"]);
            assert_eq!(
                std::fs::canonicalize(&top).unwrap(),
                std::fs::canonicalize(scratch.0.join(".git")).unwrap(),
                "the scratch git commands resolve to another repository"
            );
            scratch
        }

        fn git(&self, args: &[&str]) -> String {
            let output = git_at(&self.0)
                // Never fall through to a repository above the temp dir.
                .env("GIT_CEILING_DIRECTORIES", std::env::temp_dir())
                .args([
                    "-c",
                    "user.name=t",
                    "-c",
                    "user.email=t@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "tag.gpgsign=false",
                    // No hook of the developer's (a global commit-msg
                    // checker) may judge the scratch commits.
                    "-c",
                    "core.hooksPath=/dev/null",
                ])
                .args(args)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        }

        fn commit(&self, files: &[(&str, &str)], message: &str) -> String {
            for (path, text) in files {
                std::fs::write(self.0.join(path), text).unwrap();
            }
            self.git(&["add", "-A"]);
            self.git(&["commit", "-q", "-m", message]);
            self.git(&["rev-parse", "HEAD"])
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The real `git` plumbing, end to end on a scratch repository: the bump
    /// commit is found past a later manifest edit and a later unrelated
    /// commit, and a tag makes the next plan a no-op.
    #[test]
    fn git_plans_against_a_real_history() {
        let repo = Scratch::new("history");
        let manifest = |version: &str, extra: &str| {
            format!("[workspace.package]\nversion = \"{version}\"\n{extra}")
        };
        let release = ".github/workflows/release.yml";
        repo.commit(
            &[
                ("Cargo.toml", &manifest("0.4.0", "")),
                ("CHANGELOG.md", NOTES_040),
                (release, RELEASE_BY_REF),
            ],
            "chore(release): bump version to 0.4.0",
        );
        repo.git(&["tag", "-a", "v0.4.0", "-m", "Curio 0.4.0"]);
        let bump = repo.commit(
            &[
                ("Cargo.toml", &manifest("0.5.0", "")),
                ("CHANGELOG.md", NOTES_050),
            ],
            "chore(release): bump version to 0.5.0",
        );
        repo.commit(
            &[(
                "Cargo.toml",
                &manifest("0.5.0", "rust-version = \"1.88\"\n"),
            )],
            "build: declare the msrv",
        );
        repo.commit(&[("README.md", "hi\n")], "docs: readme");

        let git = Git::new(&repo.0);
        assert_eq!(git.show("HEAD", "missing.txt"), Ok(None));
        assert!(git.show("nope", "Cargo.toml").is_err());
        assert_eq!(
            plan(&git),
            Ok(Plan::Tag {
                tag: "v0.5.0".to_owned(),
                sha: bump.clone(),
                message: "Curio 0.5.0 — 2026-11-01".to_owned(),
            })
        );

        repo.git(&["tag", "-a", "v0.5.0", &bump, "-m", "Curio 0.5.0"]);
        assert_eq!(
            plan(&git),
            Ok(Plan::Tagged {
                tag: "v0.5.0".to_owned()
            })
        );
    }

    /// Inside a git hook (the pre-push test run) `GIT_DIR` & co. point at
    /// the hook's repository; `git_at` must drop them so `-C` decides.
    #[test]
    fn git_ignores_the_repository_a_hook_exports() {
        let command = git_at(Path::new("/nowhere"));
        let removed: Vec<_> = command
            .get_envs()
            .filter(|(_, value)| value.is_none())
            .filter_map(|(key, _)| key.to_str())
            .collect();
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_COMMON_DIR",
        ] {
            assert!(removed.contains(&key), "{key} kept: {removed:?}");
        }
        let args: Vec<_> = command.get_args().filter_map(|a| a.to_str()).collect();
        assert_eq!(args, ["-C", "/nowhere"]);
    }

    /// A repository with no commit yet: every read fails rather than
    /// planning from nothing.
    #[test]
    fn git_errors_on_an_empty_history() {
        let repo = Scratch::new("empty");
        let git = Git::new(&repo.0);
        assert!(git.show("HEAD", "Cargo.toml").is_err());
        assert!(git.first_parent_commits("Cargo.toml").is_err());
        assert!(plan(&git).is_err());
        assert_eq!(git.tag_exists("v0.1.0"), Ok(false));
    }
}
