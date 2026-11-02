//! The article-retention policy (#137).

/// The optional article-retention policy the maintenance pass applies
/// (see [`crate::CoreHandle::maintenance`]). Off by default: Curio keeps
/// every article until a limit is set.
///
/// Only feed articles are ever reaped, and never a **protected** one:
/// starred, in read-later, tagged, ever exported to a destination, or
/// — while [`Self::keep_unread`] is on — unread (neither read nor
/// archived). Feedless saves (clips, imports) are never reaped by policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RetentionPolicy {
    /// Reap articles Curio first stored more than this many days ago.
    /// `None` = no age limit.
    pub max_age_days: Option<std::num::NonZeroU32>,
    /// Keep at most this many articles per feed — the newest stored;
    /// protected ones count towards the limit but are never reaped.
    /// `None` = no count limit.
    pub max_per_feed: Option<std::num::NonZeroU32>,
    /// Never reap an unread article. On by default.
    pub keep_unread: bool,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            max_age_days: None,
            max_per_feed: None,
            keep_unread: true,
        }
    }
}

impl RetentionPolicy {
    /// Whether the policy reaps anything at all (a limit is set).
    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.max_age_days.is_some() || self.max_per_feed.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_is_off_by_default_and_keeps_unread() {
        let policy = RetentionPolicy::default();
        assert!(!policy.is_enabled());
        assert!(policy.keep_unread);
        let aged = RetentionPolicy {
            max_age_days: std::num::NonZeroU32::new(30),
            ..policy
        };
        assert!(aged.is_enabled());
        let counted = RetentionPolicy {
            max_per_feed: std::num::NonZeroU32::new(100),
            ..policy
        };
        assert!(counted.is_enabled());
    }
}
