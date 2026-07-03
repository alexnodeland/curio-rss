# Privacy

Curio is **local-first**. Your reading data — feeds, articles, read/star
state, tags, the behavioral event log — lives on your machine, in files and
a SQLite database you own, and nowhere else.

## What Curio sends over the network

Exactly one thing: **fetches of the feeds and articles you subscribed to**
(and their favicons/images). Requests go to the servers you chose by adding
a feed, with an honest User-Agent. That's it.

## What Curio never does

- **No telemetry.** No usage analytics, no crash reporting to us, no
  metrics, no "anonymous statistics".
- **No phone-home.** No update pings, no license checks, no first-run
  beacons. (When a release updater exists, checking for updates will be
  explicit and off-by-default or clearly user-controlled.)
- **No accounts.** There is no Curio service to sign in to.
- **No third parties.** Your data is never sent to, shared with, or sold to
  anyone, because it never leaves your machine.

## Your data, your files

- Saved articles export as **plain markdown** with YAML frontmatter into
  directories you configure (`curio.frontmatter.v1`).
- Reading behavior is an **append-only local JSONL event log**
  (`curio.events.v1`) under `.curio/events/` — designed for *your* tools to
  consume, gitignored by default so it never accidentally leaves via a repo.
- `curio export --all` (Phase 3) gives you the complete portable archive:
  OPML, events, settings. Deleting Curio's data directory deletes everything.

## Diagnostics

If you hit a crash, any diagnostics bundle is generated locally, redacted,
and shared only if **you** attach it to a report yourself.
