export const meta = {
  name: 'curio-sweep-wave',
  description: 'Implement, review, CI-babysit, squash-merge and audit a wave of Curio sweep issues (worktree per agent)',
  whenToUse: 'Working through alexnodeland/curio-rss sweep issues end to end',
  phases: [
    { title: 'Implement', detail: 'worktree: code + tests + gates + PR' },
    { title: 'Review', detail: 'independent skeptical review of the PR' },
    { title: 'Land', detail: 'address review, green CI loop, sync with main, squash-merge' },
    { title: 'Audit', detail: 'acceptance-criteria audit on origin/main; gaps loop back' },
  ],
}

// Session-specific values come in through args (see .claude/workflows/README.md).
const REPO_DIR = args.repoDir || '/home/user/curio-rss'
const SCRATCH = args.scratch || '/tmp/curio-sweep'
const CI_WAIT = `${REPO_DIR}/.claude/scripts/ci-wait.sh`
const TRAILERS = (args.trailers || []).join('\n')
const MAX_ROUNDS = 3

const ENV = `export CARGO_TARGET_DIR=${REPO_DIR}/target CARGO_TERM_COLOR=never NO_COLOR=1 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0 CARGO_INCREMENTAL=0;`

const COMMON = `
CONTEXT: Curio — repo alexnodeland/curio-rss (public). Main checkout: ${REPO_DIR} — NEVER modify its working tree, branch or index (read-only use such as \`git -C ${REPO_DIR} fetch\` is fine). CLAUDE.md rules apply. The house procedures are the repo skills gates, ship-pr, visual-check (read them at .claude/skills/<name>/SKILL.md on origin/main; if missing there, \`git -C ${REPO_DIR} show origin/chore/claude-dx:.claude/skills/<name>/SKILL.md\`).

ENVIRONMENT (shell state does NOT persist between Bash calls — start EVERY Bash command that runs cargo, just, npm test, git commit or git push with this prefix, because the git hooks build too):
  ${ENV}
- One shared build dir serves every worktree. "Blocking waiting for file lock" = another agent is building: wait. Never delete, move or copy the build dir; never create target/ in your worktree — that includes \`npm run tauri build\`, the e2e/tauri-driver harness and any script under apps/desktop: run them with the prefix too. Before you finish, run \`du -sh target apps/desktop/src-tauri/target 2>/dev/null\` in your worktree; if either exists, delete it.
- Disk is tight (~25 GB free in total). Do NOT run \`just cov\` / cargo llvm-cov locally (CI's coverage job enforces the 85% curio-core floor — keep new core code well tested). Before you finish: kill any dev server you started, and \`rm -rf apps/desktop/node_modules apps/desktop/.svelte-kit\` in YOUR worktree if they exist.
- Rust toolchain = stable 1.99 (same as CI). Frontend: \`cd apps/desktop && npm ci\` once in your worktree before any frontend gate or visual check.
- GitHub: use the MCP tools only (load with ToolSearch, e.g. "select:mcp__github__issue_read,mcp__github__create_pull_request,mcp__github__pull_request_read,mcp__github__merge_pull_request,mcp__github__get_job_logs,mcp__github__issue_write,mcp__github__search_issues,mcp__github__add_issue_comment"). No gh CLI. \`git push\` to origin works.
- CI wait helper: \`${CI_WAIT} <pr> 540\` (run with Bash timeout 600000) polls until all checks finish (≈3–6 min) and prints ALL_GREEN or "FAILED: <job name> (job <id>)". Call it again if it prints TIMEOUT. Never loop on raw sleeps.

GIT RULES: Conventional Commits (commit-msg hook). Every commit message ends with exactly these trailer lines (the session's attribution):
${TRAILERS}
Never commit on or push to main; never --no-verify; never plain --force. Keep YOUR OWN feature branch current by rebasing onto its target and pushing with a lease: \`git fetch origin main && git rebase origin/main\` → re-run gates → \`git push --force-with-lease --force-if-includes origin <branch>\`. Never force-push any branch you didn't create. Fallback if a rebase is impractical or a lease push is refused by the permission system: \`git merge origin/main -m "chore(sync): merge origin/main into <branch>"\` (plus the two trailer lines — the commit-msg hook rejects git's default merge message) and a normal push. Let the lefthook hooks run (pre-push runs the workspace tests — that's expected). Do not edit CHANGELOG.md. Do update user-facing docs (README, docs/, site/) when behaviour changes. Never put model names anywhere except the Co-Authored-By trailer.

QUALITY BAR: implement to the issue's acceptance criteria, fully and idiomatically for this codebase. Hermetic tests only (fixtures, wiremock, 127.0.0.1). Never skip, ignore, delete or weaken tests, lints, gates or floors; no coverage exclusions; no unwrap/expect/todo!/dbg! outside tests; \`#[allow]\` only with a justifying comment. Tauri command/DTO/event changes → \`just bindings\` and commit the regenerated bindings.ts (never hand-edit it). Anything touching schemas/, frontmatter keys, event types or the manifest → load Skill "curio-rss:contract-change" first and follow it. The owner has pre-approved ADDITIVE minor contract changes (new optional fields, new event types; schemas/CHANGELOG.md entry first). Any BREAKING change: stop and return status "blocked" explaining why it can't be additive. Curio's contracts are consumed by the Curator repo (/home/user/curator, alexnodeland/curator): note any contract addition in the PR body so the consumer side can follow. i18n: every new user-facing string goes in all 8 locale catalogs.
VISUAL: any user-visible UI change follows the visual-check skill — npm run dev on port PORT, Playwright screenshots of origin/main (before) and your branch (after), light + dark + ~900px wide + every state you touched; Read every PNG and iterate until it looks deliberate, consistent with the design tokens, and native. Save PNGs under ${SCRATCH}/shots/ISSUE/ (never in the repo).
FOLLOW-UPS: anything real but out of scope → mcp search_issues first; if it's new, create an issue in alexnodeland/curio-rss with labels ["sweep","found-during-sweep","P1"|"P2"|"P3","bug"|"enhancement"|"documentation"], with sections Problem (file:line evidence), Proposal, Acceptance (checkboxes). Never fix unrelated things in your PR.
Every GitHub issue body, PR comment or issue comment you write ends with:

---
_Generated by [Claude Code](https://claude.ai/code)_
PR bodies instead end with:

🤖 Generated with [Claude Code](https://claude.com/claude-code)

${args.sessionUrl || ''}
`

