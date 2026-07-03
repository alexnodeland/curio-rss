# Curio design decisions — verdict-resolved

The record of choices resolved during the planning workflow. Sources: the three-lens
judging in [research/verdicts.json](research/verdicts.json), the deep-read of the old
sketch in [research/critique.json](research/critique.json) and
[research/backend.json](research/backend.json), and the production-engineering plan in
[research/ossEng.json](research/ossEng.json). Where the old sketch code disagrees with
[contracts-draft.md](contracts-draft.md), **the contract wins**.

## D1. Architecture: core-plus-heads (won 3-for-3)

Three candidate designs were judged under three adversarial lenses (OSS adoption;
one-person-plus-agents two-year maintenance; product/user). **core-plus-heads won all
three** (8.5 vs 7/8/6.5-range for the desktop-monolith and server-first candidates):

- vs **desktop-first monolith**: the monolith's crate purity is review-policed only — no
  shipped headless consumer proves it, so logic accretes into `src-tauri` over two years
  of agent-authored patches. core-plus-heads makes the boundary a CI-verified fact
  (cargo-deny ban + the CLI head's headless build), adds a scripting/agent surface the
  monolith lacks entirely, and opens a crates.io adoption channel.
- vs **server-first (curiod + cockpit)**: disqualified by ship-risk and maintenance
  profile for a solo maintainer — remote auth surface (every bug a potential CVE), three
  public protocols, unbounded GReader/Fever client-compat QA, doubled release matrix,
  embedded-server lifecycle bugs. Its own risk list named "shipping nothing" as the
  likeliest failure mode.
- Each losing design's best mechanisms were grafted in (see D13).

## D2. Storage: writer thread + WAL, not `Mutex<Connection>`

The sketch used a single sync `Arc<Mutex<Connection>>` — no WAL, no busy_timeout, sync
rusqlite called directly from async Tauri commands (blocking tokio workers, serializing
globally: a refresh visibly froze the UI), and `upsert_articles` ran 2N untransacted
statements with N implicit fsyncs per refresh. **Replaced by**: WAL + a dedicated writer
thread (channel-fed, all mutations transactional, batch `INSERT..ON CONFLICT`) + a read
pool via `spawn_blocking`, behind an async repo facade so heads never see rusqlite. A
single writer matches SQLite's actual concurrency model; no sqlx/ORM — boring, auditable
SQL. Fallback decision point (tokio-rusqlite/sqlx) is pre-agreed, not fought mid-crisis.

## D3. Schema: INTEGER PK + UUIDv7; state split out of articles

The sketch keyed FTS5 external-content to the implicit rowid of a TEXT-PK table —
**silently corrupted by VACUUM** — and used random UUIDv4 TEXT keys (insert-locality
poison at 100k rows), mixed `datetime('now')` and RFC3339 string formats (breaking
`ORDER BY` and parses), and assumed `guid` exists. **Resolved**: `INTEGER PRIMARY KEY`
rowid alias + UUIDv7 unique column; dedupe chain `guid → link → hash(title+published)` as
a total function; one timestamp format; `source_updated_at` split from row `modified_at`;
read/star/read-later state moved out of `articles` into an event-sourced `article_state`
projection — which also fixes retention (never reaps stated articles) and stops FTS
trigger churn on `is_read` flips.

## D4. FTS that actually indexes content

The sketch's full-text search was a silent no-op: `content_text` was never populated by
any code path, so the index's content column was always NULL — search matched
title/author only, and no test would have caught it. **Resolved**: the ingest pipeline
populates `content_text` as a mandatory stage; the FTS query builder escapes/compiles
user input (never raw `MATCH`, which threw user-facing DB errors on unbalanced quotes);
FTS triggers fire only on content changes.

## D5. Markdown conversion: htmd DOM-walk, not regex

The sketch's `html_to_markdown` — the exact seam the vault export depends on — was a
regex converter using `(.*?)` without `(?s)`: any element spanning lines (most real
`<p>`, every real `<pre><code>`) failed to match and was destroyed by the final
strip-all-tags pass. The `htmd` crate sat declared in Cargo.toml, used only in a comment.
**Resolved**: DOM-based HTML→CommonMark via htmd with a real-world golden-fixture suite;
`ImageExportMode`/`LinkStyle` silent no-ops deleted rather than shipped as lies.

## D6. Sanitize-at-ingest (the stored-XSS chain, killed at the root)

The sketch shipped a complete stored-XSS → arbitrary-file-write chain: the parser stored
raw feed HTML, the well-configured ammonia sanitizer was **called from nowhere**, the
frontend rendered `{@html article.content_html}` under `"csp": null` with no capabilities
file, and `export_to_markdown` accepted arbitrary destination paths over IPC.
**Resolved**: sanitization is a mandatory ingest stage (raw HTML never reaches the DB),
with strict CSP + per-window capabilities + a single sanitized-render component +
the named-destination registry as defense-in-depth — no single failure is sufficient for
compromise. The hostile-fixture ingest test is a permanent CI regression gate.

## D7. One fetch pipeline, policy-hardened

The sketch had two divergent refresh implementations: the good one
(`services/refresh.rs` — bounded concurrency, timeouts, conditional GET) was **completely
unwired dead code**; the shipped one was sequential with a per-call untimed reqwest
client (one hung feed wedged the app) and clobbered `etag`/`last_modified` on any error.
**Resolved**: the fixed refresh design becomes the *only* fetch path, over one mandatory
`PolicedClient` (SSRF deny-by-default post-DNS, redirect cap with per-hop re-check,
streaming size caps, timeouts, conditional-GET state preserved on error). The per-feed
`allow_private_network` allowlist (contract W1) is the only exemption, config-edit only.

## D8. yt-dlp: never bundled — and enrichment out of v1

The sketch committed an unverified 33MB single-platform yt-dlp binary to git, downloaded
updates without checksums, and passed URLs positionally with no `--` separator or
validation — flag injection (`--exec`) = RCE from a crafted feed link. Bundling also
breaks macOS notarization, Flathub, and distro norms; unauthenticated Reddit JSON is
~10 req/min and ToS-gray post-2023. **Resolved**: yt-dlp is **never bundled, ever** —
when the enricher returns (post-v1, behind an `Enricher` trait cargo feature) it is an
external binary: pinned version + SHA256 verified into the data dir, `--`-separated argv,
strict URL validation, no bare PATH fallback, no runtime self-update (Renovate bumps the
pin). v1 ships reddit/youtube *layouts* over RSS-native data only; the history purge
removed the binary from git.

## D9. IPC by construction: tauri-specta, not hand-typed types

The sketch's app was broken at runtime by a snake_case-vs-camelCase argument mismatch and
a drifted `CommandError` shape that 139 green tests never touched (ts-rs was scaffolding
theater: the feature existed, zero `#[ts(export)]`). **Resolved**: tauri-specta generates
TS commands, types, and event bindings from Rust signatures; bindings are committed and
CI-diffed; hand-written `invoke()` is lint-banned. The bug class becomes unrepresentable.

## D10. Contract semantics (KP consumer review, resolved into the contract)

Fixed in [contracts-draft.md](contracts-draft.md), binding on this build:

- **Identity vs change-token**: `curio_id` (UUIDv7) is identity; `checksum` is a change
  token only — consumers must never use checksum as identity.
- **Managed region**: Curio owns the frontmatter machine keys + one marked content
  region; everything outside is user-/other-tool-owned and preserved byte-for-byte.
- **Manifest rules**: sorted keys, one entry per line (git-mergeable); note written
  first, manifest second via atomic rename; export idempotent on `(curio_id, checksum)`.
- **Events**: ULID `event_id` (dedupe/replay idempotency), RFC 3339 UTC millis `ts`,
  tags-in-payload on state-carrying events (no DB join needed), **negation events**
  (histories are not monotone; folds must honor `unstarred`/`read_later.removed`/…),
  daily/50MB rotation, ≥90-day producer retention, consumer-owned `(file, line)` cursors.
- **Schema files are versioned-immutable**: breaking change → `*.v2.json`, never edit v1.

## D11. Honest engineering posture

- **No sync in v1**, stated plainly (the README carries the tiered "what v1 does NOT do"
  section verbatim); server head and client mode are named future seams; file-based
  journal merge is the named serverless direction.
- **curio-server demoted to a design doc** (judge graft): nothing unshipped has to keep
  compiling through two years of refactors, and governance explicitly holds it post-1.0.
- **Coverage gates only where meaningful**: 85% region on curio-core; report-only
  elsewhere. The old blanket 80/90% gates and the nursery cognitive-complexity clippy
  gate were theater the sketch itself violated.
- **Playwright-drives-Tauri dropped as unimplementable** (Playwright cannot drive a Tauri
  webview; no macOS tauri-driver; the `window.__TAURI_TEST_RESET__` hooks were a shipping
  backdoor). Replaced by mockIPC component tests + tauri-driver Linux/Windows smoke + a
  versioned manual macOS checklist.
- **Test reality over test count**: the sketch's 139 fast tests covered easy code and
  missed every load-bearing failure (dead FTS, IPC arg bug, vanishing folder subtrees,
  multi-line markdown destruction, UTF-8 truncation panic). Each of those becomes a
  permanent regression test; risky surfaces get corpus/proptest/fuzz walls.

## D12. Pre-public irreversibles (Phase 0)

Name/trademark clearance before any public artifact ("Curio" collides with Zengobi
Curio); reverse-DNS identifier replacing the malformed near-immutable
`com.curio-reader.app`; dual MIT/Apache-2.0 with cargo-deny enforcement; DCO not CLA;
history purged of >1MB blobs while private (done); updater minisign keypair generated
offline with a written custody/rotation plan; zero telemetry with PRIVACY.md enumerating
every network-call class; Google favicon fallback strictly opt-in.

## D13. Grafts adopted from the losing designs

From **server-first**: the named export-destination registry (paths never cross a
boundary); the network-filesystem guard on the SQLite DB (refuse Syncthing/iCloud/NFS
mounts without an override); webhook event sink; shared `events since <cursor>` semantics
as the server-head down-payment; the public benchmark page; the honest tiered sync
section. From **desktop-monolith**: the in-vault `.curio/manifest.json` with
wipe-and-reinstall reconciliation; `curio export --all` full-portability archive;
human-readable schema docs alongside JSON Schema artifacts; the `ipc_policy` path gate;
config/data/cache platform-path split; event-sink rotation/cursor ergonomics; per-PR CI
on one cheap Linux runner with the 3-OS Tauri matrix merge/nightly-gated.
