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
  (`handle/refresh.rs::preserve_validators`): an error-path preserve wrote back
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
  Across processes too since #234 (the desktop app and a CLI
  `curio fetch` refreshing the same feed at the same moment): after the
  in-process mutex, a refresh takes the feed's lock file under
  `.curio/locks/`. Regression test:
  `concurrent_refreshes_from_two_handles_cannot_clobber_validators`.
- **FIXED (#114 / #234): destinations were not process-safe.** Each
  `CoreHandle` loaded the destination registry once at open and wrote
  the whole cached map back on `add_destination`/`remove_destination`,
  so a head could undo a destination change another process made while
  it was open, and the CLI's `curio.toml` reconcile (run by every
  command) re-added a destination the desktop app had removed. The
  settings table is now the only copy (every change a read-modify-write
  in one `IMMEDIATE` transaction) and `curio.toml` seeds destinations
  instead of owning them (D16). Regression tests:
  `a_destination_removed_by_another_head_stays_removed_after_a_cli_run`,
  `a_destination_added_through_another_handle_is_never_erased`
  (tests/handle.rs) and
  `a_destination_removed_in_the_desktop_stays_removed_after_a_cli_run`
  (curio-cli tests/cli.rs). One gap is left by design: the seeding
  record starts empty on upgrade, so a destination removed in the
  desktop app *before* upgrading comes back once on the first CLI run
  if `curio.toml` still lists it, and stays removed after the user
  removes it again.
- **Event emission is at-least-once under heavy cross-process
  contention.** A drain appends and fsyncs under the events lock, then
  deletes the drained intents through this process's writer. If another
  process holds a database write past the 5 s busy timeout, that delete
  fails with `Busy` after the lines are durable; the intents stay
  staged and the next drain (in either process) appends them again. The
  contract requires consumers to dedupe by `event_id`, so nothing is
  lost or corrupted, but the log can carry a duplicate line. Related:
  the events lock is held while the delete waits behind this process's
  queued write jobs, so a long in-process job (a vacuum, a big import)
  stalls other processes' emits. Past 15 s their drain gives up with
  `ProfileLocked` and, emission being non-fatal (#118), the events stay
  staged behind the pending-events flag until the next mutation, the
  desktop's periodic retry, or the next open drains them; only opening
  a profile reports it (CLI exit 75). Trigger to act: a consumer that
  cannot dedupe, or `ProfileLocked` reports from real use. The fix would be a
  durable "appended, not yet retired" record kept under the lock.
- **FIXED (#111): every refresh reverted hydrated/enriched full text to
  the feed excerpt and rewrote unchanged rows.** `upsert_articles`
  (`storage/repo/articles.rs`) now consults `articles.content_origin`
  (migration 0007): hydrated (`full_text`) and `enriched` bodies take
  title/metadata updates only, and every update is change-aware — an
  identical entry writes nothing, so `updated` counts real changes, the
  next export stays `unchanged`, and the FTS row is untouched (the
  `articles_fts_au` trigger also gained a `WHEN` value-change guard).
  The upgrade infers provenance: rows of a feed in full-text mode are
  backfilled `full_text` (so their newest, still-hydrated batch survives),
  everything else `feed`/`manual`. A body loaded in full (load-full) on a
  feed *not* in full-text mode carries no recoverable provenance — if it
  has not been reverted yet, the first post-upgrade refresh that still
  lists the item reverts it to the excerpt one last time; load it in full
  again and it is kept from then on. Regression tests:
  `hydrated_full_text_survives_refresh_and_re_export_is_unchanged`,
  `enriched_reddit_body_survives_a_refresh_of_its_feed` (tests/handle.rs),
  `re_upserting_identical_entries_writes_nothing` (tests/storage.rs).
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

## Publication status (RESOLVED at v0.2.0)

The repo is **public** and v0.1.0, v0.2.0, and v0.3.0 are published GitHub
releases. `.github/workflows/` is now the active server-side gate on every
push — the same `just ci` suite (fmt, clippy `-D warnings`, hermetic tests,
cargo-deny, boundary check, coverage floor, blob guard, frontend gates) runs
in CI, so the earlier "nothing server-side enforces the gate" risk no longer
applies. Any residual hardening (e.g. branch-protection required-check config)
is repo settings, not a code issue.
