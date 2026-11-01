//! Rule 7 of the workflow lint — automatic release tagging (issue #298).
//!
//! `release-tag.yml` tags a merged version bump with `GITHUB_TOKEN` and
//! dispatches `release.yml` on the tag ref (a tag made with that token fires
//! no `push` event; a dispatch does). These checks keep that path as narrow
//! as the tag push it replaces: the tagger holds no secret but its token,
//! writes from a job that runs no repository code, never moves a tag, and is
//! the only workflow that creates one; and `release.yml` decides "real
//! release" by the tag ref alone, so the dispatch signs, attests and drafts
//! exactly like a tag push — behind the same `updater-signing` approval.

use std::collections::BTreeSet;

use super::{
    BUILD_WORDS, Parsed, RELEASE_TAG_WORKFLOW, RELEASE_WORKFLOW, SIGNING_ENVIRONMENT,
    SIGNING_SECRETS, TAG_REF_CONDITION, collect_strings, compact_expression, env_entries, job_if,
    jobs, needs, references_secret, shell_commands, shell_words, steps, triggers,
    without_outer_parens,
};
use crate::yaml::{self, Value};

/// Does `source` (a `release.yml`) release for real when it is dispatched on
/// a tag ref — what `release-tag.yml` sends? It must have a
/// `workflow_dispatch` trigger, and the one step holding the signing key
/// must run under exactly [`TAG_REF_CONDITION`]: a release.yml that decides
/// by the event name treats every dispatch as a dry run.
///
/// # Errors
/// Why it would not: invalid YAML, no dispatch trigger, no key step, or a key
/// step guarded by anything but the tag ref.
pub fn releases_on_tag_dispatch(source: &str) -> Result<(), String> {
    let doc = yaml::from_str(source).map_err(|err| format!("not valid YAML: {err}"))?;
    tag_ref_release(&doc)
}

fn tag_ref_release(doc: &Value) -> Result<(), String> {
    if !triggers(doc).contains("workflow_dispatch") {
        return Err(format!(
            "no `workflow_dispatch` trigger — {RELEASE_TAG_WORKFLOW} cannot dispatch the release on its tag"
        ));
    }
    let Some((job_id, index, step)) = key_step(doc) else {
        return Err("no step receives the updater signing key".to_owned());
    };
    let condition = step.get("if").and_then(Value::as_str).map(bare_condition);
    if condition.as_deref() == Some(TAG_REF_CONDITION) {
        Ok(())
    } else {
        Err(format!(
            "job `{job_id}` step {} signs with the real key on `{}`, not on the tag ref (`if: startsWith(github.ref, 'refs/tags/v')`) — a `workflow_dispatch` on the tag would sign with a throwaway key and draft nothing",
            index + 1,
            condition.as_deref().unwrap_or("every event")
        ))
    }
}

/// An `if:` condition compacted, without its `${{ }}` and outer parentheses.
pub(super) fn bare_condition(text: &str) -> String {
    let compact = compact_expression(text);
    let bare = compact
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix("}}"))
        .unwrap_or(&compact);
    without_outer_parens(bare).to_owned()
}

/// The first step whose `env` reads an updater signing secret.
fn key_step(doc: &Value) -> Option<(&str, usize, &Value)> {
    jobs(doc).into_iter().find_map(|(job_id, job)| {
        steps(job).iter().enumerate().find_map(|(index, step)| {
            env_entries(step)
                .iter()
                .filter_map(|(_, value)| value.as_str())
                .any(|text| {
                    SIGNING_SECRETS
                        .iter()
                        .any(|name| references_secret(text, name))
                })
                .then_some((job_id, index, step))
        })
    })
}

/// `release.yml` decides real-vs-dry by the ref alone.
pub(super) fn real_release_by_ref(release: &Parsed<'_>) -> Vec<String> {
    let mut out = Vec::new();
    if let Err(err) = tag_ref_release(&release.doc) {
        out.push(format!("{}: {err}", release.name));
    }
    let mut hits = Vec::new();
    collect_strings(&release.doc, &mut Vec::new(), &mut hits);
    for (path, text) in hits {
        if text.contains("github.event_name") {
            out.push(format!(
                "{}: `{}` reads `github.event_name` — decide real release vs dry run by the tag ref (`startsWith(github.ref, 'refs/tags/v')`), or a dispatch on the tag releases in some steps and dry-runs in others",
                release.name,
                path.join(".")
            ));
        }
    }
    out
}

