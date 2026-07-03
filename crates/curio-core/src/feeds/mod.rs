//! Feed understanding: RSS/Atom/JSON-Feed parsing and OPML exchange.
//!
//! [`parse_feed`] turns fetched bytes into normalized [`ParsedFeed`] /
//! [`ParsedEntry`] values with a **total dedupe key** (D3):
//! `guid → link → hash(title + published)` — every entry gets a stable
//! key, no matter how sloppy the feed. Content here is still *raw*
//! source HTML: sanitization happens in [`crate::content`] before
//! anything reaches storage.
//!
//! [`import_opml`] / [`export_opml`] exchange subscription lists;
//! folder nesting maps onto tags (folder path → tags on import, tags →
//! the `category` attribute on export, which import also reads — so an
//! import/export cycle is lossless after one normalization).

mod opml;
mod parse;

pub use opml::{OpmlError, OpmlFeed, export_opml, import_opml};
pub use parse::{FeedParseError, ParsedEntry, ParsedFeed, ParsedFeedMeta, parse_feed};

use sha2::{Digest as _, Sha256};

use curio_types::Timestamp;

/// The total dedupe chain: `guid → link → hash(title + published)`.
///
/// Returns a namespaced key so the tiers can never collide with each
/// other (`guid:…`, `link:…`, `hash:…`).
#[must_use]
pub fn dedupe_key(
    guid: Option<&str>,
    link: Option<&str>,
    title: &str,
    published: Option<Timestamp>,
) -> String {
    let guid = guid.map(str::trim).filter(|g| !g.is_empty());
    if let Some(guid) = guid {
        return format!("guid:{guid}");
    }
    let link = link.map(str::trim).filter(|l| !l.is_empty());
    if let Some(link) = link {
        return format!("link:{link}");
    }
    let mut hasher = Sha256::new();
    hasher.update(title.as_bytes());
    hasher.update([0x1f]); // unit separator: "a"+"bc" never equals "ab"+"c"
    if let Some(published) = published {
        hasher.update(published.to_string().as_bytes());
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        #[allow(clippy::unwrap_used, reason = "writing hex to a String is infallible")]
        write!(hex, "{byte:02x}").unwrap();
    }
    format!("hash:{hex}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn guid_wins_over_everything() {
        let key = dedupe_key(
            Some("urn:x:1"),
            Some("https://example.com/a"),
            "Title",
            None,
        );
        assert_eq!(key, "guid:urn:x:1");
    }

    #[test]
    fn link_is_the_second_tier() {
        let key = dedupe_key(None, Some("https://example.com/a"), "Title", None);
        assert_eq!(key, "link:https://example.com/a");
        let blank_guid = dedupe_key(Some("  "), Some("https://example.com/a"), "Title", None);
        assert_eq!(blank_guid, key, "whitespace guid falls through");
    }

    #[test]
    fn hash_tier_is_total_and_stable() {
        let ts: Timestamp = "2026-07-01T12:00:00Z".parse().unwrap();
        let a = dedupe_key(None, None, "Title", Some(ts));
        let b = dedupe_key(None, None, "Title", Some(ts));
        assert_eq!(a, b);
        assert!(a.starts_with("hash:"));
        assert_eq!(a.len(), "hash:".len() + 64);
    }

    #[test]
    fn hash_tier_distinguishes_title_and_published() {
        let ts: Timestamp = "2026-07-01T12:00:00Z".parse().unwrap();
        let base = dedupe_key(None, None, "Title", Some(ts));
        assert_ne!(dedupe_key(None, None, "Other", Some(ts)), base);
        assert_ne!(dedupe_key(None, None, "Title", None), base);
    }

    #[test]
    fn tiers_never_collide() {
        // A guid that *looks* like a link key still lands in guid-space.
        let guid = dedupe_key(Some("link:https://x"), None, "T", None);
        let link = dedupe_key(None, Some("https://x"), "T", None);
        assert_ne!(guid, link);
        assert!(guid.starts_with("guid:"));
    }
}
