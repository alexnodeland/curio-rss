# Contributing to Curio

Thanks for your interest. Curio is pre-1.0 and mid-reset — the ground rules
below are stable, the code is not yet. Issues and small PRs are welcome;
coordinate before starting anything large.

## Dev setup

Everything goes through [`just`](https://github.com/casey/just) — run `just`
with no arguments to list the recipes.

```sh
git clone https://github.com/alexnodeland/curio-rss
cd curio-rss
just setup     # installs git hooks (lefthook) and checks required tools
just ci        # everything CI runs: fmt, clippy, tests, cargo-deny, boundary
```

Required: stable Rust (see `rust-version` in `Cargo.toml`),
[lefthook](https://github.com/evilmartians/lefthook),
[cargo-deny](https://github.com/EmbarkStudios/cargo-deny).
Not required: Node, npm, or a webview — the cargo workspace builds and tests
fully headless. `apps/desktop/` is the parked desktop sketch and is outside
the workspace until Phase 4.

## Ground rules

- **Contract first.** The published contracts (`curio.frontmatter.v1`,
  `curio.events.v1`) in [docs/design/contracts-draft.md](docs/design/contracts-draft.md)
  and `schemas/` are load-bearing. Code conforms to the contract, never the
  other way around; breaking changes mint a new version (see `schemas/README.md`).
- **The core stays headless.** `curio-core` must never depend on tauri or any
  webview. `deny.toml` and `cargo run -p xtask -- boundary` enforce this in CI.
- **Tests are hermetic.** No test may touch the real network — use fixtures
  from `fixtures/` and in-process servers on `127.0.0.1` (wiremock).
- **Lints are the floor.** `cargo fmt` clean, `cargo clippy --workspace
  --all-targets -- -D warnings` clean, `cargo test --workspace` green.
  `unwrap`/`expect` are denied outside tests.
- **No blobs.** CI fails the push if any git object exceeds 1 MB.

## Commits and PRs

- Commit subjects follow [Conventional Commits](https://www.conventionalcommits.org/):
  `type(scope): description` with types `feat fix docs style refactor perf
  test chore ci build revert`. The commit-msg hook enforces this.
- Keep PRs focused; one logical change per PR. CI must be green.

## Developer certificate of origin (DCO-light)

By submitting a contribution you certify that you wrote it (or have the right
to submit it) and that you license it under this project's terms,
MIT OR Apache-2.0, per the
[Developer Certificate of Origin](https://developercertificate.org/).
No `Signed-off-by` trailer is required — opening the PR is your assertion.

## Security issues

Never open a public issue for a vulnerability — see [SECURITY.md](SECURITY.md).
