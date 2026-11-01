# macOS release checklist (manual)

macOS has no `tauri-driver`, so its correctness is verified **by hand** against
this checklist for every release — it is the macOS counterpart to the
automated Linux/Windows smoke (`apps/desktop/e2e`). Copy the checklist block
below into the release PR / tag notes and tick every box on the actual build
before publishing.

- **Version:** `vX.Y.Z`
- **Commit:** `<sha>`
- **Builder / host:** `<name> — macOS <version>, <arm64|x64>`
- **Build type:** unsigned (Apple enrollment skipped)

## Build

- [ ] Working tree clean, on the tag commit; submodules populated (`git submodule status` shows no `-`/`+`).
- [ ] `just ci` green locally.
- [ ] `cd apps/desktop && npm ci && npm run tauri build` completes.
- [ ] `target/release/bundle/macos/Curio.app` exists and launches from Finder.
- [ ] `target/release/bundle/dmg/Curio_<ver>_<arch>.dmg` produced.

## The five smoke scenarios (do each by hand on the built .app)

Run against a **throwaway profile** (`CURIO_PROFILE=$(mktemp -d) open -a Curio`)
and a **local** feed only (127.0.0.1 / a file:// or local server — never a
production feed).

- [ ] **Boot** — the app opens to the three-pane reader shell; no console errors.
- [ ] **Add feed** — add a local test feed; it appears in the sidebar with an unread count.
- [ ] **Stored-XSS inert** — open an article built from `fixtures/html/xss-corpus.html`; confirm no alert/script runs and the content renders sanitized (via `SanitizedHtml`).
- [ ] **Reader body in WebKit** — with Settings → media prefetch on, open a local-feed article carrying a 12-column table, footnotes and a few images (one with a dead URL): the table scrolls sideways inside the column (trackpad and, once focused with Tab, the arrow keys); clicking a footnote number scrolls smoothly to the note (no browser opens) and its `↩` scrolls back; images below the fold only appear in the image cache (`~/Library/Caches/curio/images/`) after scrolling near them; the dead image and its caption leave no gap; clicking a loaded image opens the lightbox, Escape closes it without deselecting the article, and focus is back on the image.
- [ ] **Keyboard-only session** — navigate feeds/articles and read one end-to-end with only the keyboard (`j`/`k`, `o`, `s`, `l`, `m`, `p`, `/`, `?`).
- [ ] **Promote-to-destination** — press `p`, pick a temp destination, confirm a Markdown note with valid `curio.frontmatter.v1` frontmatter is written.
- [ ] **APFS filenames** — on an APFS destination, promote two local-feed articles titled "Café", one spelled NFC (precomposed U+00E9) and one NFD (`e` + combining U+0301): two distinct notes exist afterwards (`café.md` and `café-<id prefix>.md`) and the first one's text was not replaced.

- [ ] **Per-profile Reddit credentials (real Keychain)** — `curio --profile /tmp/a reddit login --client-id a` and `curio --profile /tmp/b reddit login --client-id b` (any secrets); Keychain Access shows two `Curio` items with accounts `reddit-api/<key>`, `reddit status` reports `a` / `b` per profile, and `reddit logout` on one leaves the other configured. Moving a profile (`mv /tmp/a /tmp/a2`) keeps its credentials. Upgrading from 0.4.x: the old single `reddit-api` item is copied into each profile that opens (both the CLI and the desktop app come up authenticated) and stays in Keychain Access until a `reddit logout`.

## Startup recovery (do each on the built .app)

A bundled app has no visible stderr, so a profile that won't open must land
on the recovery screen, never a bouncing icon that vanishes. Use a
throwaway profile — `open -a` starts the app through LaunchServices, which
does not pass on your shell's environment, so hand the variable over with
`--env` (and `export` it so the later steps can use it):

```sh
export CURIO_PROFILE="$(mktemp -d)"
open --env CURIO_PROFILE="$CURIO_PROFILE" -a Curio   # quit it, then break the profile
# or run the one executable inside the bundle, which inherits the environment:
# /Applications/Curio.app/Contents/MacOS/*
```

