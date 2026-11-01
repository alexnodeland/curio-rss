# Changelog

All notable changes to the Curio app. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

(The versioned Curio↔notes **contract** schemas have their own changelog at
[`schemas/CHANGELOG.md`](schemas/CHANGELOG.md).)

## [0.5.0] — 2026-11-01

The data-safety release: a sweep through everything that could lose, revert,
or corrupt your reading — notes, full text, events, and the library itself.
Exports never clobber a note and land durably, upgrades back up first and
recover instead of crashing, and the CLI and the app now share one profile
safely. It also patches the RUSTSEC advisories in the HTML sanitizer and the
TLS stack, and hardens the release pipeline.

### Added

- **Startup recovery screen** — if your library can't be opened (a corrupt
  or too-new database, a busy profile, a failed upgrade), the app now opens
  to a plain-words recovery screen instead of quitting silently. It offers
  the most useful next step — restore the pre-upgrade backup, try again,
  check for updates, or start fresh — plus Show profile folder, Copy error,
  and Quit. Recovery never deletes anything: a replaced database is moved to
  `<profile>/set-aside/<timestamp>/`, and a healthy one is never moved.
- **Pre-migration backups** — before upgrading an existing library, Curio
  snapshots it to `curio.db.pre-vN.bak` (the two newest are kept). No
  backup, no upgrade. docs/cli.md explains how to roll back by hand.
- **Network-filesystem guard** — a profile on NFS, SMB, AFP, WebDAV,
  iCloud Drive, Dropbox, or Syncthing storage is refused up front (SQLite is
  not safe there) instead of risking corruption. Override with
  `--allow-network-fs` or `CURIO_ALLOW_NETWORK_FS` if you know better.
- **`curio doctor` checks** for a left-behind pre-0.5 profile, whether you
  are on the shared default profile, and for undecodable staged events.
- **CLI exit status 75** when another Curio process keeps the profile busy,
  so cron wrappers can retry instead of alerting.
- **Smarter feed discovery** — adding a site finds its feed more often and
  only offers feeds that actually parse: scheme-less and `feed:` input,
  direct rewrites for Reddit, GitHub releases, YouTube channels and
  playlists, Mastodon, and Substack, and a small same-origin probe (at most
  six requests) when a page doesn't advertise a feed.

### Changed

- **`curio feed add` validates what you typed** — a site or page subscribes
  to its discovered feed, and a page with no feed is refused with a hint
  instead of subscribing a URL that fails on every refresh. `--no-validate`
  keeps the old behaviour.

- **One shared profile for the CLI and the app** — on macOS and Windows the
  CLI used a different default location, so `curio clip` never reached the
  app's library. Both now use the same profile; an old CLI-only or
  Roaming-AppData profile is moved over once on upgrade (never merged into an
  existing library, never moved while open). Only `curio init` creates a
  profile now, so a mistyped `--profile` no longer shows an empty library.
- **Safe multi-process profiles** — the app and a CLI cron job can work on
  the same profile at once: event emission is serialized by a profile lock,
  refreshes of one feed take turns across processes, and destinations have a
  single source of truth (`curio.toml` seeds them; it no longer undoes a
  removal made in the app). A busy profile is reported as a retryable,
  localized message instead of a raw SQLite error.
- **Durable storage commits** — the database now fsyncs every commit, so a
  power loss can no longer forget a change the event log already recorded.
- **Upgrades are all or nothing** — every pending migration runs in one
  transaction, so a failing step leaves the library at its old version
  instead of half-way.
- **Reddit credentials are per profile** — each profile keeps its own
  keychain item; existing credentials are copied over on first use.
- Internal: the storage, handle, export, and desktop UI stores were split
  into smaller modules with no behaviour change.

### Fixed

- **Full text no longer reverts on refresh** — an article hydrated with
  **Load full article** / **Load full post**, per-feed full text, or Reddit
  enrichment used to fall back to the feed's excerpt on the next refresh (and
  the next export pushed that excerpt to your notes). Hydrated bodies now
  stay, and unchanged
  entries are no longer rewritten at all.
- **Export never overwrites another article's note** — a note is only
  rewritten when its own `curio_id` matches; a path held by another article
  or by a non-Curio file gets the next free filename. Concurrent exports to
  one destination no longer drop each other's manifest entries.
