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
- [ ] **Keyboard-only session** — navigate feeds/articles and read one end-to-end with only the keyboard (`j`/`k`, `o`, `s`, `l`, `m`, `p`, `/`, `?`).
- [ ] **Promote-to-destination** — press `p`, pick a temp destination, confirm a Markdown note with valid `curio.frontmatter.v1` frontmatter is written.

## Gatekeeper (unsigned) sanity

- [ ] Fresh-install path works: right-click → Open (or `xattr -dr com.apple.quarantine`) launches the app.
- [ ] `caveats` text in `dist/homebrew/Casks/curio.rb` still matches the actual first-run experience.

## Publish

`release.yml` builds the unsigned universal DMG, renames it to
`Curio-universal.dmg`, and drafts the release automatically on the tag — so
this is verification, not manual assembly.

- [ ] The draft `vX.Y.Z` release has `Curio-universal.dmg` attached (it backs `releases/latest/download/Curio-universal.dmg`).
- [ ] Publish the draft, then `brew update && brew install --cask alexnodeland/tap/curio` installs it on a clean machine.
- [ ] `docs/release/runbook.md` steps all completed.

## Acceptance (Phase 4 gate)

- [ ] Cold start to interactive < 1.5s on the 50k fixture (the nightly `bench` job number, or `just bench-cold-start` locally).
- [ ] Stored-XSS inert end-to-end (scenario above).
- [ ] A complete keyboard-only reading session is possible (scenario above).
- [ ] Zero hand-written IPC (`bash scripts/check-frontend-bans.sh` green).
