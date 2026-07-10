# Changelog

All notable changes to the Curio app. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project uses
[semantic versioning](https://semver.org/spec/v2.0.0.html).

(The versioned Curio↔notes **contract** schemas have their own changelog at
[`schemas/CHANGELOG.md`](schemas/CHANGELOG.md).)

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

[0.2.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.2.0
[0.1.0]: https://github.com/alexnodeland/curio-rss/releases/tag/v0.1.0
