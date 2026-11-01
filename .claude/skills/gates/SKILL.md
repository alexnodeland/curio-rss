---
name: gates
description: Pick and run the right local quality gates for a change before committing or pushing — maps changed paths to the fmt/clippy/test/bindings/coverage/frontend checks CI will run. Use before every push, and whenever unsure which `just` recipes a change needs.
---

# Gates — run what CI will run, scoped to the change

`just ci` is the whole gate and is always correct. It is also slow on a
4-core box, so while iterating, run the subset the change touches, then the
full `just ci` (or at least every row that applies) before the final push.
CI is the server-side copy of `just ci` plus two jobs with no local twin in
it: `msrv` (locally `just msrv`, see the table) and `test-cross-os` (the
engine + CLI tests on macOS and Windows, CI-only — keep paths, line endings
and process spawning portable). A red CI job that the local gates would have
caught is wasted wall-clock.

## Path → gates

| Changed | Run |
|---|---|
| any `*.rs` | `cargo fmt --all -- --check` · `cargo clippy --workspace --all-targets -- -D warnings` · `just check-prod` (libs + bins without dev-dependency features — the only gate where a head calling the test-only `CoreHandle::storage()` fails) |
| `crates/curio-core/**` | `cargo test -p curio-core` · `just cov` when adding/removing meaningful code (85% region floor, a ratchet — never lower it, never add exclusions) |
| `crates/curio-types/**`, `schemas/**` | `cargo test -p curio-types` (round-trips pin types to the schema artifacts) · contract-change skill |
| `crates/curio-cli/**` | `cargo test -p curio-cli` |
| `apps/desktop/src-tauri/**` | `cargo test -p curio-desktop` · if a `#[tauri::command]`, DTO or event type changed: `just bindings` and commit the regenerated `bindings.ts` (`just bindings-check` must be clean) |
| `apps/desktop/src/**` | `just frontend-bans` · `just desktop-lint` · `just desktop-npm-test` (70% floor on state/utils) · the visual-check skill for anything a user can see |
| `Cargo.toml` / `Cargo.lock` / new deps | `just deny` · `just boundary` (curio-core stays webview-free) · `just msrv` (every crate builds on its declared `rust-version`; needs `jq`, downloads the pinned toolchains) |
| `.github/workflows/**`, `.github/dependabot.yml` | `just workflows` (every action SHA-pinned with a `# vX.Y.Z` comment, release gated on `ci.yml`, updater signing key in exactly one build-free step of a job deployed to the `updater-signing` environment, draft release carries SHA256SUMS/SBOM/attestation) · `cargo test -p xtask` · a `release.yml` change also gets a `workflow_dispatch` dry run after merge (runbook §2) |
| docs / rustdoc comments | `just doc` (rustdoc `-D warnings`) |
| `fuzz/**` | `just workflows` (every `[[bin]]` target has a bounded, unconditional leg in nightly.yml's `fuzz` job) · `just fuzz <target> 30` if you have a nightly toolchain + cargo-fuzz (outside the stable gate; the PR's nightly run smoke-fuzzes every target for 60s) |
| anything added to git | `just blob-guard` (no object over 1 MB, ever) |

## Rules

- **Hermetic tests only.** No real network: fixtures, wiremock, `127.0.0.1`
  servers. A test that needs the internet is a bug (`just test-live` is the
  opt-in exception and never part of the gate).
- **Never weaken a gate to get green**: no `#[ignore]`, `.skip`, lowered
  floors, coverage exclusions, `#[allow]` without a justified comment, or
  `--no-verify` (the Bash guard hook blocks the last one).
- **Fix the cause, not the symptom.** A flaky test is a bug to root-cause
  (see commit 7627c87 for the house style), not a retry loop.
- Shared build dirs: concurrent worktrees may share a `CARGO_TARGET_DIR`;
  "Blocking waiting for file lock" is normal — wait, don't delete the dir.
  Cargo reuses a test binary built in another worktree as fresh, so tests
  resolve paths at run time (`std::env::var_os("CARGO_MANIFEST_DIR")`),
  never through `env!` — `cargo test -p xtask` fails on a compile-time read
  (#183). If a build reports items you just added as missing, another
  worktree's build of that crate looks fresh: `touch` the edited file and
  rebuild.
