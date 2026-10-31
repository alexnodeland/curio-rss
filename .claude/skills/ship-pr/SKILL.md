---
name: ship-pr
description: Land a change trunk-based — short-lived branch off main, small green commits, PR from the template, wait for CI and fix in a loop, sync with main, squash-merge. Use whenever work is ready to leave the local machine, or when asked to open, babysit, or merge a PR.
---

# Ship a PR (trunk-based)

`main` is always releasable. Every change reaches it through a short-lived
branch and a squash-merged PR whose CI is green **on the commit that merges**.

## 1. Branch

```sh
git fetch origin main
git switch -c <type>/<issue>-<topic> origin/main     # e.g. fix/111-keep-hydrated-text
```

One issue (or one coherent slice of a big issue) per branch. If an issue is
too big for one reviewable PR (~600 changed lines is a smell), land it as a
sequence of small PRs, each green and merged before the next starts.

## 2. Commit

- Conventional Commits (`type(scope): subject`), enforced by the commit-msg
  hook. Scopes follow history: `core`, `fetch`, `ingest`, `storage`,
  `export`, `events`, `cli`, `desktop`, `ui`, `reader`, `sidebar`, `ci`, …
- The lefthook pre-commit runs fmt + clippy (+ frontend lint for
  `apps/desktop/**`); pre-push runs tests. Never skip them.
- Don't touch `CHANGELOG.md` in feature PRs — the release skill writes the
  section from the merged history. Do update user-facing docs
  (README, `docs/`, `site/`) when behaviour changes.

## 3. Gate locally

Run the `gates` skill's rows for every path you touched; run `just ci` before
the final push when the change is non-trivial.

## 4. Open the PR

Push (`git push -u origin <branch>`), then open the PR against `main`
mirroring `.github/PULL_REQUEST_TEMPLATE.md`'s headings (Summary, Type of
Change, Related Issues, Changes Made, Testing, Checklist, Screenshots).
`Closes #N` only when the PR completes the issue's acceptance criteria;
otherwise `Part of #N` and list what remains.

## 5. CI loop

Poll the PR's check runs (≈3 min per run). On red: read the failing job's
log, reproduce locally, fix the cause, push a new commit (no force-push
needed). Repeat until every required check is green. Never re-run CI blindly,
never push empty commits, never disable a check.

A PR that touches the nightly's plumbing (the `pull_request.paths` list in
`.github/workflows/nightly.yml`) also runs the nightly — the 3-OS bundles and
the Linux + Windows e2e smoke. Those checks are **not** required by branch
protection (a path-filtered workflow can't be), so the merge button won't
wait for them: you must. Don't merge until every nightly leg on the head
commit is green too.

## 6. Merge

1. If `main` moved: `git fetch origin main && git merge origin/main -m
   "chore(sync): merge origin/main into <branch>"` (the commit-msg hook
   rejects git's default "Merge remote-tracking branch…" message; the
   squash-merge flattens it, so `main` stays linear and no force-push is
   needed), resolve conflicts, re-run the gates for the touched paths, push,
   and wait for CI to go green again on the updated head.
2. Squash-merge with the PR title as the commit title (history reads as one
   conventional commit per PR, `(#NNN)` suffix).
3. Confirm the linked issue closed; delete the branch.

## 7. Follow-ups

Anything noticed but out of scope becomes a new issue (search first to avoid
duplicates) — never a drive-by change in the current PR.
