# Curio — agent guide

Curio is a local-first reader: it exports your reading into your notes as
plain markdown plus an append-only behavioral event log — the reading
surface of a personal knowledge plane. Pre-1.0, mid-reset; the build plan is
[docs/design/roadmap.md](docs/design/roadmap.md).

## Repo map

| Path | What it is |
|------|------------|
| `crates/curio-core` | The engine: fetch, ingest, store, state, export, events. **Never depends on tauri.** |
| `crates/curio-types` | Shared DTOs + published contract types; round-trip tests pin them to the `schemas/` artifacts |
| `crates/curio-cli` | The v1 head (`curio`): agent/cron/scripting surface, CI proof the core is headless |
| `xtask` | Workspace automation (`cargo run -p xtask -- boundary`) |
| `apps/desktop/` | The desktop head (Phase 4, in progress): `src-tauri/` is the `curio-desktop` workspace crate (thin IPC wrappers over the core, generated TS bindings); `src/` is the Svelte 5 **runes** frontend (query cache + stores over the generated bindings; no legacy stores) |
| `schemas/` | Published JSON Schema artifacts (versioned-immutable) + CHANGELOG |
| `fixtures/` | Hermetic test inputs; nothing over 1MB (CI blob guard) |
| `docs/design/` | Architecture, roadmap, contracts, decisions, history |

## The contract-first rule

`curio.frontmatter.v1` and `curio.events.v1` are published contracts. The
spec — [docs/design/contracts-draft.md](docs/design/contracts-draft.md) and
the artifacts in `schemas/` — wins over any code that disagrees with it.
Schemas are versioned-immutable: breaking change → new `*.v2.json`, never an
edit to v1 semantics. Use the `contract-change` skill before touching either
contract.

## Front door

