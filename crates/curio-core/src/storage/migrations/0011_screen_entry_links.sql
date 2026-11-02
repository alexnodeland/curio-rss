-- Migration 0011 — screen stored feed-entry links to http(s) (#262).
--
-- Builds before #262 stored an entry's declared <link> unscreened, so a
-- `javascript:`, custom-scheme (`foo://…`, `ftp://…`) or base-less relative
-- link became the article's source_url: the reader's source link, the
-- exported `source`, the base a load-full would fetch. Ingest now keeps a
-- link only when the policed client's screen passes it (`fetch::screen_url`:
-- an absolute http(s) URL with a host and no userinfo) and otherwise stores
-- the entry the link-less way, under its feed's URL. This brings the rows
-- earlier builds wrote into the same shape, so they read as link-less too
-- (no source link, never hydrated).
--
-- SQL has no URL parser, so the screen is spelled out on the text, the way
-- the WHATWG parser the screen uses reads a special (http/https) URL:
--
-- * the scheme must be `http://` or `https://` (LIKE is ASCII
--   case-insensitive, so `HTTPS://…` is kept);
-- * the parser skips any further `/` or `\` before the authority, which then
--   runs up to the first `/`, `\`, `?` or `#`;
-- * an `@` in that authority is userinfo (`https://user:pw@host/…`): moved;
-- * an empty host (`http://`, `http:///`, `https://?q`, `http://:80/`):
--   moved.
--
-- Only the source_url moves; the dedupe key stays: ingest still keys an
-- entry by its declared link, so the next refresh still finds this row.
-- modified_at is bumped as a refresh's own rewrite would bump it. Rows whose
-- feed is gone (feed_id NULL: manual saves, imports, starred survivors) are
-- untouched — saves and imports were always screened, and there is no feed
-- URL to fall back on.
WITH
  -- Everything after `scheme://` and any further slashes, `?`/`#`/`\`
  -- folded to `/` so the authority ends at the first `/`.
  rest (id, tail) AS (
    SELECT id,
           replace(replace(replace(
             ltrim(substr(source_url, instr(source_url, '://') + 3), '/\'),
             '\', '/'), '?', '/'), '#', '/')
    FROM articles
    WHERE source_url LIKE 'http://%' OR source_url LIKE 'https://%'
  ),
  authority (id, host_port) AS (
    SELECT id,
           CASE WHEN instr(tail, '/') > 0
                THEN substr(tail, 1, instr(tail, '/') - 1)
                ELSE tail END
    FROM rest
  ),
  web (id) AS (
    SELECT id FROM authority
    WHERE instr(host_port, '@') = 0
      AND host_port <> ''
      AND host_port NOT LIKE ':%'
  )
UPDATE articles
SET source_url = (SELECT feeds.url FROM feeds WHERE feeds.id = articles.feed_id),
    modified_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
WHERE id NOT IN (SELECT id FROM web)
  AND EXISTS (SELECT 1 FROM feeds WHERE feeds.id = articles.feed_id);
