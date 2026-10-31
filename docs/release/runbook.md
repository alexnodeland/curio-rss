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

- `main` is green on CI and on the latest nightly `bundle` + `smoke` + `bench` runs
  (the smoke is a hard gate: all five Linux/Windows scenarios, none skipped).
- No open release blockers; `CHANGELOG` drafted.

## 1. Version bump

- [ ] Bump `version` in `Cargo.toml` (`[workspace.package]`), `apps/desktop/package.json`, and `apps/desktop/src-tauri/tauri.conf.json` to `X.Y.Z` (keep them in lockstep).
- [ ] Update `CHANGELOG` for `vX.Y.Z`.
- [ ] PR the bump; merge on green.

## 2. Tag → automated draft release

Pushing the tag triggers [`release.yml`](../../.github/workflows/release.yml):

1. **`gate`** re-runs the whole CI suite (`ci.yml`, called as a reusable
   workflow) on the tagged commit. Every other job `needs` it, so a tag on a
   commit with red CI produces **no** draft release.
2. **`build`** builds the macOS **universal** DMG (arm64 + x64 in one, on a
   single Apple-silicon runner — avoids the scarce Intel-runner queue) with
   **no secrets at all**: Apple notarization is skipped and the updater
   archive (`Curio.app.tar.gz`) is tarred unsigned.
3. **`sign`** is the only job that sees `TAURI_SIGNING_PRIVATE_KEY`, in one
   step that runs nothing but the prebuilt `tauri signer sign` (installed with
   `npm ci --ignore-scripts` from the lockfile — no build scripts, no vite
   plugins, no npm lifecycle scripts). It writes the updater manifest
   `latest.json`.
4. **`sbom`** writes CycloneDX SBOMs: the Rust graph per darwin arch and the
   whole npm lockfile (dev dependencies included — the build toolchain that
   compiled the frontend bundle).
5. **`release`** writes `SHA256SUMS` over every asset, mints a build-provenance
   attestation for them (`actions/attest-build-provenance`), and creates a
   **draft** GitHub release with the DMG (under the cask's stable name,
   `Curio-universal.dmg`), the updater archive + `.sig`, `latest.json`,
   `SHA256SUMS`, the SBOMs and the attestation bundle attached, and the
   CHANGELOG section + unsigned banner as the notes.

Nothing is public — and neither `brew` nor the in-app updater sees anything —
until you publish the draft in §5. `cargo run -p xtask -- workflows` (`just
workflows`, a CI job) fails the build if this shape regresses: an unpinned
action, a job that skips the gate (or runs on `always()` / `failure()` /
`cancelled()` past it), the signing key in a second step, anything ahead of
the key step that runs project code, backgrounds a process or writes
`GITHUB_ENV`/`GITHUB_PATH`, or a missing checksum/SBOM/attestation asset.

**Dry run first (recommended).** Actions → `release` → **Run workflow** on
`main` runs all of the above except the publish steps: it signs with a
throwaway key generated on the runner, mints no attestation, drafts no
release, and uploads the would-be assets as the `release-dry-run` run
artifact. Use it after any change to `release.yml`, or before tagging.

```sh
git checkout main && git pull
git tag -a vX.Y.Z -m "Curio vX.Y.Z"
git push origin vX.Y.Z          # → release.yml drafts the release
```

- [ ] `release.yml` run for the tag is green and a **draft** `vX.Y.Z` release
      exists with `Curio-universal.dmg`, `Curio.app.tar.gz(.sig)`,
      `latest.json`, `SHA256SUMS`, the three `*.cdx.json` SBOMs and
      `Curio-provenance.sigstore.json` attached.
- [ ] Download the DMG + `SHA256SUMS` and verify both:
      `shasum -a 256 -c SHA256SUMS --ignore-missing` and
      `gh attestation verify Curio-universal.dmg --repo alexnodeland/curio-rss`.
- [ ] `tar -tzf Curio.app.tar.gz | head` lists `Curio.app/…` entries (the
      updater strips that first path component on install) and no `._*`
      AppleDouble files.

## 3. (Optional) other-platform bundles

Linux `.deb`/`.rpm`/`.AppImage`, Windows `.msi`/`.exe` and a macOS universal
`.dmg` are built by the nightly `bundle` matrix (updater artifacts off — only
`release.yml` holds the updater signing key) and downloadable as run artifacts (informational; not the
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