- **Durable note writes** — exported notes are fsynced before and after the
  atomic rename, so a crash can't leave a blank note; an empty or
  NUL-filled note on disk is repaired on the next export.
- **Filenames that are safe on every OS** — titles are Unicode-normalized,
  stems capped by bytes so the longest dedupe suffix still fits, and Windows
  device names (`con`, `nul`, `com1`, …) are suffixed.
- **Durable event emission** — a write that fails part-way (disk full, I/O
  error) is rolled back so a retry never extends a torn line; a failed event
  flush no longer reports an already-saved change as failed (the event stays
  staged and is retried on the next tick); and one undecodable staged event
  is quarantined instead of stopping the whole profile from opening.
- **An unusable events folder no longer blocks opening the library** — the
  event log attaches lazily and catches up once it can.
- **Adding a feed no longer crashes on non-ASCII pages** — feed discovery
  panicked when its sniff window ended inside a multi-byte character (CJK,
  accents, emoji); it now cuts on a character boundary and recognizes feeds
  by their root element.
- **The sanitizer keeps more of the article** — lazy-loaded images,
  footnotes (exported as markdown `[^n]` footnotes), right-to-left text,
  code-fence languages, and embeds from Spotify, SoundCloud, CodePen,
  Bluesky, and Mastodon (kept as links) now survive ingest.
- **Messy OPML and CSV imports** — HTML entities, Latin-1 and UTF-16
  files, `feed://` and padded URLs now import; a bad row is skipped and
  reported instead of failing the whole file, and OPML and theme exports
  replace the target file atomically.
- **Main-content extraction no longer drops feed-item text** — intros, split
  sections, and quoted asides are kept.
- **Sloppy feed entries** — updated-only Atom entries get a date, a guid
  reused across links no longer collapses distinct items, future dates are
  clamped, relative lead images resolve, and a link-less entry is never
  "full-texted" against its feed's URL.

### Security

- **Patched RUSTSEC advisories**, most importantly in **ammonia**, the HTML
  sanitizer every feed and page passes through (4.1.3 → 4.2.4:
  RUSTSEC-2021-0157/0158, RUSTSEC-2026-0213/0373 — CSS `url()` escaping and
  SVG animation-tag XSS). Also h2, rustls, rustls-webpki, webpki-roots,
  tokio, and zip (via the updater plugin and tauri 2.12).
- **Reddit secrets are redacted** from debug output and logs — the client
  secret and live bearer token print as `<redacted>`.
- **Release hardening** — a tag only drafts a release after the full CI
  suite passes on the tagged commit; the updater signing key is isolated in a
  single step of a tag-restricted, reviewer-gated environment; every
  workflow action is SHA-pinned; and releases now ship `SHA256SUMS`,
  CycloneDX SBOMs, and a build-provenance attestation.
- **CI hardening** — cross-OS core tests, a CI-enforced MSRV (1.88), a
  nightly bundle and five-scenario Linux/Windows smoke as hard gates, a
  parseable cold-start bench, and macOS jobs moved off the retired
  `macos-14` image.

## [0.4.0] — 2026-07-13

The read-later release: Curio grows from an RSS reader into a full
read-it-later hybrid — save any link, pull full text for excerpt-only feeds,
export the whole library as markdown, and read complete Reddit posts.

### Added

- **Save any URL to Read Later** — paste a link in the add dialog and hit
  **Save to Read Later** (or `curio clip <url>`). The page is fetched and
  readability-extracted with its own title, author, and lead image; an
  unreachable page still saves the bare link, and re-saving a known URL
  re-flags it without refetching.
- **Per-feed full text** — a **Fetch full text** toggle (edit-feed dialog, or
  `curio feed full-text <feed> on|off`) hydrates each *new* article from its
  source page at refresh time, for feeds that ship only excerpts.
- **Export everything** — an **Export all** button per destination (and
  `curio save --all`, with `--read-later` / `--starred` / `--feed` / `--tag`
  filters) writes the whole library as markdown notes. Idempotent: re-running
  only rewrites what changed.
- **Full Reddit posts** — a **Load full post** action on Reddit items pulls
  the complete selftext, inline images, and galleries (in order) through the
  same sanitize-at-ingest gate as everything else.
- **Optional Reddit API credentials** — bring your own free Reddit app
  (Settings → Media & Privacy, or `curio reddit login`) to load full posts at
  the authenticated ~100 requests/min tier. The secret is stored in the
  system keychain, never in config or logs, and never leaves the device
  except to Reddit.
