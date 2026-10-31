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
   `release-dry-run` artifact before tagging.
6. **Tag**: annotated tag `vX.Y.Z` on the release commit. Human pushes tags
   — agents never push.
7. **Never** publish to crates.io, sign, or upload artifacts without the
   maintainer explicitly asking, per repo policy (BDFL: Alex).
