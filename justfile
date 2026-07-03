# Curio — DX front door. `just` lists the recipes; `just ci` is the gate.
#
# The cargo workspace is webview-free (no Node, no npm, no webview needed).
# apps/desktop/ is the parked desktop sketch, outside the workspace until
# Phase 4 — nothing here touches it.

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

# Region-coverage floor on crates/curio-core — the enforced number. Ratchet
# rule in CONTRIBUTING.md: it only moves up, and it moves here and in
# .github/workflows/ci.yml together.
core-cov-floor := "85"

# Everything in the workspace that is NOT crates/curio-core, for the
# enforced report below. A new crate counts against the core floor until
# it is added here — the gate fails loud, never silently narrows.
cov-non-core-regex := "(crates/curio-cli|crates/curio-types|xtask)/"

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

# Regenerate generated test fixtures (fixtures/generated — gitignored, never committed)
fixtures:
    @echo "the seeded fixture generator lands in Phase 1 (docs/design/roadmap.md) — nothing to generate yet"

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
ci: fmt-check clippy test deny boundary cov doc blob-guard
    @echo "ci suite green"
