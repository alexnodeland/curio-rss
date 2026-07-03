//! Contract timestamps: RFC 3339 UTC with exactly millisecond precision.

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, SecondsFormat, Timelike as _, Utc};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// An RFC 3339 UTC timestamp with exactly millisecond precision — the wire
/// form of every `ts` / `saved` / `exported_at` field in the published
/// contracts (e.g. `2026-07-03T09:15:00.123Z`).
///
/// Construction truncates to millisecond precision, so a `Timestamp` always
/// round-trips byte-identically through its string form. Parsing is a
/// lenient reader: any valid RFC 3339 timestamp is accepted (offsets are
/// normalized to UTC, sub-millisecond precision is truncated); serialization
/// always emits the canonical `Z`-suffixed, 3-fractional-digit form the
/// schemas require.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Timestamp(DateTime<Utc>);

impl Timestamp {
    /// Wraps a [`DateTime<Utc>`], truncating to millisecond precision.
    #[must_use]
    pub fn new(datetime: DateTime<Utc>) -> Self {
        let nanos = datetime.nanosecond();
        let truncated = datetime
            .with_nanosecond((nanos / 1_000_000) * 1_000_000)
            .unwrap_or(datetime);
        Self(truncated)
    }

    /// The current time, truncated to millisecond precision.
    #[must_use]
    pub fn now() -> Self {
        Self::new(Utc::now())
    }

    /// Builds a timestamp from milliseconds since the Unix epoch.
    ///
    /// Returns `None` if the value is out of chrono's representable range.
    #[must_use]
    pub fn from_unix_millis(millis: i64) -> Option<Self> {
        DateTime::from_timestamp_millis(millis).map(Self::new)
    }

    /// Milliseconds since the Unix epoch.
    #[must_use]
    pub fn unix_millis(&self) -> i64 {
        self.0.timestamp_millis()
    }

    /// The underlying [`DateTime<Utc>`] (already millisecond-truncated).
    #[must_use]
    pub fn as_datetime(&self) -> DateTime<Utc> {
        self.0
    }
}

impl From<DateTime<Utc>> for Timestamp {
    fn from(datetime: DateTime<Utc>) -> Self {
        Self::new(datetime)
    }
}

impl fmt::Display for Timestamp {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.to_rfc3339_opts(SecondsFormat::Millis, true))
    }
}

/// Error parsing a [`Timestamp`] from its RFC 3339 string form.
#[derive(Debug, thiserror::Error)]
#[error("invalid RFC 3339 timestamp: {0}")]
pub struct ParseTimestampError(#[from] chrono::ParseError);

impl FromStr for Timestamp {
    type Err = ParseTimestampError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = DateTime::parse_from_rfc3339(s)?;
        Ok(Self::new(parsed.with_timezone(&Utc)))
    }
}

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn formats_with_exactly_millisecond_precision() {
        let ts = Timestamp::from_unix_millis(1_782_119_700_123).unwrap();
        let s = ts.to_string();
        assert!(s.ends_with('Z'));
        assert_eq!(s.len(), "2026-07-03T09:15:00.123Z".len());
        assert!(s.contains('.'));
    }

    #[test]
    fn whole_seconds_still_carry_three_fractional_digits() {
        let ts: Timestamp = "2026-07-01T12:00:00Z".parse().unwrap();
        assert_eq!(ts.to_string(), "2026-07-01T12:00:00.000Z");
    }

    #[test]
    fn parse_truncates_submillisecond_precision() {
        let ts: Timestamp = "2026-07-03T09:15:00.123456789Z".parse().unwrap();
        assert_eq!(ts.to_string(), "2026-07-03T09:15:00.123Z");
    }

    #[test]
    fn parse_normalizes_offsets_to_utc() {
        let ts: Timestamp = "2026-07-03T11:15:00.123+02:00".parse().unwrap();
        assert_eq!(ts.to_string(), "2026-07-03T09:15:00.123Z");
    }

    #[test]
    fn rejects_non_rfc3339_input() {
        assert!("2026-07-03 09:15:00".parse::<Timestamp>().is_err());
        assert!("not a timestamp".parse::<Timestamp>().is_err());
    }
}
