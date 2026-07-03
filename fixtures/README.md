# fixtures/

Test fixtures for the Curio workspace. Everything here is local and static —
tests are hermetic and never touch the network (feed servers are simulated
with wiremock / in-process `127.0.0.1` servers fed from these files).

Planned layout (populated from Phase 1 onward, per `docs/design/roadmap.md`):

- `feeds/` — real-world RSS/Atom/JSON Feed samples for the ingest pipeline
- `nasty/` — the hostile corpus: malformed XML, encoding bombs, stored-XSS
  attempts (`<script>`, `onerror=`), oversized entries
- `html/` — article HTML → readability → CommonMark conversion cases
- `db/` — migration fixture databases from every released version
- `generated/` — output of the deterministic seeded fixture generator
  (1000 feeds / 50k articles); never committed, rebuilt via `just fixtures`

Rule: no fixture file may exceed 1 MB — CI's blob-size guard fails the push.