/// Flags that make `git tag` list or verify instead of create.
const GIT_TAG_READS: &[&str] = &[
    "-l",
    "--list",
    "-v",
    "--verify",
    "--contains",
    "--no-contains",
    "--points-at",
    "--merged",
    "--no-merged",
];

/// A shell word without the quotes around it.
fn unquoted(word: &str) -> &str {
    word.trim_matches(['"', '\''])
}

/// Does this simple command create (or overwrite) a tag? `git tag <name>`,
/// a `git push` of tags (`--tags`, `--follow-tags`, `--mirror`, a
/// `refs/tags/` refspec), a `gh api` call on the git refs/tags endpoints, or
/// `gh release create` (which tags a missing ref). A plain `git push origin
/// v1.2.3` names its tag like a branch and is not caught — this is a
/// tripwire for the usual spellings, not a sandbox.
fn creates_tag(command: &[String]) -> bool {
    let word = |i: usize| command.get(i).map(String::as_str);
    match word(0) {
        Some("git") => {
            if let Some(at) = command.iter().position(|w| w == "tag") {
                let rest = &command[at + 1..];
                return !rest.is_empty()
                    && !rest.iter().any(|w| GIT_TAG_READS.contains(&w.as_str()));
            }
            command.iter().any(|w| w == "push")
                && command.iter().any(|w| {
                    matches!(w.as_str(), "--tags" | "--follow-tags" | "--mirror")
                        || w.contains("refs/tags")
                })
        }
        Some("gh") => match word(1) {
            Some("api") => command
                .iter()
                .any(|w| w.contains("git/refs") || w.contains("git/tags")),
            Some("release") => word(2) == Some("create"),
            _ => false,
        },
        _ => false,
    }
}

/// Does this simple command move or delete a ref? A `PATCH`/`DELETE` API
/// call or a `force` field, `git tag -f`/`-d`, or a forced / deleting
/// `git push` (including `+src:dst` and `:dst` refspecs).
fn moves_or_deletes_ref(command: &[String]) -> bool {
    let has = |flags: &[&str]| command.iter().any(|w| flags.contains(&unquoted(w)));
    let first = command.first().map(String::as_str);
    let api = first == Some("gh")
        && (has(&["PATCH", "DELETE"]) || command.iter().any(|w| unquoted(w).starts_with("force=")));
    let git = first == Some("git");
    let tag = git && has(&["tag"]) && has(&["-f", "--force", "-d", "--delete"]);
    let push = git
        && has(&["push"])
        && (has(&["-f", "--force", "--force-with-lease", "-d", "--delete"])
            || command
                .iter()
                .skip(2)
                .any(|w| w.starts_with(':') || w.starts_with('+')));
    api || tag || push
}

