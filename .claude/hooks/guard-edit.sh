#!/bin/bash
# PreToolUse(Edit|Write|MultiEdit|NotebookEdit) guard — protects files that
# must never be edited by hand. Exit 2 blocks the edit and explains why.
#
#   - apps/desktop/src/lib/bindings.ts is generated from the Rust command
#     surface (`just bindings`); CI's ipc-contract job fails on drift.
#   - published schema artifacts (schemas/*.v<N>.json) are versioned-immutable:
#     a change goes through the contract-change skill, whose first durable
#     step is the schemas/CHANGELOG.md entry. So an existing schema file may
#     only be edited once schemas/CHANGELOG.md carries an uncommitted change;
#     brand-new schema files (a fresh *.v2.json) are always allowed.
set -euo pipefail

path=$(jq -r '.tool_input.file_path // .tool_input.notebook_path // empty')
[ -n "$path" ] || exit 0

root="${CLAUDE_PROJECT_DIR:-$(pwd)}"
rel="${path#"$root"/}"

deny() {
  echo "Blocked by .claude/hooks/guard-edit.sh: $1" >&2
  exit 2
}

case "$rel" in
  */apps/desktop/src/lib/bindings.ts | apps/desktop/src/lib/bindings.ts)
    deny "bindings.ts is generated — change the Rust command/DTO surface and run \`just bindings\`."
    ;;
  */schemas/*.v[0-9]*.json | schemas/*.v[0-9]*.json)
    if [ -f "$path" ] && git -C "$root" diff --quiet HEAD -- schemas/CHANGELOG.md 2>/dev/null; then
      deny "published schemas are versioned-immutable. Load the contract-change skill: a breaking change mints a new *.vN+1.json; an additive one is recorded in schemas/CHANGELOG.md FIRST — this guard allows the schema edit once that entry exists."
    fi
    ;;
esac

exit 0
