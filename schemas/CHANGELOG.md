# Schema changelog

All notable changes to the published contract schemas are recorded here.
Rule: breaking change → new `*.vN.json` file with a new `$id`; v(N-1)
semantics are never edited. See [README.md](README.md).

## v1 — initial publication (2026-07-03)

- `frontmatter.v1.json` (`$id` `https://curio.dev/schemas/frontmatter.v1.json`):
  `curio.frontmatter.v1` — YAML frontmatter of exported notes. UUIDv7
  `curio_id` as identity, `sha256:`-prefixed checksum as change token ONLY
  (covers the managed region bytes), RFC 3339 UTC millis `saved`,
  nullable-but-present `feed`/`published`, additional (user) keys allowed.
  Includes `curio.manifest.v1` (`.curio/manifest.json`) under
  `$defs/manifest`.
- `events.v1.json` (`$id` `https://curio.dev/schemas/events.v1.json`):
  `curio.events.v1` — JSONL event envelope (ULID `event_id`, RFC 3339 UTC
  millis `ts`) with per-type payload requirements for the 13 v1 event
  types, the tags-in-payload rule on state-carrying events, and negation
  semantics documented per negation event.
- Both authored per
  [../docs/design/contracts-draft.md](../docs/design/contracts-draft.md)
  and pinned to `crates/curio-types` by the round-trip test suite
  (supersedes the earlier plan to generate them via schemars in Phase 3).