const IMPL_SCHEMA = {
  type: 'object',
  properties: {
    status: { type: 'string', enum: ['pr_open', 'already_done', 'blocked'] },
    branch: { type: 'string' },
    pr: { type: 'number' },
    summary: { type: 'string' },
    closes: { type: 'boolean', description: 'true if the PR body says Closes #N (acceptance criteria fully met by this PR)' },
    remaining: { type: 'array', items: { type: 'string' } },
    followups: { type: 'array', items: { type: 'number' } },
    screenshots: { type: 'array', items: { type: 'string' } },
  },
  required: ['status', 'summary'],
}

const REVIEW_SCHEMA = {
  type: 'object',
  properties: {
    verdict: { type: 'string', enum: ['approve', 'changes_requested'] },
    findings: {
      type: 'array',
      items: {
        type: 'object',
        properties: {
          severity: { type: 'string', enum: ['blocker', 'major', 'minor', 'nit'] },
          file: { type: 'string' },
          description: { type: 'string' },
        },
        required: ['severity', 'description'],
      },
    },
  },
  required: ['verdict', 'findings'],
}

const LAND_SCHEMA = {
  type: 'object',
  properties: {
    status: { type: 'string', enum: ['merged', 'blocked'] },
    pr: { type: 'number' },
    merge_sha: { type: 'string' },
    summary: { type: 'string' },
    followups: { type: 'array', items: { type: 'number' } },
  },
  required: ['status', 'summary'],
}

