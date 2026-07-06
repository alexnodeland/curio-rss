#!/usr/bin/env bash
# Frontend bans (roadmap Phase 4 security posture):
#
#   1. `{@html}` may appear in exactly one file — the sanitized-render
#      component (SanitizedHtml.svelte). Everywhere else it is a stored-XSS
#      hole waiting for a sanitizer gap.
#   2. `@tauri-apps/api/core` (hand-written invoke) may be imported only by
#      the generated bindings file — the snake_case/error-shape drift class
#      stays unrepresentable.
#
# Run from anywhere; CI job `frontend-bans` and the lefthook pre-commit
# step both call this. Mirrored as an eslint no-restricted-imports rule so
# it also fails in-editor.
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
src="$repo_root/apps/desktop/src"
status=0

html_hits="$(grep -rn '{@html' "$src" --include='*.svelte' | grep -v 'components/reader/SanitizedHtml.svelte' || true)"
if [ -n "$html_hits" ]; then
    echo "BAN: {@html} outside components/reader/SanitizedHtml.svelte:" >&2
    echo "$html_hits" >&2
    status=1
fi

invoke_hits="$(grep -rnE "from ['\"]@tauri-apps/api/core['\"]" "$src" | grep -v 'lib/bindings.ts' || true)"
if [ -n "$invoke_hits" ]; then
    echo "BAN: raw @tauri-apps/api/core import outside lib/bindings.ts:" >&2
    echo "$invoke_hits" >&2
    status=1
fi

if [ "$status" -eq 0 ]; then
    echo "frontend bans: OK — no stray {@html}, no hand-written invoke imports"
fi
exit "$status"
