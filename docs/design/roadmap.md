# Curio roadmap — six phases to v1.0

Distilled from [research/roadmap.json](research/roadmap.json). Honest total: **~28 weeks**
(6.5–8 months) for one developer with heavy AI-agent leverage — versus the old PRD's
17–23 weeks, which budgeted zero for release engineering, and the old validation report's
"Phase-1 ~70%" claim, re-based to ~35–40% salvage-by-relocation.

**v1.0 =** a publicly-announceable, single-machine, local-first power reader: **two
shipped heads** over one Tauri-free `curio-core` crate — the desktop app (macOS 13+,
Windows 10+, Linux; signed/notarized) and **`curio-cli`, which ships in v1** as the
agent/cron/scripting surface and the CI proof the core is headless. Articles-only reading
done excellently; the vault-export and events contracts as core modules with published,
semver'd schemas; every quality claim CI-gated, not asserted. Explicitly **not** in v1:
sync of any kind, Reddit/YouTube enrichment (layouts only), podcasts, profiles, theme
editor, server head.

## Phase 0 — Pre-public reset & workspace skeleton *(1.5 weeks — happening now)*

Settle every irreversible-after-first-artifact decision before feature code exists.

- Name/trademark clearance ("Curio" collides with Zengobi Curio); reverse-DNS bundle
  identifier replacing `com.curio-reader.app`; scrub stray "Flowfeed" branding.
- **History purge while still private** (done — this repo is freshly history-purged): the
  33MB committed yt-dlp binary is filter-repo'd out; gitignore `src-tauri/bin/` +
  `src-tauri/gen/`; dual LICENSE-MIT + LICENSE-APACHE.
- Branch reconciliation: cherry-pick the Cfqby branch's frontend-before-clippy CI fix +
  real icons; commit a salvage manifest of what ports and what dies.
- Cargo + npm workspace: `crates/curio-core`, `curio-types`, `curio-cli`, `apps/desktop`,
  `fixtures/`, `xtask/`; workspace lints (`unwrap_used`/`expect_used` deny outside tests);
  `deny.toml` with **tauri banned from curio-core's dependency tree**; curio-server
  demoted to a design doc only.
- Governance/docs skeletons (SECURITY, PRIVACY, GOVERNANCE, CONTRIBUTING, DCO, templates).
- Long-lead credentials started now: Apple Developer Program, Windows signing eligibility,
  updater minisign keypair generated offline + key-custody/rotation ADR.

Acceptance highlights: zero blobs > 1MB in history; a canary PR adding tauri to curio-core
fails CI; `cargo test --workspace` green on a webview-less Linux clone for core/types/cli.

## Phase 1 — Production scaffolding: CI/CD, release pipeline, test substrate *(2.5 weeks, overlaps Phase 0 tail)*

The OSS production layer **before** features, releasing from the first alpha.

- PR gate suite on one cheap Linux runner: fmt, clippy `-D warnings`, nextest, MSRV,
  frontend lints, and the **ipc-contract job** (regenerate tauri-specta bindings, diff —
  the single highest-value gate); cargo-deny enforced; SHA-pinned actions.
- Merge/nightly (not per-PR): 3-OS `tauri build` matrix, security scanners, criterion vs
  stored baseline (future public benchmark page).
- Release pipeline live from v0.1.0-alpha.1: draft releases, human-gated publish, single
  version source, SHA256SUMS + minisign + SBOM + provenance on every release.
- Test substrate before feature code: deterministic seeded fixture generator (1000 feeds /
  50k articles), wiremock harness, nasty-feed corpus skeleton, cargo-fuzz targets,
  coverage wiring (85% gate on core only, report-only elsewhere).

## Phase 2 — curio-core reading engine *(7 weeks)*

The hard 20% of an RSS reader as a headless, plain-`cargo test`-able crate.

- `store/`: migrations (INTEGER PK + UUIDv7, `article_state`/events split, FTS trigger
  scoped to content changes), writer thread + read pool behind the async repo facade,
  escaped FTS5 builder, keyset pagination, network-filesystem guard, retention that never
  reaps starred/read-later/exported items.
- `fetch/`: the PolicedClient — SSRF guard, size/redirect caps, timeouts, conditional GET
  preserved on error, 410 auto-pause.
- `ingest/`: parse → normalize (total dedupe chain, date fallbacks, xml:base, JSON Feed)
  → ammonia → readability → populated `content_text`; char-boundary-safe truncation.
