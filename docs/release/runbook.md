# Release runbook

How a Curio release is cut. Distribution is an **unsigned** macOS Homebrew cask
(Apple enrollment skipped, Windows deferred — docs/design/decisions.md). Two
automated tiers back this: the per-PR gate (`ci.yml`) and the merge/nightly
tier (`nightly.yml`: the 3-OS `tauri build` matrix, the Linux/Windows smoke,
and the cold-start bench). macOS correctness is signed off by hand via
[`macos-checklist.md`](macos-checklist.md).

## 0. Preconditions

- `main` is green on CI and on the latest nightly `bundle` + `bench` runs.
- No open release blockers; `CHANGELOG` drafted.

## 1. Version bump

- [ ] Bump `version` in `Cargo.toml` (`[workspace.package]`), `apps/desktop/package.json`, and `apps/desktop/src-tauri/tauri.conf.json` to `X.Y.Z` (keep them in lockstep).
- [ ] Update `CHANGELOG` for `vX.Y.Z`.
- [ ] PR the bump; merge on green.

## 2. Tag

```sh
git checkout main && git pull
git tag -a vX.Y.Z -m "Curio vX.Y.Z"
git push origin vX.Y.Z
```

## 3. Build the bundles

The nightly `bundle` matrix builds all four targets unsigned; use its
artifacts, or build macOS locally for the manual checklist:

```sh
cd apps/desktop && npm ci && npm run tauri build
# -> target/release/bundle/{macos/Curio.app, dmg/Curio_X.Y.Z_<arch>.dmg}
```

- [ ] arm64 DMG built (macos-14 / Apple silicon).
- [ ] x64 DMG built (macos-13 / Intel).
- [ ] Linux `.deb`/`.AppImage` and Windows `.msi`/`.exe` available from the nightly artifacts (informational; not the shipped channel yet).

## 4. macOS manual verification

- [ ] Complete every box in [`macos-checklist.md`](macos-checklist.md) on the built `.app` (both arches if available).

## 5. Publish the GitHub release

- [ ] Create the `vX.Y.Z` GitHub release from the tag.
- [ ] Rename + upload the DMGs under the cask's stable names: `Curio-arm64.dmg`, `Curio-x64.dmg` (these resolve `releases/latest/download/Curio-<arch>.dmg`).
- [ ] Paste the `CHANGELOG` section into the release notes; note "unsigned — right-click → Open on first launch".

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
