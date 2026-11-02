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
  copy).json`, iCloud-style `manifest 2.json`) to detect and merge;
  `frontmatter/` holds the byte-for-byte frontmatter goldens
  (`render/<case>.json` → the note Curio writes, `reexport/<case>.edited.md`
  → the note a re-export writes over a hand-edited one). The expected `.md`
  files are the compatibility baseline the notes in users' vaults were
  written with — a diff in them is a `curio.frontmatter.v1` contract
  decision, never a regenerate-and-commit

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

- `feeds/` — well-formed RSS/Atom/JSON Feed (and Reddit JSON) samples for
  the ingest pipeline's unit and integration tests

- `nasty/` — the **seeds** of the hostile corpus: RSS 1.0/RDF, JSON Feed
  1.0/1.1 oddities, `xml:base`, duplicate guids, every kind of bad date,
  media/iTunes namespaces, HTML served as a feed, malformed and truncated
  XML, an entity bomb, stored-XSS payloads (`<script>`, `onerror=`,
  `javascript:`), entry links with parseable custom schemes (the content
  base a feed author controls — `custom-scheme-links.xml`, the first
  `feed_parse` fuzz find), an entry nested 900 levels deep
  (`deep-nesting.xml`: the depth that aborted an export, #244), and a
  hostile Reddit listing.
  `crates/curio-core/tests/nasty_corpus.rs` pins each file (with
  `feeds/`) to its expected outcome — parses with N entries, or rejected —
  and **a new file here must be added to its `SEEDS` table**
  (`committed_seeds_are_all_pinned` fails otherwise). The corpus itself,
  over 1000 inputs, is generated from these seeds at test time,
  deterministically: byte-level encoding variants (BOMs, UTF-16, Latin-1,
  lying declarations, control characters), size/shape extremes (HTML
  nested 10 000 levels deep, a ~700 KB entry, thousands of items,
  attribute floods) and seeded mutants. That is why nothing large is
  committed. Every input must end in a classified outcome with no panic,
  and everything it stores must pass the inertness audit
  (`tests/common/inert.rs`). The same files seed the `fuzz/` targets
  (`just fuzz`, nightly), and a crash a fuzz target finds is pinned back
  here as a new seed.

- `html/` — article HTML → sanitize → extract → CommonMark cases, pinned to
  insta snapshots; `xss-corpus.html` is the stored-XSS regression gate.
  They also seed the `content_pipeline` fuzz target, whose checks
  `content_golden.rs` replays over every file here, so a find pinned as a
  fixture (`c0-control-urls.html`, the first one) stays fixed

- `import/`, `opml/` — refugee-importer exports and OPML subscription lists
  (`opml/` also seeds the `opml` fuzz target)

Rule: no fixture file may exceed 1 MB — CI's blob-size guard fails the push
(the 50k `generated/` database is ~100 MB, which is exactly why it is
generate-on-demand and never committed).