- **Instant first fetch** — a newly added feed loads its articles immediately
  instead of waiting for the next refresh cycle.

### Changed

- **Reddit request pacing is honest about the limits** — anonymous requests
  are spaced ~9/min (under Reddit's unauthenticated cap), authenticated API
  calls ~85/min. A rate-limit response opens a circuit breaker immediately
  (honoring `Retry-After`) instead of hammering, and surfaces a clear
  "try again in N min" message.
- **Anonymous Reddit full-post requests are blocked by Reddit** (their 2026
  API lockdown) — Curio now says so plainly and points at the credentials
  setting, instead of failing with an opaque error. Feeds themselves still
  work without credentials.
- PRIVACY.md discloses the new Reddit pacing and the opt-in authenticated
  mode.

### Removed

- **Arrow/Enter navigation** — the spatial pane navigation never worked well
  and fought native key handling. `j`/`k`/`n`, all letter shortcuts and `g`
  chords, Home/End/Page jumps, and Alt+↑/↓ feed reorder are unchanged;
  clicking into an article restores native scrolling.

## [0.3.0] — 2026-07-10

A reading-and-refinement release: new reader typography controls and paper
reading themes, faster keyboard-driven feed management, sharper search, and a
thorough folder, accessibility, and localization polish pass.

### Added

- **Reader typography** — new **font-weight**, **letter-spacing**, and
  **hyphenation** controls, plus **Sepia** and **Paper** reading themes (a warm
  paper ground with dark ink, independent of the app theme). The Settings
  preview now renders real headings, lists, inline code, and links, so what you
  see is what the reader paints.
- **Keyboard feed reordering** — reorder a feed within its folder with
  **Alt+↑ / Alt+↓**, no mouse required.
- **Drag a feed out of a folder** — a "Remove from folder" drop zone appears
  while dragging, and the sidebar auto-scrolls near its top/bottom edge.
- **Search-term highlighting** — matched query terms are highlighted in the
  result titles and snippets.
- **"New folder…"** directly in a feed's Move-to-folder menu.
- **`mailto:` and `tel:` links** in articles now open your mail client / dialer.
- **Arrow-key navigation** between panes is now documented in the `?` overlay.

### Changed

- **Honest feed health on launch** — a feed whose last refresh failed now shows
  its warning dot immediately on a cold start, not only after the first manual
  refresh.
- **Localized plurals** across all eight languages — counts like "1 feed" vs
  "2 feeds" (and the correct Polish forms) are now grammatical everywhere.
- The **Move-to-folder** menu no longer lists the feed's own folder, and hides
  "Remove from folder" for feeds that aren't in one.

### Fixed

- Renaming a folder onto an existing sibling no longer silently **merges** the
  two — the collision is refused and the old name kept.
- Empty folders show an "Empty folder" label instead of a blank gap.
- A long submenu that runs past the screen edge now flips to the other side
  instead of covering its parent menu.
- Feed titles that start with an emoji render a whole glyph in the monogram.
- YouTube grid cards line up regardless of title length.
- Double-clicking a folder or feed name to rename no longer re-scopes the
  article list first.

## [0.2.0] — 2026-07-09

A polish-and-power release: right-click everywhere, tabbed settings, source
presets, background refresh with notifications, native menus and hotkeys,
custom themes, and an in-app auto-updater.

### Added

- **Right-click context menus** on feeds, folders, and articles, with
  submenus, plus hover **tooltips** throughout the UI.
- **Source presets for adding feeds:** subscribe by `r/subreddit`,
  `@user@instance` (Mastodon), a YouTube channel, or a Hacker News feed, with
  **smart-input detection** and a **"Popular sources" quick-add** row.
- **Sidebar organization:** create / rename / delete **folders**, drag feeds
  into folders, **drag-reorder** feeds, inline rename, and persistent collapse
  state; a warning **dot** marks any feed whose last refresh errored. Plus
  **keyboard tree navigation** of the sidebar.
- **Edit-feed modal:** the feed URL is now visible (copy / open), with an
  editable name, site, and description, tag chips, and a feed-health section.
- **Tabbed Settings** — General, Appearance, Reading, Notifications,
  Media & Privacy, Data, and Advanced — surfacing toggles that were previously
  hidden.
- **Custom themes:** create, edit, and share your own themes, with **YAML
  theme export / import**; plus a **live typography preview** in Settings →
  Reading.
- **Background refresh:** automatic refresh on launch and a configurable
  interval (Off / 15 / 30 / 60 / 180 min).
- **Desktop notifications** on background refresh, with granular preferences —
  per-event toggles, quiet hours, and per-feed opt-out.
- **Native menu bar and keyboard hotkeys:** ⌘-chords for Settings (⌘,), add
  feed (⌘N), refresh (⌘R), and the smart lists (⌘1–4), plus a help overlay.
- **In-app auto-updater:** "Check for updates", one-click install and
  relaunch, and auto-check / auto-install toggles in Settings; updates are
  delivered as signed GitHub-release artifacts.
- **Reading polish:** list **density** (comfortable / compact), article
  **snippets** in the list, loading **skeletons**, friendly **empty states**,
  and safe **placeholders** for embedded media (a link in place of raw
  iframes / video).
- A new **macOS-squircle app icon**.

### Changed

- **Per-host fetch policy** so sources like Reddit `.rss` load reliably — a
  browser User-Agent plus a politeness delay for hosts that reject the honest
  default, disclosed in [PRIVACY.md](PRIVACY.md).
- **YouTube** now plays via a click-to-load facade and renders non-empty
  article bodies.

### Distribution

- Universal macOS `.dmg` from the GitHub release and the Homebrew cask
  (`brew install --cask alexnodeland/tap/curio`, which auto-serves the latest
  release). The app remains **unsigned / un-notarized by design** — open it
  once via right-click → Open (macOS 12–14) or System Settings → Privacy &
  Security → Open Anyway (macOS 15+).

## [0.1.0] — 2026-07-07

First public release — a complete, local-first desktop reader.

### Added

- **Reading core.** Subscribe to RSS/Atom feeds (with URL autodiscovery),
  refresh per-feed or all-at-once with health tracking and backoff, and read
  in a three-pane reader. On-demand **full-article readability** extraction,
  mark-as-read-on-open, opt-in **mark-as-read-on-scroll**, and **next-unread**
  navigation across feeds.
- **State**, event-sourced with negation events: read/unread, star,
  read-later, archive.
- **Folders** as `/`-path tags, rendered as a nested tree with a folder
  filter, **drag-to-reorder**, and move/rename.
- **Full-text search** (SQLite FTS5).
- **Import / export.** OPML in and out, plus refugee importers for **Pocket,
  Instapaper and Readwise** CSV exports (saved articles become read-later
  items with their tags).
- **Save to your notes.** Named destination directories and idempotent
  promotion of an article to a markdown note per the **`curio.frontmatter.v1`**
  contract, with a byte-preserved managed region.
- **Behavioral event log** (`curio.events.v1`): append-only JSONL under
  `.curio/events/`, never committed to git, built for downstream replay.
- **Appearance & reading:** 9 themes plus System, adjustable reading
  typography (font, size, line height, measure), and RSS-native Reddit /
  YouTube layouts (zero enrichment; nothing loads from the network until you
  click).
- **Accessibility:** modal focus-trap with an inert background, a skip link
  and `<main>` landmark, a listbox article list with `aria-activedescendant`
  and Arrow/Home/End navigation, labelled reader-toolbar controls, live-region
  toasts, and WCAG-AA text contrast across every theme (gated by a test).
- **Internationalization:** 8 fully translated locales — English, Español,
  Français, Deutsch, Italiano, Polski, 简体中文 (Simplified Mandarin), and
  廣東話 (Cantonese) — with English fallback and full-coverage gating.
- **Privacy by default:** no telemetry, no phone-home; the only outbound
  traffic is fetching the feeds you subscribed to. Remote media and the
  Google favicon fallback are opt-in.

### Distribution

- Unsigned macOS Homebrew cask (`brew install --cask alexnodeland/tap/curio`)
  and a downloadable `.dmg` from the GitHub release. The app is **not signed
  or notarized** — open it once via right-click → Open (or System Settings →
  Privacy & Security → Open Anyway on macOS 15+). Windows and Linux bundles
  are built nightly and available as artifacts, but are not yet a shipped
  channel.

[0.5.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.5.0
[0.4.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.4.0
[0.3.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.3.0
[0.2.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.2.0
[0.1.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.1.0
