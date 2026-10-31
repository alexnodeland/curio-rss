#!/bin/bash
# PostToolUse(Edit|Write|MultiEdit) — keep edited files formatted so the
# fmt gates (cargo fmt --check, biome) never fail on whitespace. Best-effort
# and silent: a formatter error is left for the real gate to report.
set -uo pipefail

path=$(jq -r '.tool_input.file_path // empty')
[ -n "$path" ] && [ -f "$path" ] || exit 0

case "$path" in
  *.rs)
    # Edition matches the workspace (Cargo.toml `edition = "2024"`).
    rustfmt --edition 2024 --quiet "$path" 2>/dev/null
    ;;
  */apps/desktop/*.ts | */apps/desktop/*.js | */apps/desktop/*.json | */apps/desktop/*.svelte)
    desktop="${path%%/apps/desktop/*}/apps/desktop"
    if [ -x "$desktop/node_modules/.bin/biome" ]; then
      (cd "$desktop" && ./node_modules/.bin/biome format --write "$path" >/dev/null 2>&1)
    fi
    ;;
esac

exit 0
