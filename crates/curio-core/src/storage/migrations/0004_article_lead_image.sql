-- Migration 0004 — persist an article's lead image.
--
-- RSS carries images as first-class metadata (media:thumbnail,
-- media:content, image enclosures) and, failing that, inline in the body.
-- The reader now surfaces those images in list rows and the Reddit/YouTube
-- home views, so ingest extracts a single lead-image URL per article and
-- stores it here. The value is always an absolute http(s) URL; consumers
-- load it through the policed image cache, never directly.
--
-- Existing rows get NULL: the image was never captured at their ingest, and
-- backfilling would mean re-fetching. New and re-ingested articles populate
-- it going forward.

ALTER TABLE articles ADD COLUMN lead_image TEXT;
