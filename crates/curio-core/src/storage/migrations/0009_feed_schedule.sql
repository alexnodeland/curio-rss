-- Per-feed refresh scheduling: failure backoff, Retry-After, the feed's
-- own refresh hints and auto-pause (#122).
--
-- consecutive_failures  failed attempts in a row; reset by a success or a
--                       resume.
-- next_attempt_at       RFC 3339 UTC; a sweep skips the feed until then.
--                       NULL = due on every sweep.
-- last_error_kind       the class of the latest failure (rate_limited,
--                       gone, server, client, network, refused, parse);
--                       NULL once a fetch succeeds.
-- refresh_interval_min  the user's per-feed interval; NULL = global.
-- failing_since         when the current run of failures began.
-- auto_paused_at        set when the engine paused the feed by itself.
ALTER TABLE feeds ADD COLUMN consecutive_failures INTEGER NOT NULL DEFAULT 0;
ALTER TABLE feeds ADD COLUMN next_attempt_at TEXT;
ALTER TABLE feeds ADD COLUMN last_error_kind TEXT;
ALTER TABLE feeds ADD COLUMN refresh_interval_min INTEGER;
ALTER TABLE feeds ADD COLUMN failing_since TEXT;
ALTER TABLE feeds ADD COLUMN auto_paused_at TEXT;

-- fetch_log is now kept to the newest 50 rows per feed; trim what older
-- builds let grow without bound.
DELETE FROM fetch_log
WHERE id < (
    SELECT newer.id FROM fetch_log AS newer
    WHERE newer.feed_id = fetch_log.feed_id
    ORDER BY newer.id DESC
    LIMIT 1 OFFSET 49
);
