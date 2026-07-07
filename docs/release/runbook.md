# Release runbook

How a Curio release is cut. Distribution is an **unsigned** macOS Homebrew cask
(Apple enrollment skipped, Windows deferred — docs/design/decisions.md). Three
workflows back this: the per-PR gate (`ci.yml`), the nightly/on-demand tier
(`nightly.yml`: the 3-OS `tauri build` matrix, the Linux/Windows smoke, and the
cold-start bench), and the tag-triggered `release.yml` (builds the unsigned
macOS **universal** DMG, renames it to the cask's stable name, and drafts the
GitHub release). macOS correctness is signed off by hand via
[`macos-checklist.md`](macos-checklist.md).

## 0. Preconditions

- `main` is green on CI and on the latest nightly `bundle` + `bench` runs.
- No open release blockers; `CHANGELOG` drafted.

## 1. Version bump

- [ ] Bump `version` in `Cargo.toml` (`[workspace.package]`), `apps/desktop/package.json`, and `apps/desktop/src-tauri/tauri.conf.json` to `X.Y.Z` (keep them in lockstep).
- [ ] Update `CHANGELOG` for `vX.Y.Z`.
- [ ] PR the bump; merge on green.

## 2. Tag → automated draft release

Pushing the tag triggers [`release.yml`](../../.github/workflows/release.yml):
it builds the unsigned macOS **universal** DMG (arm64 + x64 in one, on a single
Apple-silicon runner — avoids the scarce Intel-runner queue), renames it to the
cask's stable name (`Curio-universal.dmg`), and creates a **draft** GitHub
release with it attached and the CHANGELOG section + unsigned banner as the
notes. Nothing is public — and `brew` sees nothing — until you publish the
draft in §5.

```sh
git checkout main && git pull
git tag -a vX.Y.Z -m "Curio vX.Y.Z"
git push origin vX.Y.Z          # → release.yml drafts the release
```

- [ ] `release.yml` run for the tag is green and a **draft** `vX.Y.Z` release
      exists with `Curio-universal.dmg` attached.

## 3. (Optional) other-platform bundles

Linux `.deb`/`.AppImage` and Windows `.msi`/`.exe` are built by the nightly
`bundle` matrix and downloadable as run artifacts (informational; not the
shipped channel yet). Attach them to the draft only if you want them surfaced.

## 4. macOS manual verification

Download the drafted DMGs (or build locally: `cd apps/desktop && npm ci &&
npm run tauri build`).

- [ ] Complete every box in [`macos-checklist.md`](macos-checklist.md) against
      the `.app` from each DMG (both arches if available).

## 5. Publish the draft

- [ ] Confirm the notes read well (CHANGELOG section + unsigned/Gatekeeper
      banner are auto-inserted by `release.yml`).
- [ ] Click **Publish release** — this makes it the `latest`, so
      `releases/latest/download/Curio-<arch>.dmg` resolves and the cask installs.

## 6. Publish the Homebrew cask

Follow [`dist/homebrew/README.md`](../../dist/homebrew/README.md): copy
`dist/homebrew/Casks/curio.rb` into `alexnodeland/homebrew-tap` under `Casks/`
and push. Because the cask is `version :latest` + `sha256 :no_check`, a routine
release needs **no** formula edit — only re-copy if the URL scheme, app name,
or uninstall footprint changed.

- [ ] Tap updated (only if the template changed).
- [ ] `brew update && brew install --cask curio` installs `vX.Y.Z` on a clean machine.

## 7. Close out

- [ ] Release notes link the nightly `bench` result (cold-start < 1.5s @50k).
- [ ] Announce; open the milestone for the next version.

## Rollback

Unsigned + `version :latest` means the tap always points at the newest
release. To roll back, delete or unpublish the bad GitHub release (or publish a
superseding `vX.Y.(Z+1)`); `releases/latest/download/...` then resolves to the
previous good build with no cask change.
