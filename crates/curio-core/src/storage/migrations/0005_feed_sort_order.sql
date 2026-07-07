-- Migration 0005 — user-defined feed ordering.
--
-- Feeds were listed by row id (subscription order). The sidebar now lets you
-- drag feeds into whatever order you want — within a folder or the top-level
-- list — so each feed carries an explicit sort_order. Ordering is global:
-- feeds render by sort_order everywhere, and since feeds in different folders
-- never interleave visually, one global sequence is enough.
--
-- Existing rows seed sort_order = id, preserving the current subscription
-- order exactly. New feeds append (add_feed sets sort_order = max + 1).

ALTER TABLE feeds ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
UPDATE feeds SET sort_order = id;
