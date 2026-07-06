# Known issues

Findings from the 2026-07-03 adversarial review that were deliberately
**not** (or only partially) fixed, with rationale. Everything else from
that review was fixed in the same change series — see the
`fix(events)`/`fix(export)`/`fix(storage)`/`fix(core)`/`fix(fetch)`
commits around this file's introduction.

Rule of thumb for this file: an entry is either (a) a contract-document
amendment that has to be coordinated with the KP consumer side (the
shared contract draft is versioned-immutable and co-owned), or (b) a
behavior whose proper fix is structural and whose blast radius today is
negligible. Each entry names the trigger that would promote it to a
task.

## Contract-document amendments (coordinate with KP before editing)

The shared contract draft (`docs/design/contracts-draft.md`, mirrored in
the KP build session) needs four clarifications. The **code** is the
reference behavior in the meantime; none of these change bytes on disk.

1. **Checksum byte-range is underspecified.** The doc says "sha256 of
   the managed content region bytes"; the implementation hashes the
   `trim_end()`-ed markdown body **excluding** the single `\n` that
   follows the begin-marker line and the single `\n` that precedes the
   end-marker line. An independent consumer hashing "everything between
   the marker lines" verbatim gets a permanent mismatch. Amendment: state the
   exact range (bytes strictly between the delimiting newlines; body is
   right-trimmed before writing). Reference: `export::region_checksum`
   + `note::split_note`; the regression test
   `marker_literals_in_the_body_cannot_break_the_managed_region`
   demonstrates consumer-side reproduction.
2. **Per-file `ts` ordering is not unconditional.** Crash-window B with
   2+ pending intents replays duplicates *after* later-`ts` lines in the
   same file (proven by `replay_after_partial_emission_duplicates_harmlessly`),
   and a wall-clock step backwards writes a decreasing `ts` (no
   monotonic clamp — the `ts` is fixed at staging time inside the state
   transaction and cannot be rewritten at append time without breaking
   intent/line identity). Amendment: "consumers MUST dedupe by
   `event_id` before relying on per-file `ts` monotonicity; duplicates
   from crash replay may appear out of `ts` order."
3. **The missing-note heal path is undocumented.** When the manifest
   says unchanged but the note file is gone, export recreates the note
   and rewrites the manifest (new `exported_at`) with disposition
   `Unchanged` and **no event** (test:
   `a_deleted_note_is_healed_without_a_new_event_disposition`). This is
   deliberate — the event stream already told the story, and a deleted
   file must not fork state — but the contract neither authorizes nor
   describes it. Amendment: specify the heal.
4. **Required-key set: doc vs schema.** The doc's YAML comments mark
   only `curio_id`/`title`/`source`/`saved` REQUIRED; the schema
   (authoritative) additionally requires `schema`, `feed`, `published`,
   `tags`, `checksum` (as keys — `feed`/`published` may be null).
   Amendment: mark the full required set in the doc.

Trigger to act: the next KP↔Curio contract sync, or the first external
consumer implementation.

## Behavior

- **FIXED (Phase 4 / desktop WP1a): concurrent refreshes of one feed
  raced on conditional-GET validators**
  (`handle.rs::preserve_validators`): an error-path preserve wrote back
  the pre-fetch etag/last-modified snapshot and could clobber fresher
  validators a parallel successful refresh had just stored (bounded
  consequence: one wasted full-body 200 or a missed 304). The named
  trigger arrived — the desktop head refreshes the same feed from two
  tasks (`refresh_feed` while `refresh_all` runs) — so `refresh_feed`
  now **serializes per feed**: concurrent callers queue on a per-feed
  `tokio::sync::Mutex` (`CoreHandle::refresh_locks`) and the feed
  snapshot is read inside the critical section, so a preserve always
  carries the freshest stored pair. Different feeds still refresh in
  parallel. Regression test:
  `concurrent_refreshes_of_one_feed_cannot_clobber_validators`
  (tests/handle.rs — deterministically fails when the lock is removed).
- **CRLF-rewritten notes still refuse re-export** (diagnostic fixed —
  the error now names line endings and the remedy; see
  `split_names_crlf_line_endings_in_its_refusal`). Accepting CRLF
  losslessly means byte-preserving surgery on a file whose every line
  ending differs from what Curio writes — replace-managed would emit
  mixed-endings files or silently normalize user bytes, both worse than
  an honest refusal. Trigger: first real-user report; the likely shape
  is a one-time `curio doctor --fix-line-endings` normalization with
  explicit consent.
- **Feed-scoped dedupe uses the feed row id** (`f<id>:` prefix). Rows
  orphaned by a feed removal keep the historical prefix; if SQLite ever
  reuses that rowid for a new feed (only possible when the deleted feed
  held the maximum id), the new feed could match an orphaned article's
  key and update it in place while `feed_id` stays NULL. Personal-scale
  probability ~0, damage is one stale row refreshed. Noted in
  `scoped_dedupe_key`'s doc. Trigger: a real collision, or migration to
  `AUTOINCREMENT` if it ever matters.

## Publication status (tracked, not a code issue)

CONTRIBUTING/README/CLAUDE.md now state that the GitHub remote and the
CI workflows are pre-publication artifacts and that `just ci` (including
the blob guard) is the gate until then. The remaining risk — nothing
server-side prevents a collaborator from skipping `just ci` — is
accepted while the repo is single-author and unpublished. Trigger:
publication, which activates `.github/workflows/` as the backend for
every enforcement sentence.
