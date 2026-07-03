# schemas/ — published contract artifacts

This directory is the home of Curio's machine-readable JSON Schemas, the
versioned public API of the two published contracts:

| File | Contract | Stable `$id` |
|------|----------|--------------|
| `frontmatter.v1.json` | `curio.frontmatter.v1` — exported markdown notes | `https://curio.dev/schemas/frontmatter.v1.json` |
| `events.v1.json` | `curio.events.v1` — append-only behavioral event log | `https://curio.dev/schemas/events.v1.json` |

(`$id` domain is a placeholder until the project-identity decision lands.)

The schema files themselves are **generated from `curio-types` via schemars
in Phase 3** (see [../docs/design/roadmap.md](../docs/design/roadmap.md)) —
until then the authoritative spec is the human-readable
[../docs/design/contracts-draft.md](../docs/design/contracts-draft.md).

## The versioned-immutability rule (binding)

Schema files are **versioned-immutable**: once published, a schema's
semantics are frozen. A breaking change mints a new file (`*.v2.json`) with a
new `$id` — it never edits v1 semantics. Additive, backward-compatible
clarifications are allowed only while no external consumer exists; after
that, additions also version. Every change is recorded in
[CHANGELOG.md](CHANGELOG.md). The `.claude/skills/contract-change/` skill
encodes the procedure.
