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

- `generated/` — output of the deterministic seeded fixture generator
  (`crates/curio-fixtures`; default 1000 feeds / 50k articles). **Never
  committed** — rebuilt via `just fixtures` (`.gitignore` covers it). The
  generator is deterministic: the same seed yields a **byte-identical**
  `curio.db` (hash-asserted by the crate's own tests), written through the
  real migrated schema. It powers the criterion cold-start bench
  (`just bench-cold-start`) and can seed a profile for manual QA.

Planned layout (populated from later phases, per `docs/design/roadmap.md`):

- `feeds/` — real-world RSS/Atom/JSON Feed samples for the ingest pipeline
- `nasty/` — the hostile corpus: malformed XML, encoding bombs, stored-XSS
  attempts (`<script>`, `onerror=`), oversized entries
- `html/` — article HTML → readability → CommonMark conversion cases
- `db/` — migration fixture databases from every released version

Rule: no fixture file may exceed 1 MB — CI's blob-size guard fails the push
(the 50k `generated/` database is ~100 MB, which is exactly why it is
generate-on-demand and never committed).