Relaunch the same way after each step, and check that the profile the app
reports (Settings, or the recovery screen's error details) is the temp
directory, never your real one.

- [ ] **Corrupt DB → recovery screen** — `printf 'garbage, not sqlite' > "$CURIO_PROFILE/curio.db"`, relaunch: the window opens on "Curio can't read its database" (no crash, no silent exit), and `logs/curio.log.*` in the profile has an `ERROR … startup failed` line naming the class.
- [ ] **Start fresh** — on that screen, Start fresh → confirm: the app opens into an empty reader after "Open Curio", and the garbage file sits untouched in `set-aside/<timestamp>/curio.db`.
- [ ] **Restore backup** — with a profile from the previous release (so the upgrade wrote `curio.db.pre-vN.bak`), corrupt `curio.db` again and relaunch: "Restore backup" is offered, and restoring brings back the feeds.
- [ ] **Too-new DB** — `sqlite3 "$CURIO_PROFILE/curio.db" 'PRAGMA user_version = 999'`: the screen says the database is from a newer Curio and leads with Check for updates (which reports no newer version, or installs one).
- [ ] **Unwritable event log is not a startup failure** — on a healthy profile, `chmod 000 "$CURIO_PROFILE/.curio/events"` and relaunch: the library opens with its feeds, no recovery screen. Star an article and quit: `curio --profile "$CURIO_PROFILE" doctor` reports the star as a staged intent. `chmod 755` it, relaunch, and the star event is in the log.
- [ ] **Profile folder can't be created** — `export CURIO_PROFILE="$(mktemp -d)/blocked" && touch "$CURIO_PROFILE"`, launch with `--env`: the screen says Curio can't use its profile folder and offers only Try again (no Restore, no Start fresh); `rm "$CURIO_PROFILE"`, Try again, and an empty library opens.
- [ ] **Show profile folder / Copy error / Quit** — Finder opens on `curio.db`; the clipboard holds the error report; Quit exits.
- [ ] **Keychain off the launch path** — with Reddit credentials stored, an unsigned build's keychain access prompt (if any) appears *after* the window is up and does not block it.

## Background refresh: sleep/wake and offline (real macOS only — CI can't suspend a machine or drop its network)

Use a throwaway profile with a few real public feeds, the refresh interval
at 15 minutes, and notifications on (Settings → General / Notifications).

- [ ] **Catches up after sleep** — refresh, then close the lid (or Apple menu → Sleep) for longer than the interval. Within about a minute of waking, a refresh runs on its own (the refresh pill appears, `logs/curio.log.*` has a `woke from sleep` line), not up to a full interval later.
- [ ] **Remembers the last refresh** — with Refresh on launch off, quit and relaunch more than one interval after the last refresh: a refresh runs within about a minute. Relaunch again straight away: none runs until the interval is up.
- [ ] **Offline is quiet** — turn Wi-Fi off and wait for a scheduled refresh (or relaunch with Refresh on launch on): the sidebar's brand tile turns into the amber "Offline" badge, no feed gets a new error in its health log, and no "feeds failed to refresh" notification appears.
- [ ] **Back online** — turn Wi-Fi on: within about a minute the badge clears and the overdue refresh runs. A manual Refresh while still offline does not cancel it.
- [ ] **Launch-only, started offline** — interval set to Off, Refresh on launch on: launch with Wi-Fi off (the badge appears), then turn Wi-Fi on: within about a minute the skipped launch refresh runs.
- [ ] **Badge by keyboard** — while offline, Tab to the badge: the "No network — background refresh is paused" hint appears with a focus ring.

## Network-filesystem guard (real macOS only — CI has no iCloud account or SMB server)

- [ ] **iCloud Drive refused (CLI)** — `curio --profile ~/Library/Mobile\ Documents/com~apple~CloudDocs/curio-test init` fails with "is on iCloud Drive" and creates nothing. With `--allow-network-fs` (or `CURIO_ALLOW_NETWORK_FS=1`) it opens.
- [ ] **iCloud Drive refused (app)** — `open --env CURIO_PROFILE="$HOME/Library/Mobile Documents/com~apple~CloudDocs/curio-test" -a Curio` (`open -a` alone does not pass the variable on): the window opens on "Your library is on a network or synced folder", offering only Try again (no Restore, no Start fresh, even with a backup there), and the error details name `CURIO_ALLOW_NETWORK_FS`. Quit, then relaunch with `--env CURIO_ALLOW_NETWORK_FS=1` added: the library opens. Delete the folder afterwards.
- [ ] **SMB share refused** — mount any SMB share in Finder (`⌘K`, `smb://…`), then `curio --profile /Volumes/<share>/curio-test init` fails with "is on an SMB/CIFS network share"; the override opens it.
- [ ] **Local disk unaffected** — the default profile and `CURIO_PROFILE=$(mktemp -d)` open with no warning.

## Shared default profile (real macOS only — CI can't point the app at a temp `~/Library`)

Use a throwaway macOS user account, or move your real
`~/Library/Application Support/io.github.alexnodeland.curio` and
`~/Library/Application Support/curio` aside first.

- [ ] **One library** — with no `CURIO_PROFILE`, `curio clip <local-url>` then launch the app: the clip is in Read Later. `curio doctor` reports the profile as `~/Library/Application Support/io.github.alexnodeland.curio (the default, shared with the desktop app)`.
- [ ] **Legacy CLI profile is adopted** — with only an old-CLI profile at `~/Library/Application Support/curio` (made with `curio --profile ~/Library/Application\ Support/curio init` + one `clip`), the first `curio list` (or app launch) prints/logs "moved the profile…", and the clip is visible in the app.
- [ ] **Legacy profile never merged** — with both directories holding a `curio.db`, every `curio` command warns about the old one, `curio doctor` fails `legacy-profile`, the app shows a "left at…" notice (and the reason under Settings › Advanced), and neither directory is modified.
- [ ] **A legacy library last used by the app still moves** — quit a pre-0.5 app that used the old location (it leaves `curio.db-wal` behind), then start the new CLI or app: the library moves, and its last writes are there.
- [ ] **An open legacy library is never stranded** — with an old-location library held open by a running pre-0.5 `curio` (e.g. a long `fetch`), `curio list` warns "another curio process has it open" and exits 1 without creating the shared directory; the new app opens the old library in place and says so. After the old process quits, the next start moves it.

## Gatekeeper (unsigned) sanity

- [ ] Fresh-install path works: right-click → Open (or `xattr -dr com.apple.quarantine`) launches the app.
- [ ] `caveats` text in `dist/homebrew/Casks/curio.rb` still matches the actual first-run experience.

## Publish

`release.yml` builds the unsigned universal DMG, renames it to
`Curio-universal.dmg`, and drafts the release automatically on the tag — so
this is verification, not manual assembly.

- [ ] The draft `vX.Y.Z` release has `Curio-universal.dmg` attached (it backs `releases/latest/download/Curio-universal.dmg`).
- [ ] The draft also carries `SHA256SUMS`, the `*.cdx.json` SBOMs and `Curio-provenance.sigstore.json`; `shasum -a 256 -c SHA256SUMS --ignore-missing` and `gh attestation verify Curio-universal.dmg --repo alexnodeland/curio-rss` both pass on the downloaded DMG.
- [ ] Publish the draft, then `brew update && brew install --cask alexnodeland/tap/curio` installs it on a clean machine.
- [ ] **In-app update** — on a machine running the *previous* release, Settings → Check for updates finds `vX.Y.Z`, installs it (the signature on the CI-tarred `Curio.app.tar.gz` verifies), relaunches, and the app reports the new version. (The updater archive is signed in its own CI job since #174; this is the end-to-end proof the signature and archive layout are right.)
- [ ] `docs/release/runbook.md` steps all completed.

## Acceptance (Phase 4 gate)

- [ ] Cold start to interactive < 1.5s on the 50k fixture (the nightly `bench` job number, or `just bench-cold-start` locally).
- [ ] Stored-XSS inert end-to-end (scenario above).
- [ ] A complete keyboard-only reading session is possible (scenario above).
- [ ] Zero hand-written IPC (`bash scripts/check-frontend-bans.sh` green).