- `pipeline/`: the fixed refresh design as the **only** fetch path — bounded
  `buffer_unordered`, per-feed backoff/health, batch single-transaction upserts.
- `events/` write path with cursor semantics designed now.
- Test walls: wiremock integration, proptest suites, concurrent reader+writer WAL tests,
  migration fixtures, writer-thread stress test.

Acceptance highlights: 1000-feed wiremock refresh < 30s (M-series) / < 60s (CI); ≥500-feed
nasty corpus parses without panic; FTS @100k p95 < 50ms; reads mid-refresh p95 < 100ms;
the hostile `<script>`/`onerror` fixture stores provably clean; ≥85% region coverage.

## Phase 3 — Vault export + event log (the strategic seam) + curio-cli *(3.5 weeks)*

- `export/`: htmd DOM-walk conversion with real-world fixtures; templated frontmatter
  mirroring the published contract; per-feed/per-tag routing; **named destination
  registry** (paths never cross IPC/CLI); dual-homed idempotency manifest with
  wipe-and-reinstall reconciliation; atomic writes.
- `events/` read side: JSONL projection, file sink with rotation + cursor checkpointing,
  webhook sink, `curio events tail` / `events since <cursor>`.
- `curio-types` contracts: JSON Schema artifacts + human-readable frontmatter-v1/events-v1
  docs with a written breaking-change policy; mandatory schema review before the first
  external consumer wires in.
- `curio-cli` v1 surface incl. `curio export --all` (full-portability archive); wired as
  the daily headless E2E job in CI.

Acceptance highlights: double-export → zero writes; wipe-and-reinstall reconciles with
zero duplicates; every emitted note/line validates against the schemas in CI; a
standalone no-Curio-code script reconstructs read/star state from vault + JSONL.

## Phase 4 — Desktop head: the v1 reader *(8 weeks)*

- tauri-specta generated bindings (committed, CI-diffed, hand-written `invoke()` banned);
  ~40 thin commands over `Arc<CurioCore>`.
- Security posture: strict CSP, per-window capabilities, no shell plugin, URL-scoped
  opener, `ipc_policy` path gate, single sanitized-render component + CI grep banning
  `{@html}` elsewhere.
- Runes state layer on the ported token system: backend-owned counts/filter/sort,
  virtualized lists, keyset pagination, 9 themes, shortcut registry (j/k, g-chords, `?`).
- Reader: view-mode dispatch, reddit/youtube **layouts** (no enrichment),
  youtube-nocookie click-to-load, one-keystroke promote-to-destination, read-later queue,
  search, OPML in/out, feed health.
- Platform correctness: single instance, config/data/cache path split, LRU image cache,
  panic hook + redacted diagnostics bundle (zero telemetry), i18n routing, a11y baseline.
- Honest test tiers: vitest + mockIPC component tests, tauri-driver smoke (Linux/Windows),
  versioned manual macOS checklist.

Acceptance highlights: cold start < 1.5s on the 50k fixture; stored-XSS fixture inert
end-to-end in the real webview; a complete keyboard-only session; zero hand-written IPC.

## Phase 5 — Beta hardening, docs, packaging → v1.0 *(5.5 weeks)*

- ≥4 weeks dogfooding at 300+ real feeds; feed-compatibility reports become corpus
  fixtures; beta updater channel exercised.
- Refugee importers: Pocket, Omnivore, Instapaper, Readwise, Feedly/Inoreader.
- Docs site: install guides, keyboard reference, **the vault-export + event-log
  integration guide**, schema contract pages, rehearsed release runbook.
- Public benchmark page; distribution (brew tap, AUR, winget; Flathub post-1.0).
- Final security pass: STRIDE-lite threat model, 24h fuzz-clean, updater upgrade-path test.

Acceptance highlights: 14 consecutive beta days with zero P0/P1; all artifacts signed;
`curio export --all` re-seeds a fresh install to identical state; a release cut purely by
following the runbook.

## Post-v1 (named seams, not commitments)

Server head (GReader/Fever/REST — stays a design doc until then); sync client mode
(StateBackend port → FreshRSS/Miniflux/Feedbin); file-based journal merge as **the**
serverless multi-device direction; enrichment providers behind cargo features (Reddit
JSON with circuit breaker; external pinned SHA256-verified yt-dlp — never bundled);
podcasts v1.1; git-push vault destination; highlights-in-vault; strictly-optional BYO-AI;
multi-profile (SQLite-first); MCP server head; Flathub. Full list and the first-30-days
launch plan: [research/roadmap.json](research/roadmap.json).
