# schemas/ — published contract artifacts

This directory holds Curio's machine-readable JSON Schemas (draft 2020-12),
the versioned public API of the two published contracts:

| File | Contract | Stable `$id` |
|------|----------|--------------|
| `frontmatter.v1.json` | `curio.frontmatter.v1` — exported markdown notes (plus `curio.manifest.v1` under `$defs/manifest`) | `https://curio.dev/schemas/frontmatter.v1.json` |
| `events.v1.json` | `curio.events.v1` — append-only behavioral event log | `https://curio.dev/schemas/events.v1.json` |

The human-readable spec is
[../docs/design/contracts-draft.md](../docs/design/contracts-draft.md); these
artifacts are its normative machine form. The Rust mirror lives in
`crates/curio-types`, and its round-trip test suite validates every
serialized type against these files — the types are pinned to the published
schemas mechanically, not by convention. Example instances (valid and
invalid) live in `../fixtures/contracts/`.

## The versioned-immutability rule (binding)

Schema files are **versioned-immutable**: once published, a schema's
semantics are frozen. A breaking change mints a new file (`*.v2.json`) with a
new `$id` — it never edits v1 semantics. Additive, backward-compatible
clarifications are allowed only while no external consumer exists; after
that, additions also version. Every change is recorded in
[CHANGELOG.md](CHANGELOG.md). The `.claude/skills/contract-change/` skill
encodes the procedure.

## The `$id` policy

- Every schema file carries a stable, absolute `$id` of the form
  `https://curio.dev/schemas/<name>.v<N>.json`. The `$id` is the schema's
  identity: consumers reference and cache by it, so it never changes for a
  published version — a new version is a new file with a new `$id`.
- `curio.dev` is a **placeholder domain** until the project-identity
  decision lands. When the real domain is chosen, the `$id`s change once,
  as a coordinated, announced break, before any external consumer pins them.
- Instance documents do not reference the `$id`; they carry the in-band
  `schema` discriminator (`curio.frontmatter.v1`, `curio.events.v1`,
  `curio.manifest.v1`). The `$id` exists for validators and tooling.
- Schemas are self-contained: all shared definitions are duplicated into
  each file's `$defs` so a consumer never needs to resolve a cross-file
  (or network) reference to validate.