All dev operations go through `just` (run bare to list recipes). `just ci`
runs exactly what CI runs — fmt-check, clippy `-D warnings`, tests,
cargo-deny, boundary check, the workflow supply-chain lint (`just
workflows`: SHA-pinned actions, release gated on CI, signing key isolated),
the bindings drift check (`just bindings-check`, CI's ipc-contract job),
the **85% region-coverage floor
on curio-core** (a ratchet — see CONTRIBUTING.md), rustdoc `-D warnings`,
the 1MB blob guard, and the frontend gates (`just frontend-bans` greps,
`just desktop-lint` = biome + eslint + svelte-check at zero warnings,
`just desktop-npm-test` = vitest + mockIPC with a 70% floor on the
state/util layer; they need `npm install` under apps/desktop once) — and
must be green before any commit lands. Two CI jobs have no `just ci` twin:
`test-cross-os` (engine + CLI tests on macOS and Windows, CI-only) and
`msrv` (every crate built on its declared `rust-version`; locally `just
msrv`, opt-in because it downloads toolchains — run it when a dependency,
`Cargo.lock` or a `rust-version` changes). Hooks are lefthook (`just setup`
installs them; they cover fmt/clippy + the frontend lint/bans on commit
and tests on push — the full gate is `just ci`).

## Commit conventions

Conventional Commits, enforced by the commit-msg hook:
`type(scope): description` — types `feat fix docs style refactor perf test
chore ci build revert`. Small, logical commits.

## Working agreement (trunk-based)

- `main` is always releasable. Every change lands as a short-lived branch →
  PR (mirroring `.github/PULL_REQUEST_TEMPLATE.md`) → green CI on the merging
  commit → **squash-merge**. Never commit on or push to `main`. Keep a
  feature branch current by rebasing it onto its target and pushing with
  `--force-with-lease --force-if-includes` (your own branch only). The
  **`ship-pr`** skill is the procedure.
- Before pushing, run the gates for what you touched — the **`gates`** skill
  maps paths to `just` recipes. `just ci` is the full gate.
- UI changes are validated by looking at them: the **`visual-check`** skill
  runs the frontend against the dev mock IPC and screenshots it with
  Playwright, light + dark.
- Feature PRs never edit `CHANGELOG.md` (the `release` skill writes it from
  merged history); they do update user-facing docs when behaviour changes.
- Out-of-scope problems you notice become new issues, not drive-by edits.
- Improvement work is tracked in the sweep epic (#110); issues carry `P1`/
  `P2`/`P3` and cite the code they're about.

## Skills

| Skill | Use for |
|---|---|
| `gates` | which local checks a change needs before push |
| `ship-pr` | branch → PR → CI loop → rebase + force-with-lease → squash-merge |
| `visual-check` | screenshot-driven validation of desktop UI changes |
| `contract-change` | anything touching `schemas/`, frontmatter keys, event types, the manifest |
| `add-source` | a new feed format / site layout / enrichment provider in curio-core |
| `release` | version bump, changelog, tag |

## Cloud sessions and hooks

`.claude/settings.json` wires four hooks (scripts in `.claude/hooks/`):

- **SessionStart** (`session-start.sh`, web sessions only): installs the
  webview system packages, Rust components, `just`/`cargo-deny`/
  `cargo-llvm-cov`, lefthook, the desktop npm deps (skipped when manifests are
  unchanged), and the git hooks — so `just ci` works from the first prompt.
  It tracks the latest stable Rust (CI floats on stable, and new clippy lints
  land with each release), exports `CARGO_TARGET_DIR=<main checkout>/target`
  (resolved through the git common dir, so a session started inside a
  worktree gets the same one) so the checkout and every git worktree share
  one build dir, and builds with line-table-only debuginfo so the workspace
  fits the session's disk allowance.

  Sharing has a consequence: cargo fingerprints a workspace crate by its
  source mtimes and a hash that leaves out the checkout's path, so a test
  binary built in worktree A counts as fresh in worktree B and runs there
  unrebuilt. Anything baked in at compile time therefore points into A — so
  code never reads `CARGO_MANIFEST_DIR` through `env!`/`option_env!`; it
  reads it at run time (`std::env::var_os`), which cargo sets for every
  process it runs (`cargo test`, `cargo run`). The `compile_time_paths`
  xtask test fails the suite on any compile-time read, and
  `.cargo/config.toml` hands insta its workspace root
  (`INSTA_WORKSPACE_ROOT`) at run time for the same reason (#183). The guard
  only sees this repo's sources: a dependency macro that expands to a
  compile-time read (as insta's does) needs its own run-time fix like that
  one. Cargo finds `.cargo/config.toml` from its working directory, so run
  cargo from inside the checkout (`--manifest-path` from elsewhere leaves
  insta on its fallback), and since cargo merges every config up the
  directory tree, a worktree under `.claude/worktrees/` on a branch older
  than that file inherits the main checkout's root — its golden tests read
  and write the main checkout's snapshots until it rebases onto main. The same
  freshness rule means a concurrent build in another worktree whose sources
  differ can leave a crate's output looking fresh to you while built from
  that worktree's code (symptom: errors about items you just added being
  missing); `touch` the file you edited and rebuild.
- **PreToolUse Bash** (`guard-bash.sh`): blocks `--no-verify`, plain
  `--force` pushes, pushes to `main`, and commits on `main`.
- **PreToolUse Edit/Write** (`guard-edit.sh`): blocks hand edits to the
  generated `bindings.ts`, and edits to published `schemas/*.vN.json` unless
  `schemas/CHANGELOG.md` already carries the pending entry (contract-change
  skill order).
- **PostToolUse Edit/Write** (`format-on-edit.sh`): rustfmt / biome-format the
  edited file so fmt gates never fail on whitespace.

If a hook blocks you, it is telling you the house rule — follow the message,
don't work around it.

## Boundary rules (binding)

- **`curio-core` never depends on tauri** (or wry/tao/any webview). Enforced
  by `deny.toml` (tauri bans `wrappers`-scoped to `curio-desktop` — the only
  crate allowed to pull the webview) and `cargo run -p xtask -- boundary`.
- **Desktop IPC types are generated, never hand-written.** `just bindings`
  regenerates `apps/desktop/src/lib/bindings.ts` from the Rust command
  surface; CI's `ipc-contract` job fails on drift. No hand-written
  `invoke()` anywhere.
- **Tests are hermetic.** No real network in tests — local fixtures,
  wiremock, in-process servers on `127.0.0.1` only.
- **yt-dlp is never bundled.** Post-v1 enrichment shells out to an external,
  pinned, SHA256-verified binary; no binary blobs in the repo (CI blob guard
  fails objects > 1MB).
- **All outbound requests go through the policed client** (SSRF guard) once
  `fetch/` exists — never a bare reqwest client.
- `unwrap`/`expect`/`todo!`/`dbg!` are clippy-denied outside tests.
