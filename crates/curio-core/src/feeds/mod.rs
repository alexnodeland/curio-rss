//! Feed understanding: RSS/Atom/JSON-Feed parsing, autodiscovery and OPML
//! exchange.
//!
//! [`parse_feed`] turns fetched bytes into normalized [`ParsedFeed`] /
//! [`ParsedEntry`] values with a **total dedupe key** (D3):
//! `guid → link → hash(title + published)` — every entry gets a stable
//! key, no matter how sloppy the feed — and the keys are unique within one
//! fetch (a guid reused across different links falls back to a `guid+link`
//! key; [`ParsedFeed::settle_keys`] keeps an item's key stable across
//! fetches). Content here is still *raw*
//! source HTML: sanitization happens in [`crate::content`] before
//! anything reaches storage.
//!
//! [`discover()`] turns whatever a user typed (a bare domain, a homepage, a
//! reddit/GitHub/YouTube/Mastodon/Substack page) into validated feed URLs.
//!
//! [`parse_opml`] (lenient about messy real-world exports) /
//! [`export_opml`] exchange subscription lists; folder nesting maps onto
//! tags — the enclosing folder path becomes one `/`-joined hierarchical tag
//! on import, and tags export as the `category` attribute (which import also
//! reads), so an import/export cycle is lossless after one normalization.

pub mod discover;
mod entities;
mod image;
mod keys;
mod opml;
mod parse;

pub use discover::{
    DiscoverError, DiscoverOptions, DiscoveredFeed, Discovery, Resolution, discover,
    normalize_input, resolve,
};
pub use opml::{OpmlError, OpmlFeed, OpmlImport, export_opml, import_opml, parse_opml};
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

/// The disambiguated key for an item whose guid the feed reuses for items
/// with different links (see [`ParsedEntry::dedupe_key`]). Its own
/// `guid+link:` namespace never collides with the [`dedupe_key`] tiers;
/// the unit separator keeps `("a b", "c")` apart from `("a", "b c")`.
fn guid_link_key(guid: &str, link: Option<&str>) -> String {
    let link = link.map(str::trim).unwrap_or_default();
    format!("guid+link:{guid}\u{1f}{link}")
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
    fn guid_link_keys_live_in_their_own_namespace() {
        let key = guid_link_key("g", Some(" https://x/a "));
        assert_eq!(key, "guid+link:g\u{1f}https://x/a");
        assert_ne!(key, dedupe_key(Some("g"), Some("https://x/a"), "T", None));
        assert_ne!(
            guid_link_key("a b", Some("c")),
            guid_link_key("a", Some("b c")),
            "the separator keeps the halves apart"
        );
        assert_eq!(guid_link_key("g", None), "guid+link:g\u{1f}");
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
