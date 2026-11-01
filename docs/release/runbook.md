# Release runbook

How a Curio release is cut. Distribution is an **unsigned** macOS Homebrew cask
(Apple enrollment skipped, Windows deferred — docs/design/decisions.md). Four
workflows back this: the per-PR gate (`ci.yml`), the nightly/on-demand tier
(`nightly.yml`: the 3-OS `tauri build` matrix, the Linux/Windows smoke, and the
cold-start bench), the tagger (`release-tag.yml`: tags a merged version bump
and dispatches the release on that tag), and `release.yml`, which runs on the
tag (builds the unsigned macOS **universal** DMG, renames it to the cask's
stable name, and drafts the GitHub release). macOS correctness is signed off
by hand via [`macos-checklist.md`](macos-checklist.md).

**In short:** merge the version-bump PR → approve the `sign` job's
`updater-signing` deployment → publish the draft. Nobody pushes a tag by hand
unless the tagger refuses (§2, *Fallback*).

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
- [ ] Update `CHANGELOG` for `vX.Y.Z` — its `## [X.Y.Z]` section must lead
      the file in the bump commit itself (the tagger refuses otherwise, and
      the release notes are that first section).
- [ ] PR the bump; merge on green. **Merging is the release decision.**

## 2. Merge → tag → automated draft release

On every push to `main`,
[`release-tag.yml`](../../.github/workflows/release-tag.yml) reads
`[workspace.package] version`. When `vX.Y.Z` is already tagged it does nothing
(the usual push, and every re-run). Otherwise its read-only `plan` job
(`cargo run -p xtask -- release-tag`) finds the commit that introduced the
version — the bump PR's squash-merge, even when later commits have landed — and
its `tag` job, which checks nothing out, creates the annotated tag `vX.Y.Z`
there with `GITHUB_TOKEN` (message: `Curio X.Y.Z` plus the rest of the
CHANGELOG heading) and dispatches `release.yml` on the tag ref. The tag is
created through the API's create-ref call, which refuses a ref that exists, so
an existing tag is never moved or deleted. Forks never run it.

Why a dispatch: a tag created with `GITHUB_TOKEN` fires no `push: tags` event
(GitHub suppresses runs its own token triggers), but `workflow_dispatch` is the
documented exception. A dispatch on the tag ref keeps `github.ref =
refs/tags/vX.Y.Z`, and `release.yml` decides "real release vs dry run" by that
ref alone, so the dispatched run is a real release exactly like a tag push:
same `gate`, same `updater-signing` approval.

The plan **fails instead of tagging** (an `::error::` on the `release-tag` run)
when:

- the bump commit's `CHANGELOG.md` does not lead with `## [X.Y.Z]`; or
- the bump commit's own `release.yml` would not release a tag-ref dispatch for
  real. GitHub runs the workflow file **at the tag**, and a `release.yml` that
  decides by the event name (every version up to and including 0.5.0) treats
  any dispatch as a dry run. Tagging from here would then only dry-run, and the
  tag would keep a hand-pushed one from ever triggering the real release. The
  error prints the exact fallback commands.

**Fallback (manual tag push).** When the tagger refuses, or Actions is
unavailable, push the annotated tag by hand from a machine with tag-push
rights (Claude Code cloud sessions cannot: their git proxy refuses tag pushes).
A tag push runs the same `release.yml`; the tagger's next run sees the tag and
does nothing.

```sh
git fetch origin && git checkout <bump-commit-sha>
git tag -a vX.Y.Z -m "Curio X.Y.Z — <CHANGELOG heading date/title>"
git push origin vX.Y.Z          # → release.yml drafts the release
```

Either way, [`release.yml`](../../.github/workflows/release.yml) then runs on
the tag:

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
   `latest.json`. On a tag ref (a tag push or the tagger's dispatch on the
   tag) the job deploys to the **`updater-signing`
   environment** (below), so it **waits for your approval**: open the run →
   **Review deployments** → tick `updater-signing` → **Approve and deploy**.
   The key step fails fast with a pointer here if the environment is not
   configured (its `UPDATER_SIGNING_READY` variable is missing) or the key
   comes through empty.
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
`if:`), a real-key step guarded by anything but the tag ref (or any
`github.event_name` in `release.yml` — a dispatch on the tag must release for
real in every step), or a missing checksum/SBOM/attestation asset. For the
tagger it fails on any trigger but `push` to `main`, a missing fork guard, any
secret but `GITHUB_TOKEN`, an environment, a write grant on a job that checks
out code or runs a build tool, a command that moves or deletes a ref, or a
missing create-ref call or tag-ref dispatch — and it fails any **other**
workflow that creates a tag.
It holds the nightly tier §0 relies on too: no retired runner image, no
`continue-on-error`, bundle uploads that fail when empty, and a cold-start
`bench` the tracker can read — criterion run statelessly (`CRITERION_HOME`
under `runner.temp`, `--discard-baseline`) under `pipefail`, its log checked
by `cargo run -p xtask -- bench-output` before `github-action-benchmark`
reads it, and the 130% alert kept as a hard failure (`fail-on-alert: true`).
`just bench-cold-start-ci` runs that bench + check locally.

### The `updater-signing` environment (one-time setup)

The updater signing key signs every update every user installs, so once the
setup below is done it is **not** a repository secret: it lives only as a
secret of the GitHub environment `updater-signing`, which admits `v*` tags
alone and waits for a maintainer's approval. A workflow on any branch — even
one with write access that references `secrets.TAURI_SIGNING_PRIVATE_KEY` —
then reads it as empty, and a sign job nobody approved never starts.
`release.yml` selects the environment only on a tag ref — a tag push or the
tagger's dispatch on the tag (on the `workflow_dispatch` dry run from `main`
the name is empty, i.e. no environment, so the dry run is not held at the
tag-only rule).

**Order matters.** Do steps 1–4 **before the next `v*` tag is created** —
i.e. before the next version-bump PR merges, since the tagger tags it.
GitHub silently auto-creates an environment that a job references but that
does not exist yet, and creates it with **no** protection rules. So the key
step first checks the `UPDATER_SIGNING_READY` variable, which exists only on
the environment you configure (step 4): a tag cut before the setup fails
there with `::error::the updater-signing environment is not configured`
instead of signing unapproved with the still-present repository secret.
Never define `UPDATER_SIGNING_READY` as a repository or organization
variable — those reach every job and would defeat the check. If
`updater-signing` already exists when you start (auto-created by such a
failed run, or from an earlier attempt), open it and check its protection
rules and deployment tag rule against steps 2–3 before you tag. Step 5 comes
last, after the environment holds the secrets.

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
   copy), then `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` (its password). Then
   **Environment variables → Add environment variable**:
   `UPDATER_SIGNING_READY` = `true` — add it last, once steps 2–3 are saved;
   it is what tells the release run the environment is ready.
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
      `TAURI_SIGNING_*` secret and no repository-level `UPDATER_SIGNING_READY`
      variable.

**Dry run first (recommended).** Actions → `release` → **Run workflow** on
`main` runs all of the above except the publish steps: it signs with a
throwaway key generated on the runner, mints no attestation, drafts no
release, and uploads the would-be assets as the `release-dry-run` run
artifact. Use it after any change to `release.yml`, or before merging a
bump.

- [ ] The `release-tag` run for the bump's merge is green and created
      `vX.Y.Z` on the bump commit (its summary names the commit), and a
      `release` run on `vX.Y.Z` started (Actions → release, event
      `workflow_dispatch`, branch `vX.Y.Z`). If `release-tag` failed, read
      its error: fix the CHANGELOG in a new bump, or use the fallback above.
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
