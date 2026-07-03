# Curio

> **Status: pre-1.0 — workspace reset in progress.** The original desktop
> sketch is parked in `apps/desktop/` while the engine is rebuilt properly
> from the core out. Nothing here is released yet; contracts and layout are
> settling. Follow [docs/design/roadmap.md](docs/design/roadmap.md).

Curio is a **local-first reader** — RSS/Atom feeds and read-later — that
treats your notes as the destination, not a silo. It exports what you read
into your own directories as **plain markdown** with typed YAML frontmatter,
and records how you read as an **append-only behavioral event log** (JSONL)
your own tools can consume. It is the reading surface of a personal
knowledge plane: your vault and your scripts are first-class consumers, via
two small, versioned, published contracts:

- **`curio.frontmatter.v1`** — every saved article becomes a markdown note
  with machine frontmatter and a marked managed region; everything you add
  outside the region is preserved byte-for-byte on re-export.
- **`curio.events.v1`** — saved/starred/read-later/tagged/opened events as
  append-only JSONL under `.curio/events/`, never committed to git, built
  for replay by downstream consumers.

Spec: [docs/design/contracts-draft.md](docs/design/contracts-draft.md) ·
Artifacts: [schemas/](schemas/) ·
Privacy stance: [PRIVACY.md](PRIVACY.md) — no telemetry, no phone-home; the
only thing that leaves your machine is fetching the feeds you subscribed to.

## Quickstart

Once released: `cargo install curio-cli`. Until then, from a clone:

```sh
just setup   # git hooks + tool checks (needs Rust stable, just, lefthook, cargo-deny)
just ci      # fmt, clippy -D warnings, hermetic tests, cargo-deny, boundary check
just         # list all recipes
```

No Node, npm, or webview required — the cargo workspace builds and tests
fully headless.

## Architecture

One Tauri-free engine crate, many thin heads:

| Crate | Role |
|-------|------|
| `crates/curio-core` | fetch, ingest, store, state, export, events — the whole engine |
| `crates/curio-types` | shared DTOs + the published contract schemas |
| `crates/curio-cli` | `curio`, the v1 head: agent/cron/scripting surface |
| `apps/desktop` | Tauri 2 + Svelte 5 reader — rejoins the workspace in Phase 4 |

The boundary is mechanically enforced: `deny.toml` bans tauri from the
workspace graph and CI proves the core builds headless on a bare Linux
runner. Full design: [docs/design/architecture.md](docs/design/architecture.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) (dev setup, conventional commits,
hermetic-test rule), [SECURITY.md](SECURITY.md) for private vulnerability
disclosure, and [GOVERNANCE.md](GOVERNANCE.md).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or
  <http://www.apache.org/licenses/LICENSE-2.0>)
- MIT license ([LICENSE-MIT](LICENSE-MIT) or
  <http://opensource.org/licenses/MIT>)

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
