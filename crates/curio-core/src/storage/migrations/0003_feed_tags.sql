-- Migration 0003 — persist feed tags.
--
-- Feed tags previously existed only in the one-time feed.added event
-- payload (retention-swept after 90 days), so export_opml had nothing to
-- write and the documented lossless OPML import/export cycle silently
-- flattened all folder/category structure. Tags now live on the feed row
-- as a canonical JSON array of non-empty strings.
--
-- Existing rows get the empty list: the historical tags are only in the
-- event log, and guessing is worse than an honest empty set.

ALTER TABLE feeds ADD COLUMN tags TEXT NOT NULL DEFAULT '[]';
