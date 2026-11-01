-- The machine-readable failure class of an errored fetch (refused, tls_certificate,
-- not_a_feed, …; see FetchErrorKind::as_str), next to the human `error` text.
-- NULL for successful attempts and for errored rows recorded before the kind
-- was tracked.
ALTER TABLE fetch_log ADD COLUMN error_kind TEXT;