const AUDIT_SCHEMA = {
  type: 'object',
  properties: {
    complete: { type: 'boolean' },
    gaps: { type: 'array', items: { type: 'string' } },
    issue_closed: { type: 'boolean' },
    followups: { type: 'array', items: { type: 'number' } },
    notes: { type: 'string' },
  },
  required: ['complete', 'gaps'],
}

function common(issue) {
  return COMMON.replaceAll('PORT', String(1420 + (issue.n % 60))).replaceAll('ISSUE', String(issue.n))
}

function implPrompt(issue, round, gaps, history) {
  const prior = round === 1 ? '' : `
This is round ${round}. Earlier rounds already merged: ${history.map(h => `PR #${h.pr}`).join(', ')}.
A post-merge audit found these gaps still blocking the acceptance criteria — deliver exactly these:
${gaps.map(g => `- ${g}`).join('\n')}
`
  return `${common(issue)}
TASK: implement GitHub issue #${issue.n} ("${issue.title}") — round ${round}. ${issue.note || ''}
${prior}
Steps:
1. Read the issue and its comments (mcp issue_read). Line numbers in it refer to v0.4.0 — locate the code on origin/main.
2. In your worktree: \`git fetch origin main && git switch -c <type>/${issue.n}-<short-slug>${round > 1 ? `-r${round}` : ''} origin/main\`.
3. If origin/main already satisfies everything asked of this round, verify it (read code, run the relevant tests) and return status "already_done" with evidence in summary.
4. Implement. Keep the PR reviewable (as a guide, ≤ ~800 changed lines excluding tests, fixtures and generated files). If the issue is genuinely bigger, deliver the most valuable coherent slice that leaves main better and fully working, and list what's left in "remaining" — later rounds pick it up.
5. Write tests that would fail without your change, one or more per acceptance criterion where it can be tested. For criteria that can only be checked by hand (real macOS, OS integration), add or extend the docs/release checklist entry and say so in the PR.
6. Run the gates for every path you touched (gates skill). If the change is broad, run every gates-skill row individually — NEVER \`just ci\` or \`just cov\` locally (both build a separate ~3 GB coverage tree; CI enforces coverage). Everything must be green.
7. Commit in small logical commits, then \`git push -u origin <branch>\` and open the PR against main with mcp create_pull_request. Mirror the headings of .github/PULL_REQUEST_TEMPLATE.md. Under Related Issues put "Closes #${issue.n}" only if this PR meets ALL its acceptance criteria; otherwise put "Part of #${issue.n}" and list what remains. Describe the screenshots in the Screenshots section.
8. Do NOT merge and do NOT wait for CI — a later stage does both. Return the structured result.`
}

function reviewPrompt(issue, impl) {
  return `${common(issue)}
TASK: independent, skeptical code review of PR #${impl.pr} (branch ${impl.branch}) for issue #${issue.n} ("${issue.title}"). You are read-only: no commits, no pushes, no GitHub comments.
Read the issue's Problem, Proposal and Acceptance. Get the change with \`git -C ${REPO_DIR} fetch origin ${impl.branch} main\`, then \`git -C ${REPO_DIR} diff origin/main...origin/${impl.branch}\`; use \`git -C ${REPO_DIR} show origin/${impl.branch}:<path>\` for whole files.
Hunt for:
- correctness bugs and regressions;
- acceptance criteria not actually met;
- tests that don't really exercise the claim (would they fail if the fix were reverted?);
- violations of the contract-first and boundary rules;
- error-handling and concurrency gaps;
- security slips (SSRF/policed client, sanitizer, secrets);
- performance traps;
- UI, a11y and i18n gaps (all 8 locales);
- missing docs.
Implementer's summary: ${impl.summary}
Report only real, evidenced problems with file:line. Severity: blocker (wrong/unsafe/data loss), major (criterion unmet, significant bug, missing test for core behaviour), minor, nit. verdict = approve iff there are no blocker or major findings.`
}

