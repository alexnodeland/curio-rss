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
cargo-deny, boundary check, the bindings drift check (`just
bindings-check`, CI's ipc-contract job), the **85% region-coverage floor
on curio-core** (a ratchet — see CONTRIBUTING.md), rustdoc `-D warnings`,
the 1MB blob guard, and the frontend gates (`just frontend-bans` greps,
`just desktop-lint` = biome + eslint + svelte-check at zero warnings,
`just desktop-npm-test` = vitest + mockIPC with a 70% floor on the
state/util layer; they need `npm install` under apps/desktop once) — and
must be green before any commit lands. Hooks are lefthook (`just setup`
installs them; they cover fmt/clippy + the frontend lint/bans on commit
and tests on push — the full gate is `just ci`).

## Commit conventions

Conventional Commits, enforced by the commit-msg hook:
`type(scope): description` — types `feat fix docs style refactor perf test
chore ci build revert`. Small, logical commits.

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
