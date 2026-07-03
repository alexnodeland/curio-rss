# Curio published contracts — frontmatter/v1 + events/v1 (authoritative draft)

This document is the single source of truth for this build session. Both the Curio build
and the Knowledge Plane build implement exactly these semantics. It incorporates the
KP-side consumer review (negation events, ULID event_id, tags-in-payload, ts/cursor/rotation,
stable $ids, W1 allowlist, managed-region marker, manifest ordering/mergeability).

## Contract 1: `curio.frontmatter.v1` — exported markdown notes

Curio exports saved articles as markdown notes into **named destinations** (configured
directories, e.g. an Obsidian vault folder). Each note carries YAML frontmatter:

```yaml
---
schema: curio.frontmatter.v1
curio_id: 0197b2c4-8f3e-7cc1-a5d2-3e9f10aa4b6d   # UUIDv7, REQUIRED, assigned at save, never changes
title: "Article title"                            # REQUIRED
source: "https://example.com/article"             # REQUIRED, canonical article URL
feed: "https://example.com/feed.xml"              # feed URL the article came from, null if manual save
feed_title: "Example Blog"                        # human feed name, optional
author: "Jane Doe"                                # optional
published: 2026-07-01T12:00:00Z                   # RFC 3339 UTC, null if unknown
saved: 2026-07-03T09:15:00.123Z                   # RFC 3339 UTC millis, REQUIRED
tags: [rust, databases]                           # Curio-assigned tags at export time
checksum: "sha256:9f86d08..."                     # sha256 of the managed content region bytes
lang: "en"                                        # BCP-47, optional
word_count: 1234                                  # optional
---
```

**Identity vs change-token rule (verdict-resolved):** `curio_id` is identity. `checksum` is
a change token ONLY — consumers MUST NOT use checksum as identity (two articles can have
identical content; an edited article keeps its id). Checksum covers exactly the bytes of the
managed region (see below), not the whole file.

### Managed region marker

Curio owns the frontmatter machine keys above plus one marked content region. Everything
outside the region belongs to the user / other tools (KP companion enrichment):

```markdown
<!-- curio:managed:begin v1 -->
...extracted article markdown...
<!-- curio:managed:end -->

(anything here is never touched by Curio re-exports)
```

Re-export/update of the same `curio_id` replaces frontmatter machine keys + managed region
only, preserving all other content byte-for-byte. User-added frontmatter keys unknown to
Curio are preserved.

### `.curio/manifest.json` — export idempotency oracle

Per destination, `.curio/manifest.json` maps `curio_id` → export record:

```json
{
  "schema": "curio.manifest.v1",
  "notes": {
    "0197b2c4-...": { "path": "curio/article-title.md", "checksum": "sha256:9f86d08...", "exported_at": "2026-07-03T09:15:00.123Z" }
  }
}
```

Rules: (1) keys sorted, 2-space indent, one entry per line → git-mergeable diffs;
(2) **write ordering**: note file written first, manifest updated second via temp-file +
atomic rename — a crash leaves an orphan note, never a dangling manifest entry;
(3) export is idempotent on (curio_id, checksum): unchanged → no write;
(4) consumers (KP) read the manifest to learn write-ownership — paths present in the
manifest are Curio-owned at the managed-region level.

## Contract 2: `curio.events.v1` — append-only behavioral event log

Per profile, directory `.curio/events/` holds append-only JSONL files. This is the
reading-behavior feed the KP librarian consumes. **Never committed to git**
(gitignored in every destination and in Curio's own state dir).

### Envelope (one JSON object per line)

```json
{"schema":"curio.events.v1","event_id":"01J1PZ2M3N4P5Q6R7S8T9V0W1X","ts":"2026-07-03T09:15:00.123Z","type":"article.saved","payload":{...}}
```

- `event_id`: **ULID**, unique, REQUIRED. Consumers dedupe/replay-idempotency by event_id.
- `ts`: RFC 3339 UTC with millisecond precision, REQUIRED. Within a single file, `ts` is
  non-decreasing; across files, file naming gives ordering (see rotation).
- `type`: namespaced string, `article.*` / `feed.*`.
- `payload`: object; every `article.*` event carries at minimum `curio_id`; state-carrying
  events carry the full article snapshot **including `tags`** (tags-in-payload rule, so
  consumers never need a DB join).

### Event types (v1)

| type | payload | negates |
|------|---------|---------|
| `article.saved` | full snapshot: curio_id, title, source, feed, feed_title, tags, published, destination, path, checksum | — |
| `article.updated` | same snapshot (re-export with changed checksum) | — |
| `article.starred` | {curio_id, tags} | — |
| `article.unstarred` | {curio_id} | article.starred |
| `article.read_later.added` | {curio_id, tags} | — |
| `article.read_later.removed` | {curio_id} | article.read_later.added |
| `article.archived` | {curio_id} | — |
| `article.unarchived` | {curio_id} | article.archived |
| `article.opened` | {curio_id, dwell_ms?} | — |
| `article.tagged` | {curio_id, tag} | — |
| `article.untagged` | {curio_id, tag} | article.tagged |
| `feed.added` | {feed, feed_title, tags} | — |
| `feed.removed` | {feed} | feed.added |

**Negation rule (verdict-resolved):** state reconstruction = fold events in order; a
negation event removes membership. Histories are NOT monotone; consumers implementing
"starred set" etc. MUST honor negations.

### Rotation, retention, cursors

- Files: `events-YYYYMMDD.jsonl` (UTC date of first event), append-only, never rewritten.
  Rotation at UTC midnight or 50 MB, whichever first (size overflow → `-2`, `-3` suffix).
- Producer retains files ≥ 90 days; consumers must tolerate older files vanishing.
- Consumer cursor = `(file_name, line_number)`; on resume, if `file_name` is gone, restart
  from the oldest existing file (dedupe by event_id makes this safe). Cursors are
  consumer-owned; Curio never reads them.

## W1: per-feed private-network allowlist

Curio's fetch client is SSRF-guarded by default (blocks private/loopback/link-local IPs
after DNS resolution, blocks redirects into private ranges). Per-feed config
`allow_private_network = true` (default false, requires explicit config edit — never
settable from feed content) exempts a feed. This keeps localhost digest feeds (e.g. a KP
librarian digest served on 127.0.0.1) subscribable. Reserved: `x-curio-digest: true`
frontmatter/config marker for digest feeds so they can be excluded from re-ingest loops.

## JSON Schemas

Machine-readable schemas live in the Curio repo at `schemas/frontmatter.v1.json` and
`schemas/events.v1.json` with stable `$id`s:
- `https://curio.dev/schemas/frontmatter.v1.json` (placeholder domain until identity decision)
- `https://curio.dev/schemas/events.v1.json`
Schema files are versioned-immutable: breaking change → new file `*.v2.json`, never edit v1
semantics. Each schema dir carries a CHANGELOG.md.
