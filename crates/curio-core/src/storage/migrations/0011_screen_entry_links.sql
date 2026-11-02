-- Migration 0011 — screen stored feed-entry links to http(s) (#262).
--
-- Builds before #262 stored an entry's declared <link> unscreened, so a
-- `javascript:`, custom-scheme (`foo://…`, `ftp://…`) or base-less relative
-- link became the article's source_url: the reader's source link, the
-- exported `source`, the base a load-full would fetch. Ingest now drops any
-- link that is not an absolute http(s) URL and stores the entry the
-- link-less way, under its feed's URL. This brings the rows earlier builds
-- wrote into the same shape, so they read as link-less too (no source link,
-- never hydrated).
--
-- Only the source_url moves; the dedupe key stays: ingest still keys an
-- entry by its declared link, so the next refresh still finds this row.
-- modified_at is bumped as a refresh's own rewrite would bump it. LIKE is
-- ASCII case-insensitive, so `HTTPS://…` is kept. Rows whose feed is gone
-- (feed_id NULL: manual saves, imports, starred survivors) are untouched —
-- saves and imports were always screened, and there is no feed URL to
-- fall back on.
UPDATE articles
SET source_url = (SELECT feeds.url FROM feeds WHERE feeds.id = articles.feed_id),
    modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE source_url NOT LIKE 'http://%'
  AND source_url NOT LIKE 'https://%'
  AND EXISTS (SELECT 1 FROM feeds WHERE feeds.id = articles.feed_id);
