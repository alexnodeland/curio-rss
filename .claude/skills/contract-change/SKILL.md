---
name: contract-change
description: Change or extend a published Curio contract (curio.frontmatter.v1, curio.events.v1, curio.manifest.v1) without breaking consumers. Use whenever a task touches frontmatter keys, event envelope/types, manifest shape, or anything under schemas/.
---

# Contract change

The published contracts are Curio's public API — the Knowledge Plane is
consumer #1. The spec (`docs/design/contracts-draft.md` + `schemas/*.json`)
wins over code; code conforms to it, never the reverse.

## The versioned-immutability rule (binding)

1. **Never edit vN semantics.** A breaking change — removed/renamed field,
   changed type or meaning, changed ordering/rotation/negation rules — mints
   a **new** schema file (`*.v2.json`, new `$id`), a new `schema:` string
   (`curio.events.v2`), and leaves v1 exactly as it was. Producers may
   dual-emit during migration; consumers pin by `schema` field.
2. **Additive changes** (new optional field, new event type) are allowed
   within a version ONLY while no external consumer depends on its absence —
   after first external consumer, treat additions as versioned too unless
   the consumer confirms tolerance (both contracts require consumers to
   ignore unknown fields; new *event types* are safe by design).
3. **Every change**, additive or breaking, lands in `schemas/CHANGELOG.md`
   in the same commit.

## Procedure

1. Read `docs/design/contracts-draft.md` and the current schema artifacts.
2. Classify: additive vs breaking (when in doubt, breaking).
3. Update spec + schema + `curio-types` (constants, DTOs, schemars derives)
   + `schemas/CHANGELOG.md` in one commit: `feat(contracts): ...` or
   `feat(contracts)!: ...` for breaking.
4. Update every fixture and test that exercises the contract; round-trip
   tests must cover both old and new versions when dual-emitting.
5. Invariants that must survive any change: `curio_id` is identity /
   `checksum` is a change-token only; events are append-only with ULID
   `event_id`; negation events per the table in the spec; manifest keys
   sorted + atomic write ordering (note first, manifest second).
