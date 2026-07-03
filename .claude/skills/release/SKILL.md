---
name: release
description: Cut a Curio release — version bump, changelog, tag, artifacts. Use when asked to release, tag, or publish a version.
---

# Release

Pre-1.0 status: the full release pipeline (draft GitHub releases, 3-OS
matrix, SHA256SUMS + minisign + SBOM) lands in Phase 1 of
`docs/design/roadmap.md`. Until it exists, a release is exactly this:

1. **Green gate first**: `just ci` locally AND CI green on `main`. No
   release from a red or dirty tree.
2. **Version bump**: single source of truth is `[workspace.package] version`
   in the root `Cargo.toml` (crates inherit it). Bump it in its own commit:
   `chore(release): v0.X.Y`. Follow semver; while pre-1.0, breaking changes
   bump the minor.
3. **Contract check**: if `schemas/` changed since the last tag, verify
   `schemas/CHANGELOG.md` records it and no vN file was edited in place
   (`git diff <last-tag> -- schemas/`).
4. **Changelog**: summarize commit subjects since the last tag under a new
   version heading (CHANGELOG.md at root once releases begin).
5. **Tag**: annotated tag `vX.Y.Z` on the release commit. Human pushes tags
   — agents never push.
6. **Never** publish to crates.io, sign, or upload artifacts without the
   maintainer explicitly asking, per repo policy (BDFL: Alex).