/// Only `release-tag.yml` creates tags, and it never moves or deletes one.
pub(super) fn tag_creation(parsed: &[Parsed<'_>]) -> Vec<String> {
    let mut out = Vec::new();
    for wf in parsed {
        let tagger = wf.name == RELEASE_TAG_WORKFLOW;
        for (job_id, job) in jobs(&wf.doc) {
            for (index, step) in steps(job).iter().enumerate() {
                let label = format!("{}: job `{job_id}` step {}", wf.name, index + 1);
                let uses = step.get("uses").and_then(Value::as_str).unwrap_or_default();
                if !tagger
                    && uses.starts_with("softprops/action-gh-release@")
                    && step.get("with").and_then(|w| w.get("tag_name")).is_some()
                {
                    out.push(format!(
                        "{label} sets `tag_name`, which creates a missing tag — only {RELEASE_TAG_WORKFLOW} creates tags; release the run's own tag ref"
                    ));
                }
                let run = step.get("run").and_then(Value::as_str).unwrap_or_default();
                for command in shell_commands(run) {
                    if tagger && moves_or_deletes_ref(&command) {
                        out.push(format!(
                            "{label} runs `{}`, which moves or deletes a ref — an existing tag is never touched",
                            command.join(" ")
                        ));
                    } else if !tagger && creates_tag(&command) {
                        out.push(format!(
                            "{label} runs `{}`, which creates a tag — only {RELEASE_TAG_WORKFLOW} creates tags",
                            command.join(" ")
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Does a `permissions:` value grant any write (`write-all`, or a scope set
/// to `write`)?
fn grants_write(node: Option<&Value>) -> bool {
    match node {
        Some(Value::String(all)) => all == "write-all",
        Some(Value::Mapping(scopes)) => {
            scopes.values().any(|grant| grant.as_str() == Some("write"))
        }
        _ => false,
    }
}

/// Is this simple command `gh workflow run release.yml … --ref …`?
fn dispatches_release(command: &[String]) -> bool {
    let word = |i: usize| command.get(i).map(String::as_str);
    word(0) == Some("gh")
        && word(1) == Some("workflow")
        && word(2) == Some("run")
        && command.iter().any(|w| unquoted(w) == RELEASE_WORKFLOW)
        && command.iter().any(|w| w == "--ref")
}

/// Is this simple command a `gh api` call on the create-ref endpoint?
fn creates_ref(command: &[String]) -> bool {
    command.first().is_some_and(|w| w == "gh")
        && command.get(1).is_some_and(|w| w == "api")
        && command.iter().any(|w| w.contains("/git/refs"))
}

/// The tagger's own shape: trigger, secrets, permissions, fork guard, and
/// the two calls that make it work.
pub(super) fn release_tagger(wf: &Parsed<'_>) -> Vec<String> {
    let name = wf.name;
    let mut out = Vec::new();

    let push = wf.doc.get("on").and_then(|on| on.get("push"));
    let branches: Vec<&str> = push
        .and_then(|push| push.get("branches"))
        .and_then(Value::as_sequence)
        .map(|branches| branches.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let tags = push.and_then(|push| push.get("tags")).is_some();
    if triggers(&wf.doc) != BTreeSet::from(["push".to_owned()]) || branches != ["main"] || tags {
        out.push(format!(
            "{name}: must run on `push: branches: [main]` alone — a tagger on any other event or ref could tag an unmerged commit"
        ));
    }

    let mut hits = Vec::new();
    collect_strings(&wf.doc, &mut Vec::new(), &mut hits);
    for (path, text) in hits {
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let other = compact.replace("secrets.GITHUB_TOKEN", "");
        if other.contains("secrets.") || other.contains("secrets[") {
            out.push(format!(
                "{name}: `{}` reads a secret — the tagger holds GITHUB_TOKEN alone, never the signing key",
                path.join(".")
            ));
        }
    }
    if grants_write(wf.doc.get("permissions")) {
        out.push(format!(
            "{name}: workflow-level `permissions` grant write — grant it on the tagging job alone"
        ));
    }

    let mut dispatched = false;
    let mut tagged = false;
    for (job_id, job) in jobs(&wf.doc) {
        let label = format!("{name}: job `{job_id}`");
        if job.get("environment").is_some() {
            out.push(format!(
                "{label} deploys to an environment — the tagger never reaches `{SIGNING_ENVIRONMENT}` or its secrets"
            ));
        }
        let fork_guarded =
            job_if(job).is_some_and(|condition| condition.contains("github.event.repository.fork"));
        if needs(job).is_empty() && !fork_guarded {
            out.push(format!(
                "{label} has no `if: ${{{{ !github.event.repository.fork }}}}` — a fork's `main` would tag and release"
            ));
        }
        let writes = grants_write(job.get("permissions"));
        if writes && job.get("uses").is_some() {
            out.push(format!(
                "{label} grants write to a reusable workflow — tag with `gh api` in plain steps"
            ));
        }
        for (index, step) in steps(job).iter().enumerate() {
            let run = step.get("run").and_then(Value::as_str).unwrap_or_default();
            if writes {
                if let Some(uses) = step.get("uses").and_then(Value::as_str) {
                    out.push(format!(
                        "{label} step {} uses `{uses}` beside a write token — the tagging job checks nothing out and runs no action",
                        index + 1
                    ));
                }
                let words = shell_words(run);
                for word in BUILD_WORDS {
                    if words.contains(word) {
                        out.push(format!(
                            "{label} step {} runs `{word}` beside a write token — plan in a read-only job",
                            index + 1
                        ));
                    }
                }
            }
            for command in shell_commands(run) {
                dispatched |= dispatches_release(&command);
                tagged |= creates_ref(&command);
            }
        }
    }
    if !tagged {
        out.push(format!(
            "{name}: no `gh api …/git/refs` step creates the tag — create it through the API's create-ref call, which refuses a ref that exists"
        ));
    }
    if !dispatched {
        out.push(format!(
            "{name}: no step runs `gh workflow run {RELEASE_WORKFLOW} --ref <tag>` — a tag made with GITHUB_TOKEN fires no `push` event, so nothing would release it"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(command: &str) -> Vec<String> {
        command.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn tag_creating_commands_are_recognised() {
        for command in [
            "git tag -a v1.0.0 -m x",
            "git tag v1.0.0",
            "git -C repo tag -s v1.0.0",
            "git push --tags",
            "git push origin --follow-tags",
            "git push --mirror",
            "git push origin refs/tags/v1.0.0",
            "gh api repos/o/r/git/refs -f ref=refs/tags/v1",
            "gh api \"repos/$GITHUB_REPOSITORY/git/tags\" -f tag=v1",
            "gh release create v1.0.0",
        ] {
            assert!(creates_tag(&words(command)), "{command}");
        }
        for command in [
            "git tag",
            "git tag -l",
            "git tag --list v*",
            "git tag --points-at HEAD",
            "git describe --tags",
            "git push origin main",
            "git log --oneline",
            "gh api repos/o/r/releases",
            "gh release view v1",
            "echo git tag",
        ] {
            assert!(!creates_tag(&words(command)), "{command}");
        }
    }

    #[test]
    fn ref_moves_and_deletions_are_recognised() {
        for command in [
            "gh api -X PATCH repos/o/r/git/refs/tags/v1 -f sha=x",
            "gh api --method DELETE repos/o/r/git/refs/tags/v1",
            "gh api repos/o/r/git/refs/tags/v1 -F force=true",
            "gh api repos/o/r/git/refs/tags/v1 -F \"force=true\"",
            "git tag -f v1",
            "git tag -d v1",
            "git push --force origin v1",
            "git push -f origin v1",
            "git push --force-with-lease origin v1",
            "git push origin --delete v1",
            "git push origin :refs/tags/v1",
            "git push origin +refs/tags/v1",
        ] {
            assert!(moves_or_deletes_ref(&words(command)), "{command}");
        }
        for command in [
            "gh api repos/o/r/git/refs -f ref=refs/tags/v1 -f sha=x",
            "git push origin refs/tags/v1",
            "git tag -a v1 -m x",
            "echo PATCH",
        ] {
            assert!(!moves_or_deletes_ref(&words(command)), "{command}");
        }
    }

    #[test]
    fn write_grants_are_recognised_in_both_spellings() {
        let doc = |src: &str| yaml::from_str(src).unwrap_or(Value::Null);
        assert!(grants_write(
            doc("permissions: write-all").get("permissions")
        ));
        assert!(grants_write(
            doc("permissions:\n  contents: read\n  actions: write\n").get("permissions")
        ));
        assert!(!grants_write(
            doc("permissions:\n  contents: read\n").get("permissions")
        ));
        assert!(!grants_write(
            doc("permissions: read-all").get("permissions")
        ));
        assert!(!grants_write(None));
    }

    #[test]
    fn a_dispatch_releases_only_under_the_tag_ref() {
        let release = |key_if: &str| {
            format!(
                "on:\n  push:\n    tags: [\"v*\"]\n  workflow_dispatch:\njobs:\n  sign:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Sign\n        if: {key_if}\n        env:\n          K: ${{{{ secrets.TAURI_SIGNING_PRIVATE_KEY }}}}\n        run: tauri signer sign x\n"
            )
        };
        assert_eq!(
            releases_on_tag_dispatch(&release("startsWith(github.ref, 'refs/tags/v')")),
            Ok(())
        );
        assert_eq!(
            releases_on_tag_dispatch(&release("${{ (startsWith(github.ref,'refs/tags/v')) }}")),
            Ok(())
        );
        // The v0.5.0 shape: every dispatch is a dry run.
        let by_event = releases_on_tag_dispatch(&release("github.event_name == 'push'"));
        assert!(
            by_event.as_ref().is_err_and(
                |err| err.contains("signs with the real key on `github.event_name=='push'`")
            ),
            "{by_event:?}"
        );
        let no_dispatch =
            release("startsWith(github.ref, 'refs/tags/v')").replace("  workflow_dispatch:\n", "");
        assert!(
            releases_on_tag_dispatch(&no_dispatch)
                .is_err_and(|err| err.contains("no `workflow_dispatch` trigger"))
        );
        let keyless = release("startsWith(github.ref, 'refs/tags/v')")
            .replace("secrets.TAURI_SIGNING_PRIVATE_KEY", "secrets.OTHER");
        assert!(
            releases_on_tag_dispatch(&keyless)
                .is_err_and(|err| err.contains("no step receives the updater signing key"))
        );
        assert!(
            releases_on_tag_dispatch("jobs: [unclosed")
                .is_err_and(|err| err.starts_with("not valid YAML"))
        );
    }
}