function landPrompt(issue, impl, review) {
  const findings = (review && review.findings) || []
  return `${common(issue)}
TASK: land PR #${impl.pr} (branch ${impl.branch}) for issue #${issue.n} ("${issue.title}") trunk-based.${issue.note ? "\nNOTE FROM THE ORCHESTRATOR: " + issue.note : ""}
1. In your worktree: \`git fetch origin ${impl.branch} main && git switch -C ${impl.branch} origin/${impl.branch}\`.
2. Address the independent review. Fix every blocker and major finding. Fix minors unless one is clearly wrong; if so, explain why in a PR comment (mcp add_issue_comment on the PR number). Nits are optional.
   Findings: ${JSON.stringify(findings)}
   Run the gates for what you touched, commit, push.
3. CI loop: run ${CI_WAIT} ${impl.pr} 540. On a FAILED job, read its log (mcp get_job_logs with job_id and return_content=true, tail_lines 200), reproduce it locally, fix the root cause, push, then wait again. Repeat until ALL_GREEN. Never disable, skip or weaken a check. If the failure is unrelated to this PR and is also failing on main, fix it here only if the fix is small and obvious; otherwise file an issue and report status "blocked".
4. Sync: if mergeable_state is "behind" or "dirty", or main has moved past your merge base: \`git fetch origin main && git rebase origin/main\`. Resolve conflicts commit by commit, preserving both sides' intent. Re-run the gates, then \`git push --force-with-lease --force-if-includes origin ${impl.branch}\` (use the merge fallback from GIT RULES if needed), and go back to step 3.
5. When the current head is ALL_GREEN and mergeable: call mcp merge_pull_request with merge_method "squash", commit_title "<PR title> (#${impl.pr})" and expectedHeadSha set to the head sha. If the merge is rejected because main moved, go to step 4. If merging is denied by permissions or branch protection, report status "blocked" with the exact error.
6. After merging, confirm the PR shows merged. Return the structured result.`
}

function auditPrompt(issue, history) {
  return `${common(issue)}
TASK: acceptance audit of issue #${issue.n} ("${issue.title}") after merged PR(s): ${history.map(h => '#' + h.pr).join(', ')}.
1. In your worktree: \`git fetch origin main && git switch --detach origin/main\`.
2. Read the issue: every Acceptance checkbox, plus the intent of the Proposal. Verify each one against the code on origin/main by reading the code and running the relevant tests (cargo test -p …, vitest, the CLI). For UI issues, run the visual-check loop and inspect the screenshots.
3. gaps = concrete, evidenced shortfalls: an unmet criterion, a bug introduced, a behaviour with no test. Manual-only criteria count as met when a checklist entry or doc covers them. Ignore cosmetic preferences.
4. If complete and the issue is still open: close it (mcp issue_write state "closed", state_reason "completed") with a comment that lists the evidence per criterion and the PRs. If gaps remain, leave it open and add a comment listing the gaps.
5. File unrelated problems you notice as new issues (FOLLOW-UPS rules). Do not change code.`
}

