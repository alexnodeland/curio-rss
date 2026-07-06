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

Once released: `cargo install curio-cli`. Until then, from a clone,
`cargo install --path crates/curio-cli` (or run via `cargo run -p
curio-cli --`). Then:

```sh
curio init                                  # scaffold the profile: curio.toml, curio.db, events log
curio feed add https://example.com/feed.xml --tags rust
curio fetch                                 # refresh, report new-article counts
curio list --unread                         # what's waiting (--json on any read command)
curio show 3e9f10aa                         # rendered markdown in the terminal
curio star 3e9f10aa                         # …later/archive/tag/untag; every flip is an event
curio dest add vault ~/notes/reading        # name a destination directory once
curio save 3e9f10aa --dest vault            # export the note per curio.frontmatter.v1
curio events tail -n 5                      # watch the curio.events.v1 stream
curio doctor                                # db integrity, FTS sync, events-log health
```

Full command walkthrough + `curio.toml` format: [docs/cli.md](docs/cli.md).

Developing? From a clone:

```sh
just setup   # git hooks + tool checks (needs Rust stable, just, lefthook, cargo-deny, cargo-llvm-cov)
just ci      # fmt, clippy -D warnings, hermetic tests, cargo-deny, boundary check,
             # coverage floor (85% regions on curio-core), rustdoc -D warnings, blob guard,
             # frontend gates (biome/eslint/svelte-check, vitest, {@html}+invoke bans)
just         # list all recipes
```

No Node or npm required for engine work; the desktop head's Rust crate is
in the workspace (on Linux, `--workspace` builds need the webkit2gtk/gtk
system packages — see CONTRIBUTING.md), and the core itself stays headless.
Frontend work needs Node + `npm install` under `apps/desktop/` — the
frontend gates then ride `just ci`.

## Architecture

One Tauri-free engine crate, many thin heads:

| Crate | Role |
|-------|------|
| `crates/curio-core` | fetch, ingest, store, state, export, events — the whole engine |
| `crates/curio-types` | shared DTOs + the published contract schemas |
| `crates/curio-cli` | `curio`, the v1 head: agent/cron/scripting surface |
| `apps/desktop` | Tauri 2 + Svelte 5 reader — `curio-desktop`, the Phase 4 head (in the workspace) |

The boundary is mechanically enforced: `deny.toml` scopes the tauri bans so
only the desktop head may pull the webview, and CI proves curio-core's own
tree builds headless on a bare Linux runner. Full design: [docs/design/architecture.md](docs/design/architecture.md).

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
