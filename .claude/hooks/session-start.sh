#!/bin/bash
# SessionStart hook — provisions a Claude Code cloud session so every gate in
# `just ci` runs out of the box: the Linux webview packages the curio-desktop
# workspace member links against, the Rust components, the cargo tools the
# justfile calls, lefthook, the desktop frontend's npm deps, and the git hooks.
#
# Runs only in remote (web) sessions; a local checkout is set up with
# `just setup` as CONTRIBUTING.md describes. Idempotent: every step checks
# before it installs, and the npm install is skipped unless its manifests (or
# this script) changed since the last successful run. The container snapshot
# is cached after the hook finishes, so the slow first-run installs (apt,
# `cargo install`) are paid once per environment, not per session.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

cd "${CLAUDE_PROJECT_DIR:-.}"

log() { echo "[session-start] $*" >&2; }

# Run privileged steps directly as root, through sudo otherwise.
as_root() {
  if [ "$(id -u)" -eq 0 ]; then
    "$@"
  else
    sudo -n "$@"
  fi
}

# 1. Webview system packages (ci.yml installs the same set). Without them the
#    `--workspace` gates cannot compile apps/desktop/src-tauri.
if ! pkg-config --exists webkit2gtk-4.1 2>/dev/null; then
  log "installing webkit2gtk/gtk system packages"
  as_root env DEBIAN_FRONTEND=noninteractive apt-get update -qq
  as_root env DEBIAN_FRONTEND=noninteractive apt-get install -y -qq \
    libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev \
    >/dev/null
fi

# 2. Rust toolchain: CI floats on the latest stable (dtolnay/rust-toolchain
#    @stable), and each stable release adds clippy lints — a stale local
#    toolchain passes `-D warnings` that CI then fails. Track stable here too.
#    Components: clippy + rustfmt (lint floor), llvm-tools (`just cov`).
rustup update stable --no-self-update >/dev/null 2>&1
rustup component add clippy rustfmt llvm-tools-preview >/dev/null 2>&1

# 3. Cargo-installed tools the justfile calls. Compiled once, then cached in
#    the container snapshot.
for tool in just cargo-deny cargo-llvm-cov; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    log "cargo install $tool (first run only)"
    cargo install --locked --quiet "$tool"
  fi
done

# 4. lefthook (git hooks: fmt/clippy/frontend lint on commit, tests on push,
#    conventional-commit message check). Distributed via npm, not crates.io.
if ! command -v lefthook >/dev/null 2>&1; then
  log "installing lefthook"
  npm install --global --silent lefthook@1
fi

# 5. Pre-fetch the locked crate graph so builds never stall on the network.
cargo fetch --locked --quiet

# 6. Desktop frontend deps — only when the manifests changed since the last
#    successful install. The hash file lives inside node_modules (gitignored),
#    so it disappears with the packages it describes.
hashes=apps/desktop/node_modules/.session-start.sha256
if ! sha256sum --check --status "$hashes" 2>/dev/null; then
  rm -f "$hashes"
  log "npm install (apps/desktop)"
  (cd apps/desktop && npm install --no-audit --no-fund --loglevel=error) >&2
  sha256sum .claude/hooks/session-start.sh apps/desktop/package.json \
    apps/desktop/package-lock.json >"$hashes"
fi

# 7. Git hooks. Idempotent; refreshes the shims if lefthook.yml changed.
lefthook install >/dev/null

# 8. Session environment.
if [ -n "${CLAUDE_ENV_FILE:-}" ]; then
  {
    # Plain output in tool results; no ANSI noise.
    echo 'export CARGO_TERM_COLOR=never'
    echo 'export NO_COLOR=1'
    # One build dir for the checkout and every git worktree of it (agents
    # fan out into worktrees; a target/ per worktree is ~5 GB and a cold
    # rebuild each). Cargo's build-dir lock serialises concurrent builds.
    # Resolve the main checkout even when this session starts inside a
    # worktree: its git common dir is <main checkout>/.git.
    if [ -z "${CARGO_TARGET_DIR:-}" ]; then
      common=$(git rev-parse --path-format=absolute --git-common-dir)
      echo "export CARGO_TARGET_DIR=\"$(dirname "$common")/target\""
    fi
    # Cloud sessions have a fixed disk allowance; full debuginfo makes the
    # workspace build (tauri + tests) ~15 GB, line tables alone ~5 GB, and
    # backtraces keep their file:line.
    echo 'export CARGO_PROFILE_DEV_DEBUG=line-tables-only'
    echo 'export CARGO_PROFILE_TEST_DEBUG=line-tables-only'
  } >>"$CLAUDE_ENV_FILE"
fi

log "ready — run \`just\` to list recipes, \`just ci\` for the full gate"