async function runIssue(issue) {
  const tag = `#${issue.n}`
  const history = []
  const followups = []
  let gaps = []
  for (let round = 1; round <= MAX_ROUNDS; round++) {
    // Resume mode: an issue arriving with an open PR whose review is already
    // addressed goes straight to landing (round 1 only).
    const resumed = round === 1 && issue.pr && issue.branch
      ? { status: 'pr_open', pr: issue.pr, branch: issue.branch, summary: issue.note || 'resumed open PR' }
      : null
    // Audit-only mode: the PR already merged; verify the acceptance criteria.
    if (round === 1 && issue.merged && issue.pr) {
      history.push({ round, pr: issue.pr, review: 'prior', findings: 0, land: 'merged' })
      const audit = await agent(auditPrompt(issue, history), {
        label: `${tag} audit r${round}`, phase: 'Audit', isolation: 'worktree', schema: AUDIT_SCHEMA,
      })
      if (!audit) return { issue: issue.n, status: 'agent_failed', stage: 'audit', round, history, followups }
      followups.push(...(audit.followups || []))
      if (audit.complete) return { issue: issue.n, status: 'done', rounds: round, history, followups, notes: audit.notes || '' }
      gaps = audit.gaps
      log(`${tag}: audit found ${gaps.length} gap(s), starting round ${round + 1}`)
      continue
    }
    const impl = resumed || await agent(implPrompt(issue, round, gaps, history), {
      label: `${tag} implement r${round}`, phase: 'Implement', isolation: 'worktree', schema: IMPL_SCHEMA,
    })
    if (!impl) return { issue: issue.n, status: 'agent_failed', stage: 'implement', round, history, followups }
    followups.push(...(impl.followups || []))
    if (impl.status === 'blocked') return { issue: issue.n, status: 'blocked', stage: 'implement', round, summary: impl.summary, history, followups }
    if (impl.status === 'pr_open') {
      if (!impl.pr || !impl.branch) return { issue: issue.n, status: 'agent_failed', stage: 'implement-no-pr', round, summary: impl.summary, history, followups }
      const review = resumed && !issue.rereview
        ? { verdict: 'approve', findings: issue.findings || [] }
        : await agent(reviewPrompt(issue, impl), {
          label: `${tag} review r${round}`, phase: 'Review', schema: REVIEW_SCHEMA,
        })
      const land = await agent(landPrompt(issue, impl, review), {
        label: `${tag} land #${impl.pr}`, phase: 'Land', isolation: 'worktree', schema: LAND_SCHEMA,
      })
      if (!land) return { issue: issue.n, status: 'agent_failed', stage: 'land', pr: impl.pr, round, history, followups }
      followups.push(...(land.followups || []))
      history.push({ round, pr: impl.pr, review: review ? review.verdict : 'none', findings: review ? review.findings.length : 0, land: land.status })
      if (land.status !== 'merged') return { issue: issue.n, status: 'blocked', stage: 'land', pr: impl.pr, summary: land.summary, history, followups }
      log(`${tag}: PR #${impl.pr} merged (round ${round})`)
    }
    const audit = await agent(auditPrompt(issue, history), {
      label: `${tag} audit r${round}`, phase: 'Audit', isolation: 'worktree', schema: AUDIT_SCHEMA,
    })
    if (!audit) return { issue: issue.n, status: 'agent_failed', stage: 'audit', round, history, followups }
    followups.push(...(audit.followups || []))
    if (audit.complete) {
      log(`${tag}: complete after ${round} round(s)`)
      return { issue: issue.n, status: 'done', rounds: round, history, followups, notes: audit.notes || '' }
    }
    gaps = audit.gaps
    log(`${tag}: audit found ${gaps.length} gap(s), starting round ${round + 1}`)
  }
  return { issue: issue.n, status: 'incomplete', gaps, history, followups }
}

const issues = args.issues
log(`wave ${args.wave}: ${issues.map(i => '#' + i.n).join(' ')}`)
// Worker pool: each worker takes one issue all the way to merged + audited
// before starting the next, so downstream stages (review/land) are never
// starved by queued implement agents and main keeps moving (trunk-based).
const WORKERS = Math.max(1, Math.min(args.workers || 2, issues.length))
const results = new Array(issues.length).fill(null)
let next = 0
async function worker() {
  while (next < issues.length) {
    const i = next++
    try {
      results[i] = await runIssue(issues[i])
    } catch (e) {
      results[i] = { issue: issues[i].n, status: 'crashed', error: String(e) }
    }
  }
}
await parallel(Array.from({ length: WORKERS }, () => () => worker()))
return { wave: args.wave, results: results.map((r, i) => r || { issue: issues[i].n, status: 'crashed' }) }
