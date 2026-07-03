---
name: add-source
description: Add a new content-source module to curio-core (a feed format, a site-specific layout, or a post-v1 enrichment provider). Use when a task asks Curio to understand a new kind of source.
---

# Add a content source

Three distinct kinds — pick the right one before writing code:

1. **Feed format** (e.g. a new syndication flavor): extend `ingest/`
   normalization. It must flow through the existing pipeline —
   parse → normalize → ammonia sanitize → readability → content_text → htmd.
   Never store raw source HTML; never bypass sanitize-at-ingest.
2. **View layout** (reddit/youtube-style rendering over RSS-native data):
   v1-legal. Core exposes data; heads own presentation. No new fetching.
3. **Enrichment provider** (extra API calls, external tools): post-v1 only,
   behind a cargo feature, implementing the `enrich/` trait ports. yt-dlp
   style external binaries are never bundled — external, pinned version,
   SHA256-verified, `--`-separated argv.

## Non-negotiables for any source

- All network I/O goes through the policed fetch client — SSRF guard,
  size/redirect caps, timeouts, conditional GET. No bare reqwest.
- Hermetic tests: add representative fixtures under `fixtures/feeds/` (and
  hostile cases under `fixtures/nasty/`), serve them via wiremock /
  in-process `127.0.0.1`. No fixture over 1MB.
- The hostile corpus must stay green: malformed input may fail gracefully,
  never panic, and `<script>`/`onerror` content must store provably clean.
- `curio-core` stays webview-free (`just boundary` green) and headless-
  testable (`cargo test --workspace` with no display server).
- New event types for source-specific behavior go through the
  `contract-change` skill, not ad hoc.
