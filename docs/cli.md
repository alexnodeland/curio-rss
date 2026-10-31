# `curio` — the CLI head

`curio` is the v1 surface of Curio: an agent-, cron-, and script-friendly
head over the `curio-core` engine. Every command is a thin adapter — the
fetch policy, sanitize-at-ingest pipeline, SQLite store, event log, and
export machinery all live in the core and behave identically under any
future head.

- Install: `cargo install --path crates/curio-cli` (Rust 1.88 or newer —
  the declared `rust-version`, enforced by CI's `msrv` job)
- **`--json` is a global flag** — pass it before or after any subcommand to
  get exactly one JSON document on stdout (read commands return their data;
  write commands return the changed/unchanged result). Parse, don't scrape.
- Diagnostics go to stderr, driven by `RUST_LOG` (e.g.
  `RUST_LOG=curio_core=debug curio fetch`).
- Errors exit non-zero with a one-line `error: …` on stderr.

## Commands at a glance

| Command | What it does |
|---------|--------------|
| `curio init [dir]` | Scaffold a profile (`curio.toml`, `curio.db`, events log) |
| `curio feed add <url> [--tags …]` | Subscribe (autodiscovers the feed on a site URL) |
| `curio feed list` / `curio feed rm <id>` | List / unsubscribe |
| `curio fetch` | Refresh every feed; reports new-article counts |
| `curio list [--unread\|--starred\|--tag …]` | List articles (keyset-paginated) |
| `curio show <id>` | Render an article as markdown in the terminal |
| `curio star` / `later` / `archive` / `tag` / `untag` `<id>` | Flip state — each is a `curio.events.v1` event |
| `curio search <query>` | Full-text search (FTS5) |
| `curio dest add <name> <path>` | Register a save destination |
| `curio save <id> --dest <name>` | Export a note per `curio.frontmatter.v1` |
| `curio import <file> --from <service>` | Import subscriptions + saved articles from a third-party export (idempotent) |
| `curio reddit login\|status\|logout` | Manage the optional BYO Reddit API credentials (OS keychain, per profile) |
| `curio events tail [-n N]` | Watch the behavioral event stream |
| `curio doctor` | DB integrity, FTS sync, events-log health |

A typical reading loop:

```mermaid
flowchart LR
    add["feed add"] --> fetch["fetch"] --> list["list --unread"]
    list --> show["show <id>"] --> flip["star / later / tag"]
    flip -->|append| events[".curio/events/"]
    show --> save["save --dest"] -->|write| note["markdown note"]
```

## The profile

All state lives in one profile directory:

| Path | Contents |
|------|----------|
| `curio.toml` | human-editable configuration (below) |
| `curio.db` | WAL SQLite: feeds, articles, state, FTS index |
| `curio.db.pre-vN.bak` | automatic snapshot taken before upgrading the database to schema `N` (the latest one plus the highest-numbered other kept; see [Upgrades and backups](#upgrades-and-backups)) |
| `.curio/events/events-YYYYMMDD.jsonl` | the `curio.events.v1` behavioral log |
| `.curio/.gitignore` | scaffolded so the events log is never committed |
| `.curio/profile-id` | the profile's opaque keychain id (created the first time a head loads Reddit credentials — every CLI command and app launch; holds no secret) |

The default profile is **shared with the desktop app** — the CLI and the
app open the same database, so `curio clip <url>` shows up in the reader
and a cron job sees the reader's library:

| Platform | Default profile |
|----------|-----------------|
| macOS | `~/Library/Application Support/io.github.alexnodeland.curio` |
| Linux | `~/.local/share/curio` (XDG-respecting: `XDG_DATA_HOME` wins) |
| Windows | `%LOCALAPPDATA%\alexnodeland\curio\data` |

Override per invocation with `--profile DIR` or persistently with
`CURIO_PROFILE=DIR` (the desktop app honours `CURIO_PROFILE` too).
`curio doctor` prints the resolved path and whether it is the shared
default.

```sh
curio init                 # scaffold the default profile
curio --profile /tmp/p init  # or an explicit one (great for testing)
```

`init` is idempotent: re-running never clobbers an existing `curio.toml`.
It is also the only command that creates a profile: every other command
refuses a directory without a `curio.db` (exit 1, with a "run `curio
init`" hint), so a mistyped `--profile` can't silently show an empty
library.

**Upgrading from 0.4 or earlier.** Older CLIs used
`~/Library/Application Support/curio` on macOS (and the Windows app kept
its library in Roaming AppData). The first run of either head on the
default profile moves a lone old profile into the shared location. If both
locations already hold a library — or the old one looks open in another
process — nothing is moved or merged: every command prints a warning and
`curio doctor` fails its `legacy-profile` check until you reconcile by
hand (open the old one with `--profile` to export what you need, then
remove it).

## `curio.toml`

```toml
[settings]
default_destination = "vault"   # what `curio save` uses when --dest is omitted
politeness_delay_ms = 500       # min spacing between requests to the same host
user_agent = "curio/0.2 (+https://github.com/alexnodeland/curio-rss)"  # optional; overrides the default curio/<version> UA

[destinations]
vault = "/home/you/notes/reading"

[feeds."http://127.0.0.1:8787/digest.xml"]
allow_private_network = true    # contract W1 — see below
```

Ownership split: feeds, articles, and reading state live in `curio.db`
(the engine's domain — don't edit); `curio.toml` owns what a human should
be able to change with a text editor. On every run the CLI reconciles the
file into the engine: destinations are (re-)registered and per-feed
overrides are applied.

**Contract W1 (`allow_private_network`).** The fetch client is
SSRF-guarded by default: it refuses to touch loopback, RFC 1918, and
link-local addresses, re-checking every redirect hop after DNS
resolution. A per-feed `allow_private_network = true` exempts one feed —
so a localhost digest feed stays subscribable. The flag is *explicit
configuration only*: edit `curio.toml`, or pass
`--allow-private-network` to `feed add` (which writes it to `curio.toml`
for you). Nothing in feed content can ever set it.

## Feeds

```sh
curio feed add https://example.com/feed.xml --tags rust,databases
curio feed add http://127.0.0.1:8787/digest.xml --allow-private-network
curio feed list [--json]
curio feed rm example.com          # by id, exact URL, or unique substring
```

`feed add` emits `feed.added` (with the tags — tags-in-payload rule);
`feed rm` emits the `feed.removed` negation and keeps the articles.

```sh
curio fetch                 # refresh every active feed
curio fetch example.com     # or just one
```

`fetch` reports per-feed status and new/updated article counts —
"updated" counts only entries that actually changed; a feed re-listing
identical items reports `0 updated` and rewrites no articles. The
engine does conditional GET (ETag/Last-Modified preserved on every error
path), honors permanent redirects, auto-pauses a feed forever on HTTP 410,
and stores only sanitized content — raw feed HTML never reaches the
database. An article whose full text was loaded (load-full, a feed's
full-text mode, or Reddit enrichment) keeps that body: later refreshes of
a feed still listing the item update its title and metadata, never revert
it to the feed's excerpt.

```sh
curio opml import subscriptions.opml   # folders become tags; known URLs skipped
curio opml export backup.opml          # "-" writes to stdout
```

Bring a whole library over from another reader with one idempotent command —
feeds become subscriptions, saved articles become read-later items with their
tags:

```sh
curio import subscriptions.opml --from opml         # --from defaults to opml
curio import pocket-export.csv   --from pocket       # Pocket / Instapaper / Readwise CSV
```

**Per-host fetch policy.** A few hosts fingerprint-block the honest `curio`
User-Agent + rustls TLS. `curio-core` carries a built-in per-host override
(matched by host suffix): `reddit.com` (and `*.reddit.com`) automatically get
a browser-class User-Agent, the platform-native TLS stack, and a longer 2 s
politeness delay — so `curio fetch` of a subreddit `.rss` works out of the box.
This override is hard-coded in the engine (not a `curio.toml` knob) and is
disclosed in [PRIVACY.md](../PRIVACY.md); every other host keeps rustls + the
honest `curio/<version>` UA and the default 500 ms delay.

**Reddit API credentials (optional).** Reddit enrichment works
unauthenticated, slowly. Bring your own free app from reddit.com/prefs/apps
(type "script") and Curio uses the authenticated API at the faster tier:

```sh
curio reddit login --client-id <id>   # reads the secret from stdin (keeps it out of shell history)
curio reddit status                   # configured? and the client id — never the secret
curio reddit logout                   # remove them; back to unauthenticated
```

The credentials live in the OS keychain, **one item per profile**: `--profile`
(or `CURIO_PROFILE`) selects whose app a command — including a cron
`curio fetch` — uses, and two profiles can hold different apps. Note that the
CLI's default profile is not the desktop app's, so credentials saved in the
app are not seen by a bare `curio fetch` unless you point `--profile` at the
app's profile directory. The keychain item is keyed by the id in
`.curio/profile-id`, so moving a profile directory keeps its credentials.
Upgrading from 0.4.x, the old shared item is copied into each profile the
first time it opens; `reddit logout` removes it.

## Reading

```sh
curio list [--unread] [--starred] [--read-later] [--tag rust] [--feed example.com] [--limit 20]
curio search "sqlite wal" [--limit 20]    # real FTS5, hostile input escaped
curio show 3e9f10aa                       # article as markdown; marks it read
curio open 3e9f10aa                       # emits article.opened, launches $BROWSER
```

Article handles are **any unique fragment of the `curio_id`** — listings
show the 8-character tail, which is the typable handle. (UUIDv7 ids share
their leading timestamp bits within a fetch batch, so a leading prefix
needs to be much longer; use the tail.) Ambiguous or unknown fragments
fail with a helpful message.

`open` uses `$BROWSER` when set, else the platform opener (`open` /
`xdg-open`). The `article.opened` event is written first — the record is
the point.

## State

```sh
curio star 3e9f10aa      # article.starred          curio unstar …  # article.unstarred
curio later 3e9f10aa     # article.read_later.added curio unlater … # article.read_later.removed
curio archive 3e9f10aa   # article.archived         curio unarchive …
curio tag 3e9f10aa rust  # article.tagged           curio untag … rust
```

Every flip is idempotent: re-starring a starred article changes nothing
and emits nothing (`"changed": false` in JSON). Negation events remove
membership when the stream is folded — histories are not monotone, per
the events contract.

## Saving to destinations

A destination is a named directory of markdown + YAML — your notes vault,
a git repo, anything. Paths enter the system exactly once, at
registration; everything else refers to destinations by name.

```sh
curio dest add vault ~/notes/reading
curio dest list
curio save 3e9f10aa --dest vault     # or set settings.default_destination
```

`save` writes a `curio.frontmatter.v1` note (machine frontmatter + the
marked managed region) and updates the destination's
`.curio/manifest.json` — note first, manifest second, both via atomic
rename. Export is idempotent on `(curio_id, checksum)`:

- first save → `created`, emits `article.saved`
- content changed since → `updated`, emits `article.updated`, replaces
  only the frontmatter machine keys and the managed region — everything
  you wrote outside the markers survives byte-for-byte
- unchanged → `unchanged`, writes nothing, emits nothing; if the note on
  disk is blank (emptied by a crash or a sync tool) it is quietly
  rewritten, and if it no longer parses as a Curio note (frontmatter or
  markers edited away, or bytes that are not even UTF-8) the save fails
  with a parse error instead of claiming it is intact

Bulk saves (`curio save --all --dest vault`, or a filter such as
`--read-later`) run every article through the same steps and stop at the
first one that fails — including a note that no longer parses, even if
its content has not changed. Notes saved before it stay saved; repair or
delete the offending note and re-run (already-exported notes come back
`unchanged`).

Notes land at `curio/<slug>.md`, where the slug is the title lowercased,
NFC-normalized and dash-separated, capped at 100 bytes so the name fits
every filesystem's 255-byte limit even with a dedup suffix
(`<slug>-<id prefix>.md` when another article already owns the name — in
the manifest, or because the note already on disk there belongs to a
different `curio_id`). Titles that would be a Windows device name (`con`,
`nul`, `com1`, …) get a `-note` suffix. Every write is flushed to disk
(`fsync` on the file and its directory) before the manifest records it,
and a failed write leaves no temp file behind. On filesystems that cannot
flush at all (some network shares, exFAT, FUSE mounts) the flush is
skipped rather than failing the save.

The manifest is a file in your folder, so Curio never trusts it alone:

- **A note is only rewritten for the article it belongs to** — the
  note's own frontmatter `curio_id` decides. If the manifest was lost,
  reset or restored from an old commit, a second "Weekly links" article
  gets `curio/weekly-links-<id-prefix>.md` instead of taking over the first
  one's note, and a file of your own at a slug path is never touched
  (a blank one — empty or whitespace only — holds nothing to keep, so it
  counts as free).
  An article whose own note is already there is reconciled onto it and
  reported as `updated` (never a second `created`).
- **One export per destination at a time**, across threads and processes
  (the CLI next to the desktop app): exports queue on an advisory lock,
  `.curio/manifest.lock` (gitignored, never deleted — it holds no data).
  An `unchanged` export writes nothing and takes no lock, so it works on
  a read-only folder too; on a network or FUSE mount without advisory
  locks Curio warns and serializes within its own process only.
- **A vanished vault is refused, not recreated.** If the destination
  folder is gone — or, once it has been exported to, its `.curio/` folder
  is — `save` fails with "destination folder … is missing" instead of
  re-exporting everything into an empty folder at the old path. Every
  command re-applies `curio.toml`'s destinations, but that never
  recreates such a folder. Reconnect the drive, or run `curio dest add
  <name> <path>` again (new or same path) to start the destination
  afresh; changing a destination's path in `curio.toml` does the same.
- **Sync-tool conflict copies** of the manifest (`manifest (conflicted
  copy).json`, iCloud's `manifest 2.json`, Syncthing's
  `manifest.sync-conflict-….json`, OneDrive's `manifest-HOSTNAME.json`)
  are merged back on the next export — union of the entries, the newest
  `exported_at` wins — and then removed. If the union gives one note path
  to two articles, the note's own `curio_id` keeps its entry; the other
  article gets a path of its own on its next export. Other files in
  `.curio/` (a `manifest.backup.json` of your own) are never touched.

## Events

```sh
curio events tail -n 20 [--json]
```

A debug view of the `curio.events.v1` stream: with `--json` you get the
envelopes exactly as they appear on disk (ULID `event_id`, millisecond
UTC `ts`, `type` + `payload`). Consumers should read the files directly —
`.curio/events/events-YYYYMMDD.jsonl`, append-only, rotated daily or at
50 MB, retained ≥ 90 days — and dedupe by `event_id`. See
[design/contracts-draft.md](design/contracts-draft.md) for the full
contract and [../schemas/](../schemas/) for the machine-readable schemas.

## Doctor

```sh
curio doctor [--json]
```

Health probes over the durable surfaces, exit code 1 if any fail:
schema version, `PRAGMA integrity_check`, FTS5-index/content sync,
events-log parseability, the events `.gitignore` scaffold, staged
event intents (a non-empty set means emission was interrupted — a crash,
or a flush that failed on a full disk or an unwritable events directory;
the change itself is already stored, and the next command, or the next
open, replays the staged events into the log), and quarantined intents
(`dead-intents`: staged rows whose envelope no longer decoded — they are
moved to `curio.db`'s `event_intents_dead` table so they stop blocking
every open and emit, and their events never reached the log).

## Upgrades and backups

Opening a profile whose database was written by an older Curio upgrades
it in place, after first writing a consistent snapshot next to it:
`curio.db.pre-vN.bak`, where `N` is the schema version being upgraded
to (the one just written and the highest-numbered other are kept — a
rollback to an older build never prunes its own fresh backup). The
upgrade runs as one transaction: if any step fails, the database stays
exactly at its old version and the open fails with a message naming the
backup. Two Curio processes upgrading the same profile at once (the
desktop app and a scheduled `curio`) are safe: each writes its snapshot
under its own temporary name, and the upgrade itself runs once, under
the database's write lock. A database written by a
*newer* Curio is refused untouched (update Curio instead), as is a file
that is not a Curio database at all.

To roll back by hand: quit every Curio process, delete `curio.db-wal`
and `curio.db-shm` if present, then copy the backup over `curio.db`.
