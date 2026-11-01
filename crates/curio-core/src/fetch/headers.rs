//! Parsers for the response headers that shape a feed's refresh
//! schedule: `Retry-After` and `Cache-Control: max-age`.

use std::time::Duration;

/// A `Retry-After` value as a wait from `now`: either form RFC 9110
/// allows — delta-seconds (`120`) or an HTTP-date (`Wed, 21 Oct 2026
/// 07:28:00 GMT`). A date already past is a zero wait; garbage is `None`.
pub(super) fn parse_retry_after(
    value: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<Duration> {
    let value = value.trim();
    if let Ok(seconds) = value.parse::<u64>() {
        return Some(Duration::from_secs(seconds));
    }
    let at = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    Some(
        at.with_timezone(&chrono::Utc)
            .signed_duration_since(now)
            .to_std()
            .unwrap_or(Duration::ZERO),
    )
}

/// The freshness lifetime a `Cache-Control` header grants: its
/// `max-age`, unless `no-store`/`no-cache` says not to reuse the
/// response at all. `None` when absent, zero or unparseable.
pub(super) fn parse_max_age(value: &str) -> Option<Duration> {
    let mut max_age = None;
    for directive in value.split(',') {
        let directive = directive.trim().to_ascii_lowercase();
        if directive == "no-store" || directive == "no-cache" {
            return None;
        }
        if let Some(seconds) = directive.strip_prefix("max-age=") {
            max_age = seconds.trim_matches('"').parse::<u64>().ok();
        }
    }
    max_age
        .filter(|&seconds| seconds > 0)
        .map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn retry_after_reads_both_forms() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-21T07:20:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            parse_retry_after(" 120 ", now),
            Some(Duration::from_secs(120))
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2026 07:28:00 GMT", now),
            Some(Duration::from_secs(480))
        );
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2026 07:00:00 GMT", now),
            Some(Duration::ZERO),
            "a date in the past is no wait"
        );
        assert_eq!(parse_retry_after("soon", now), None);
        assert_eq!(parse_retry_after("-5", now), None);
    }

    #[test]
    fn max_age_reads_cache_control() {
        assert_eq!(
            parse_max_age("public, max-age=3600"),
            Some(Duration::from_secs(3600))
        );
        assert_eq!(
            parse_max_age("Max-Age=\"60\""),
            Some(Duration::from_secs(60))
        );
        assert_eq!(parse_max_age("max-age=0"), None);
        assert_eq!(parse_max_age("max-age=600, no-cache"), None);
        assert_eq!(parse_max_age("no-store"), None);
        assert_eq!(parse_max_age("private"), None);
        assert_eq!(parse_max_age("max-age=soon"), None);
    }
}
