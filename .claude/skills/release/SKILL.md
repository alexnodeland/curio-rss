---
name: release
description: Cut a Curio release — version bump, changelog, tag, artifacts. Use when asked to release, tag, or publish a version.
---

# Release

The tag-triggered pipeline (`.github/workflows/release.yml`,
`docs/release/runbook.md` §2) re-runs the full CI suite on the tagged commit,
builds the macOS universal bundle with no secrets, signs the updater archive
in one isolated step, and drafts a GitHub release carrying SHA256SUMS,
CycloneDX SBOMs and a build-provenance attestation. A human publishes the
draft. The Linux/Windows release channels are still to come. A release is
exactly this:

1. **Green gate first**: `just ci` locally AND CI green on `main`. No
   release from a red or dirty tree.
2. **Version bump**: single source of truth is `[workspace.package] version`
   in the root `Cargo.toml` (crates inherit it). Bump it in its own commit:
   `chore(release): v0.X.Y`. Follow semver; while pre-1.0, breaking changes
   bump the minor. The mirrors move in the same commit: `tauri.conf.json`,
   `apps/desktop/package.json` **and** `apps/desktop/package-lock.json`
   (run `npm install --package-lock-only` under `apps/desktop` — a stale
   lock version makes every `npm install` dirty the tree), `Cargo.lock`.
3. **Contract check**: if `schemas/` changed since the last tag, verify
   `schemas/CHANGELOG.md` records it and no vN file was edited in place
   (`git diff <last-tag> -- schemas/`).
4. **Changelog**: summarize commit subjects since the last tag under a new
   version heading (CHANGELOG.md at root once releases begin).
5. **Dry run**: if `release.yml` changed since the last tag, dispatch it on
   `main` (Actions → release → Run workflow) and check the
   `release-dry-run` artifact before tagging. An agent whose token can't
   dispatch workflows (HTTP 403) says so in its hand-off instead of
   skipping silently; the tag run's own `gate` job still re-runs CI.
6. **Tag**: annotated tag `vX.Y.Z` on the release commit (the squash-merge
   of the bump PR, green on CI). Agents may create **and push** the tag
   (`git push origin vX.Y.Z`; maintainer decision, 2026-11-01), then watch
   the release run to its draft. A pushed tag is never moved or deleted;
   a broken release ships as the next patch version. Approving the `sign`
   job's `updater-signing` deployment and publishing the draft stay with
   the maintainer.
   Once the tag exists, a follow-up PR adds its migration fixture:
   generate `fixtures/db/vX.Y.Z.db` from a checkout of the tag (recipe in
   `fixtures/db/generate.rs`) and add a case to
   `crates/curio-core/tests/migration_fixtures.rs`.
7. **Never** publish to crates.io, sign, or upload artifacts without the
   maintainer explicitly asking, per repo policy (BDFL: Alex).
