# Changelog

All notable changes to the Curio app. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

(The versioned Curio↔notes **contract** schemas have their own changelog at
[`schemas/CHANGELOG.md`](schemas/CHANGELOG.md).)

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

[0.1.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.1.0
