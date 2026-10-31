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
  The latest *scheduled* run must be on a commit at or after the last change
  to the nightly's plumbing — a PR run of the nightly proves the change, the
  schedule proves `main`. With no fresh run, dispatch one (Actions → nightly →
  Run workflow).
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
   `latest.json`. On a tag push the job deploys to the **`updater-signing`
   environment** (below), so it **waits for your approval**: open the run →
   **Review deployments** → tick `updater-signing` → **Approve and deploy**.
   The key step fails fast with a pointer here if the key comes through empty.
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
`GITHUB_ENV`/`GITHUB_PATH`, a signing job that drops its `updater-signing`
environment (or selects it under any condition other than the key step's own
`if:`), or a missing checksum/SBOM/attestation asset.

### The `updater-signing` environment (one-time setup)

The updater signing key signs every update every user installs, so once the
setup below is done it is **not** a repository secret: it lives only as a
secret of the GitHub environment `updater-signing`, which admits `v*` tags
alone and waits for a maintainer's approval. A workflow on any branch — even
one with write access that references `secrets.TAURI_SIGNING_PRIVATE_KEY` —
then reads it as empty, and a sign job nobody approved never starts.
`release.yml` selects the environment only on a tag push (on the
`workflow_dispatch` dry run the name is empty, i.e. no environment, so the dry
run is not held at the tag-only rule).

**Order matters.** Do steps 1–4 **before the next `v*` tag is pushed** —
ideally before the `release.yml` that references the environment merges.
GitHub silently auto-creates an environment that a job references but that
does not exist yet, and creates it with **no** protection rules: a tag cut in
that window would deploy to an unprotected `updater-signing` and sign with the
still-present repository secret, unapproved. If `updater-signing` already
exists when you start (auto-created, or from an earlier attempt), open it and
check its protection rules and deployment tag rule against steps 2–3 before
you tag. Step 5 comes last, after the environment holds the secrets.

Setup (repository admin, in the GitHub web UI):

1. **Settings → Environments → New environment**, name `updater-signing`
   (exactly), **Configure environment**.
2. **Deployment protection rules**: tick **Required reviewers**, add the
   maintainer(s). Leave **Prevent self-review** unticked if you are the only
   maintainer (you must be able to approve your own tag), and leave **Allow
   administrators to bypass configured protection rules** unticked.
   **Save protection rules**.
3. **Deployment branches and tags**: choose **Selected branches and tags** →
   **Add deployment branch or tag rule** → Ref type **Tag**, name pattern `v*`
   → **Add rule**. Add no branch rule.
4. **Environment secrets → Add environment secret**: `TAURI_SIGNING_PRIVATE_KEY`
   (the minisign private key, the full contents of the `.key` file — GitHub
   never shows an existing secret's value, so paste it from your offline
   copy), then `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (its password).
5. Once a `release.yml` that selects the environment is on `main`:
   **Settings → Secrets and variables → Actions → Repository secrets** →
   delete `TAURI_SIGNING_PRIVATE_KEY` and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.
   (Deleting them earlier would leave a tag cut in between with an empty key.)
6. Optional hardening: **Settings → Rules → Rulesets → New ruleset → New tag
   ruleset**, target `v*`, enable **Restrict creations** / **Restrict
   deletions** with only the maintainer on the bypass list — the tag rule above
   checks the ref's *name*, so this limits who can mint such a ref at all.

Verify (after any change to the environment, and once after the setup):

- [ ] The dry run below is green from `main` and its `sign` job shows no
      environment and no approval prompt.
- [ ] A scratch branch whose workflow runs `test -z "$K"` with
      `K: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}` in a step's env passes
      (the key reads as empty off a tag); a job there declaring
      `environment: updater-signing` is refused ("not allowed to deploy …
      due to environment protection rules"). Delete the branch afterwards.
- [ ] **Settings → Secrets and variables → Actions** lists no repository-level
      `TAURI_SIGNING_*` secret.

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

- [ ] Approve the `sign` job's `updater-signing` deployment (the run waits
      there after `build`) — check that the run is for your tag first.
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
