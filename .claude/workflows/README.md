# Agent workflows

Orchestration scripts for Claude Code's Workflow tool, plus their helper
scripts in `.claude/scripts/`. They were written for the improvement sweep
(#110) and are kept so a later session can resume it.

## `sweep-wave.js`: implement → review → land → audit, per issue

For each issue in a wave:

1. **Implement** in an isolated worktree: code and tests, the `gates` skill's
   checks, the `visual-check` loop for UI changes, then push and open a PR.
2. **Review**: an independent, read-only skeptic checks the PR against the
   issue's acceptance criteria.
3. **Land**: address the review, loop on CI until green
   (`.claude/scripts/ci-wait.sh`), rebase onto `main` and push with a lease,
   then squash-merge.
4. **Audit** the acceptance criteria on `origin/main`. Gaps loop back to step
   1 as a new round, up to 3 rounds.

Two workers each take one issue all the way through these steps before
starting the next, so `main` keeps moving (trunk-based).

Run it from **inside the main checkout**:

```js
Workflow({
  scriptPath: ".claude/workflows/sweep-wave.js",
  args: {
    wave: "4a-core-perf",
    workers: 2,
    scratch: "<session scratchpad dir>",       // PR-body drafts, screenshots
    trailers: ["Co-Authored-By: …", "Claude-Session: …"],  // this session's attribution lines
    sessionUrl: "https://claude.ai/code/session_…",
    issues: [
      { n: 138, title: "…", note: "context the agent needs" },
      // resume an open PR: skip implement + review, go straight to land
      { n: 151, title: "…", pr: 311, branch: "feat/151-…", note: "…" },
      // re-review an open PR before landing it
      { n: 112, title: "…", pr: 217, branch: "fix/112-…", rereview: true },
      // already merged: audit only
      { n: 148, title: "…", pr: 304, merged: true }
    ]
  }
})
```

### Hard-won rules

- **Never let one issue wait on another in the same wave.** With two worker
  slots, a waiting agent holds a slot and can deadlock the wave. Put
  dependent issues in a later wave, or tell the agent to build on the other
  branch and rebase when it lands.
- **Keep the orchestrating shell's working directory inside the checkout**
  for the whole run. Worktrees are created from that directory when each
  agent spawns, so a stray `cd /tmp` makes every later agent fail with "not
  in a git repository".
- **Disk is the binding constraint in cloud sessions.** Run
  `.claude/scripts/disk-watchdog.sh` in the background during waves. Agents
  never run `just ci` or `just cov` locally (CI enforces them), and builds
  use `CARGO_INCREMENTAL=0` with one shared `CARGO_TARGET_DIR`.
- **Cloud sessions can't push tags or dispatch workflows** (HTTP 403).
  Releases go through `release-tag.yml`: merge a version-bump PR and the tag
  plus the release follow automatically.

## `.claude/scripts/`

| Script | Purpose |
|---|---|
| `ci-wait.sh <pr> [max-seconds]` | Poll a PR's check runs until they finish. Prints `ALL_GREEN`, `FAILED: <job (id)>` or `TIMEOUT`. |
| `disk-watchdog.sh [seconds]` | Prune the shared build dir in tiers as free disk shrinks. |
