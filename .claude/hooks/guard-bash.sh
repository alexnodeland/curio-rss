#!/bin/bash
# PreToolUse(Bash) guard — enforces the repo's git workflow rules before a
# command runs. Exit 2 blocks the call and shows the reason to Claude.
#
#   - never bypass the git hooks (--no-verify / -n on commit, --no-verify on push)
#   - never commit directly on main (trunk-based: short branch → PR → squash)
#   - never force-push without a lease, and never force-push main
#   - never push straight to main
set -euo pipefail

input=$(cat)
cmd=$(jq -r '.tool_input.command // empty' <<<"$input")
[ -n "$cmd" ] || exit 0

deny() {
  echo "Blocked by .claude/hooks/guard-bash.sh: $1" >&2
  exit 2
}

# Only git invocations are policed.
if ! grep -Eq '(^|[;&|[:space:]])git[[:space:]]' <<<"$cmd"; then
  exit 0
fi

if grep -Eq 'git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?(commit|push|merge|rebase)[^;&|]*--no-verify' <<<"$cmd" ||
  grep -Eq 'git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?commit([[:space:]]+-[a-zA-Z]*n[a-zA-Z]*)([[:space:]]|$)' <<<"$cmd"; then
  deny "git hooks must not be skipped (--no-verify). Fix what the hook reports instead; see CLAUDE.md → Front door."
fi

if grep -Eq 'git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?push[^;&|]*(--force([[:space:]]|$)|[[:space:]]-f([[:space:]]|$))' <<<"$cmd"; then
  deny "plain --force is not allowed; use --force-with-lease on your own short-lived branch."
fi

if grep -Eq 'git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?push[^;&|]*[[:space:]](origin[[:space:]]+)?(HEAD:)?(refs/heads/)?main([[:space:]]|$)' <<<"$cmd"; then
  deny "never push to main directly — open a PR (trunk-based: branch → PR → green CI → squash-merge). See the ship-pr skill."
fi

if grep -Eq 'git[[:space:]]+(-C[[:space:]]+[^[:space:]]+[[:space:]]+)?commit([[:space:]]|$)' <<<"$cmd"; then
  dir=$(jq -r '.cwd // empty' <<<"$input")
  branch=$(git -C "${dir:-.}" symbolic-ref --quiet --short HEAD 2>/dev/null || true)
  if [ "$branch" = "main" ]; then
    deny "you are on main — create a short-lived branch first (git switch -c <type>/<topic>)."
  fi
fi

exit 0
