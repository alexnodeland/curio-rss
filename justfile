# Curio — DX front door. `just` lists the recipes; `just ci` is the gate.
#
# Phase 4 brought the desktop head (apps/desktop/src-tauri, curio-desktop)
# into the cargo workspace: the Rust gates below now compile it (on Linux
# that needs the webkit2gtk/gtk system packages — see ci.yml), but only
# the desktop head may depend on the webview (deny.toml wrappers + the
# boundary check). The Svelte frontend keeps its own npm toolchain under
# apps/desktop/; its gates (frontend-bans, desktop-lint, desktop-npm-test)
# ride `just ci` too and need a one-time `npm install` there.

# List available recipes
default:
    @just --list --unsorted

# First-time setup: install git hooks, verify required tools
setup:
    lefthook install
    @command -v cargo-deny >/dev/null || echo "warning: cargo-deny not found — install with: cargo install cargo-deny"
    @command -v cargo-llvm-cov >/dev/null || echo "warning: cargo-llvm-cov not found (needed by `just cov`, part of `just ci`) — install with: cargo install cargo-llvm-cov"
    @echo "setup complete — run 'just ci' to verify the workspace"

# Build the workspace
build:
    cargo build --workspace

# Run the test suite (hermetic — no network, fixtures only)
test:
    cargo test --workspace

# Format all Rust code
fmt:
    cargo fmt --all

# Check formatting without writing
fmt-check:
    cargo fmt --all -- --check

# Clippy with warnings denied — the workspace lint floor
clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# fmt-check + clippy
lint: fmt-check clippy

# Rustdoc with warnings denied — docs are part of the contract surface
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps

# Supply-chain gate: advisories, license allowlist, webview ban
deny:
    cargo deny check

# Defense in depth: assert curio-core's dependency tree is webview-free
boundary:
    cargo run -p xtask -- boundary

# The export test IS the generator:
# Regenerate the committed TS bindings (apps/desktop/src/lib/bindings.ts)
bindings:
    cargo test -p curio-desktop --test export_bindings

# The ipc-contract gate (mirrors CI):
# Regenerate the bindings, then fail on drift from the committed file
bindings-check: bindings
    git diff --exit-code apps/desktop/src/lib/bindings.ts

# Needs `npm install` under apps/desktop first:
# Run the desktop head in dev mode (webview + vite)
desktop-dev:
    cd apps/desktop && npm run tauri dev

# The desktop head's Rust test suite (commands against a temp-profile core)
desktop-test:
    cargo test -p curio-desktop

# Frontend bans: {@html} only in SanitizedHtml.svelte, raw invoke imports only in bindings.ts
frontend-bans:
    bash scripts/check-frontend-bans.sh

# Needs `npm install` under apps/desktop first:
# Desktop frontend lint at zero warnings: biome + eslint (svelte templates) + svelte-check
desktop-lint:
    cd apps/desktop && npm run lint && npm run lint:eslint && npm run check

# Needs `npm install` under apps/desktop first:
# Desktop frontend tests: vitest + mockIPC, 70% coverage floor on state/utils
desktop-npm-test:
    cd apps/desktop && npm run test:coverage

# Region-coverage floor on crates/curio-core — the enforced number. Ratchet
# rule in CONTRIBUTING.md: it only moves up, and it moves here and in
# .github/workflows/ci.yml together.
core-cov-floor := "85"

# Everything in the workspace that is NOT crates/curio-core, for the
# enforced report below. A new crate counts against the core floor until
# it is added here — the gate fails loud, never silently narrows.
cov-non-core-regex := "(crates/curio-cli|crates/curio-types|crates/curio-fixtures|xtask|apps/desktop/src-tauri)/"

# Coverage: workspace report + enforced region floor on crates/curio-core
cov:
    cargo llvm-cov --workspace --no-report
    @echo "── workspace coverage (report-only) ──"
    cargo llvm-cov report --summary-only
    @echo "── curio-core region floor: {{core-cov-floor}}% (enforced) ──"
    cargo llvm-cov report --summary-only \
        --ignore-filename-regex '{{cov-non-core-regex}}' \
        --fail-under-regions {{core-cov-floor}}

# Coverage with a browsable HTML report (target/llvm-cov/html)
cov-html:
    cargo llvm-cov --workspace --html
    @echo "open target/llvm-cov/html/index.html"

# Regenerate the deterministic perf fixture (fixtures/generated — gitignored,
# never committed; same seed -> byte-identical curio.db, SHA-256 printed).
# Sized for the 50k cold-start bench: 1000 feeds / 50k articles.
fixtures *ARGS:
    cargo run -p curio-fixtures --release -- {{ARGS}}

# Cold-start acceptance bench: CoreHandle open + first page on the 50k
# fixture (target < 1.5s). Generates the fixture once, then times the open.
bench-cold-start:
    cargo bench -p curio-fixtures --bench cold_start

# Blob-size guard: fail if any git object in history exceeds 1MB
# (mirrors CI's blob-guard job — catches the blob BEFORE it is pushed
# and immortalized in history)
blob-guard:
    #!/usr/bin/env bash
    set -euo pipefail
    limit=1048576
    oversized=$(git rev-list --objects --all \
      | git cat-file --batch-check='%(objecttype) %(objectname) %(objectsize) %(rest)' \
      | awk -v limit="$limit" '$1 == "blob" && $3 > limit { print $3 " " $2 " " substr($0, index($0, $4)) }' \
      | sort -rn || true)
    if [ -n "$oversized" ]; then
      echo "git objects larger than 1MB found in history:"
      echo "$oversized"
      exit 1
    fi
    echo "blob guard: OK — no object in history exceeds 1MB"

# Everything CI runs, in CI order — green here means green there
ci: fmt-check clippy test deny boundary bindings-check cov doc blob-guard frontend-bans desktop-lint desktop-npm-test
    @echo "ci suite green"
