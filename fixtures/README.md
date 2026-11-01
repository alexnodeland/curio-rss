# fixtures/

Test fixtures for the Curio workspace. Everything here is local and static —
tests are hermetic and never touch the network (feed servers are simulated
with wiremock / in-process `127.0.0.1` servers fed from these files).

Current layout:

- `contracts/` — valid + invalid example instances per published contract
  (frontmatter, manifest, one per event type); the curio-types test suite
  validates them against `../schemas/*.json` and round-trips the valid ones
  through the Rust types (`*.noncanonical.json` = schema-valid but not
  byte-canonical, exempt from the equality assertion)

- `discovery/` — pages for feed autodiscovery (`curio_core::feeds::discover`):
  `wordpress-post.html` is a real-shaped WordPress post head declaring the
  site feed beside comment, REST-API and oEmbed alternates — only the site
  feed may be offered

- `export/` — destination-state inputs for the exporter's integration
  tests: `manifest-conflict/` is a `.curio/` with a manifest plus two
  sync-tool conflict copies (Dropbox-style `manifest (conflicted
  copy).json`, iCloud-style `manifest 2.json`) to detect and merge

- `generated/` — output of the deterministic seeded fixture generator
  (`crates/curio-fixtures`; default 1000 feeds / 50k articles). **Never
  committed** — rebuilt via `just fixtures` (`.gitignore` covers it). The
  generator is deterministic: the same seed yields a **byte-identical**
  `curio.db` (hash-asserted by the crate's own tests), written through the
  real migrated schema. It powers the criterion cold-start bench
  (`just bench-cold-start`) and can seed a profile for manual QA.
  `just bench-cold-start-ci` runs the bench the way the nightly `bench` job
  does — bencher output from a criterion that keeps no baseline of its own
  (the job's `github-action-benchmark` step owns the trailing baseline and
  fails at 130% of it), then `cargo run -p xtask -- bench-output` to check
  the tracker can parse the result.

- `db/` — migration fixtures: one small `vX.Y.Z.db` per released version,
  each written by **that release's own** storage code (the generator,
  `db/generate.rs`, was run inside a checkout of the tag — see its header
  for the recipe). `crates/curio-core/tests/migration_fixtures.rs` upgrades
  every one and asserts the pre-migration backup, row counts, FTS integrity
  and intent replay. Cutting a release adds its fixture here.

Planned layout (populated from later phases, per `docs/design/roadmap.md`):

- `feeds/` — real-world RSS/Atom/JSON Feed samples for the ingest pipeline
- `nasty/` — the hostile corpus: malformed XML, encoding bombs, stored-XSS
  attempts (`<script>`, `onerror=`), oversized entries
- `html/` — article HTML → readability → CommonMark conversion cases

Rule: no fixture file may exceed 1 MB — CI's blob-size guard fails the push
(the 50k `generated/` database is ~100 MB, which is exactly why it is
generate-on-demand and never committed).
