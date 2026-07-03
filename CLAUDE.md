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
| `apps/desktop/` | The parked Tauri/Svelte sketch — **outside the cargo workspace until Phase 4**; does not build, do not fix it |
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
cargo-deny, boundary check, the **85% region-coverage floor on
curio-core** (a ratchet — see CONTRIBUTING.md), rustdoc `-D warnings`,
and the 1MB blob guard — and must be green before any commit lands.
Hooks are lefthook (`just setup` installs them; they cover fmt/clippy on
commit and tests on push — the full gate is `just ci`).

## Commit conventions

Conventional Commits, enforced by the commit-msg hook:
`type(scope): description` — types `feat fix docs style refactor perf test
chore ci build revert`. Small, logical commits.

## Boundary rules (binding)

- **`curio-core` never depends on tauri** (or wry/tao/any webview). Enforced
  by `deny.toml` and `cargo run -p xtask -- boundary`; the workspace stays
  webview-free until Phase 4 deliberately moves the boundary.
- **Tests are hermetic.** No real network in tests — local fixtures,
  wiremock, in-process servers on `127.0.0.1` only.
- **yt-dlp is never bundled.** Post-v1 enrichment shells out to an external,
  pinned, SHA256-verified binary; no binary blobs in the repo (CI blob guard
  fails objects > 1MB).
- **All outbound requests go through the policed client** (SSRF guard) once
  `fetch/` exists — never a bare reqwest client.
- `unwrap`/`expect`/`todo!`/`dbg!` are clippy-denied outside tests.
