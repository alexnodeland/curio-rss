//! Dedupe-key hygiene over one parsed fetch: keys unique within the fetch
//! ([`unique_keys`]), then settled against the keys earlier fetches stored
//! ([`ParsedFeed::settle_keys`]) so an item never changes key between
//! fetches.

use std::collections::{HashMap, HashSet};

use super::parse::{ParsedEntry, ParsedFeed};
use super::{dedupe_key, guid_link_key};

/// Makes every dedupe key unique within one fetch. Without this the upsert
/// writes each same-key item over the previous one (the feed collapses to
/// one article) and full-text mode queues the survivor for hydration once
/// per duplicate.
///
/// - A guid the feed reuses for items with **different links** (a broken
///   id generator, a constant guid) is no identity: *every* item in that
///   group — the first included, so the keys don't depend on feed order —
///   is re-keyed `guid+link`.
/// - Items whose keys still match (same guid and link, same link, same
///   title + date) are one item listed twice: the first wins.
pub(super) fn unique_keys(entries: Vec<ParsedEntry>) -> Vec<ParsedEntry> {
    let mut links_by_guid: HashMap<&str, HashSet<Option<&str>>> = HashMap::new();
    for entry in &entries {
        if let Some(guid) = entry.guid.as_deref() {
            links_by_guid
                .entry(guid)
                .or_default()
                .insert(entry.link.as_deref());
        }
    }
    let reused: HashSet<String> = links_by_guid
        .into_iter()
        .filter(|(_, links)| links.len() > 1)
        .map(|(guid, _)| guid.to_owned())
        .collect();
    let mut seen = HashSet::new();
    entries
        .into_iter()
        .filter_map(|mut entry| {
            if let Some(guid) = entry.guid.as_deref().filter(|guid| reused.contains(*guid)) {
                entry.dedupe_key = guid_link_key(guid, entry.link.as_deref());
            }
            seen.insert(entry.dedupe_key.clone()).then_some(entry)
        })
        .collect()
}

/// A guid entry's two possible keys.
enum GuidKey {
    /// Keyed `guid:` (its guid is unshared in this fetch); the payload is
    /// the `guid+link:` form a fetch sharing the guid would give it.
    Plain(String),
    /// Re-keyed `guid+link:` (its guid is shared in this fetch); the
    /// payload is the plain `guid:` form.
    Reused(String),
}

fn alternate_key(entry: &ParsedEntry) -> Option<GuidKey> {
    let guid = entry.guid.as_deref()?;
    let plain = dedupe_key(Some(guid), None, "", None);
    let reused = guid_link_key(guid, entry.link.as_deref());
    if entry.dedupe_key == plain {
        Some(GuidKey::Plain(reused))
    } else if entry.dedupe_key == reused {
        Some(GuidKey::Reused(plain))
    } else {
        None
    }
}

impl ParsedFeed {
    /// The stored keys [`Self::settle_keys`] consults: every guid entry's
    /// own key and its alternate form.
    #[must_use]
    pub fn settle_lookups(&self) -> Vec<String> {
        let mut keys = Vec::new();
        for entry in &self.entries {
            if let Some(GuidKey::Plain(other) | GuidKey::Reused(other)) = alternate_key(entry) {
                keys.push(entry.dedupe_key.clone());
                keys.push(other);
            }
        }
        keys
    }

    /// Settles each guid entry's key against the rows earlier fetches
    /// stored, so an item keeps one key across fetches. Whether a guid is
    /// re-keyed `guid+link:` is decided per fetch (see
    /// [`ParsedEntry::dedupe_key`]), so without this an item would switch
    /// key whenever the guid's sharing changed between fetches — and each
    /// switch would insert a duplicate article, leaving its read/star state
    /// on the old row. It also adopts the single `guid:` row a reused guid
    /// collapsed into before `guid+link:` keys existed.
    ///
    /// `stored` maps a dedupe key to the `source_url` of the article
    /// stored under it, if any (see [`Self::settle_lookups`]); `feed_url`
    /// is the `source_url` a link-less entry is stored with.
    ///
    /// - A `guid:` entry (its guid unshared in this fetch) takes the stored
    ///   `guid+link:` row for its guid and link when there is one: that key
    ///   names the item exactly, while the `guid:` row may hold another
    ///   item of a group an earlier fetch saw.
    /// - A `guid+link:` entry keeps its key when it is stored; otherwise it
    ///   adopts the stored `guid:` row for its guid only when that row's
    ///   `source_url` is the entry's own link (else the feed URL): the
    ///   plain key names just the guid, so the link says which item of the
    ///   group the row holds.
    ///
    /// Keys stay unique within the fetch.
    pub fn settle_keys<'s>(&mut self, feed_url: &str, stored: impl Fn(&str) -> Option<&'s str>) {
        let mut taken: HashSet<String> = self
            .entries
            .iter()
            .map(|entry| entry.dedupe_key.clone())
            .collect();
        for entry in &mut self.entries {
            let adopt = match alternate_key(entry) {
                Some(GuidKey::Plain(reused)) => stored(&reused).is_some().then_some(reused),
                Some(GuidKey::Reused(plain)) if stored(&entry.dedupe_key).is_none() => {
                    let source = entry.link.as_deref().unwrap_or(feed_url);
                    (stored(&plain) == Some(source)).then_some(plain)
                }
                Some(GuidKey::Reused(_)) | None => None,
            };
            if let Some(key) = adopt.filter(|key| !taken.contains(key)) {
                taken.remove(&entry.dedupe_key);
                taken.insert(key.clone());
                entry.dedupe_key = key;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::feeds::parse_feed;

    const FEED_URL: &str = "https://example.com/feed.xml";

    fn rss(items: &[(&str, &str)]) -> ParsedFeed {
        let items = items
            .iter()
            .map(|(guid, link)| {
                let link = if link.is_empty() {
                    String::new()
                } else {
                    format!("<link>{link}</link>")
                };
                format!("<item><title>t</title><guid>{guid}</guid>{link}</item>")
            })
            .collect::<Vec<_>>()
            .concat();
        let xml = format!(
            r#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>{items}</channel></rss>"#
        );
        parse_feed(xml.as_bytes(), Some(FEED_URL)).unwrap()
    }

    fn settled(mut feed: ParsedFeed, stored: &[(&str, &str)]) -> Vec<String> {
        let stored: HashMap<String, String> = stored
            .iter()
            .map(|(key, source)| ((*key).to_owned(), (*source).to_owned()))
            .collect();
        feed.settle_keys(FEED_URL, |key| stored.get(key).map(String::as_str));
        feed.entries
            .into_iter()
            .map(|entry| entry.dedupe_key)
            .collect()
    }

    const A: &str = "https://example.com/a";
    const B: &str = "https://example.com/b";

    #[test]
    fn nothing_stored_changes_nothing() {
        let feed = rss(&[("same", A), ("same", B), ("own", A)]);
        let before: Vec<String> = feed.entries.iter().map(|e| e.dedupe_key.clone()).collect();
        assert_eq!(settled(feed, &[]), before);
    }

    #[test]
    fn a_now_unshared_guid_keeps_its_stored_guid_link_key() {
        // Fetch 1 shared `same` across a and b; fetch 2 lists only b.
        let keys = settled(
            rss(&[("same", B)]),
            &[
                ("guid+link:same\u{1f}https://example.com/a", A),
                ("guid+link:same\u{1f}https://example.com/b", B),
            ],
        );
        assert_eq!(keys, ["guid+link:same\u{1f}https://example.com/b"]);
    }

    #[test]
    fn a_newly_shared_guid_adopts_the_plain_row_holding_its_link() {
        // Fetch 1 (or a pre-`guid+link:` version) stored b under `guid:same`.
        let keys = settled(rss(&[("same", A), ("same", B)]), &[("guid:same", B)]);
        assert_eq!(
            keys,
            ["guid+link:same\u{1f}https://example.com/a", "guid:same"]
        );
    }

    #[test]
    fn a_link_less_item_adopts_the_plain_row_stored_under_the_feed_url() {
        let keys = settled(
            rss(&[("same", A), ("same", "")]),
            &[("guid:same", FEED_URL)],
        );
        assert_eq!(
            keys,
            ["guid+link:same\u{1f}https://example.com/a", "guid:same"]
        );
    }

    #[test]
    fn a_stored_guid_link_key_is_kept_while_its_guid_stays_shared() {
        let keys = settled(
            rss(&[("same", A), ("same", B)]),
            &[
                ("guid:same", B),
                ("guid+link:same\u{1f}https://example.com/b", B),
            ],
        );
        assert_eq!(
            keys,
            [
                "guid+link:same\u{1f}https://example.com/a",
                "guid+link:same\u{1f}https://example.com/b"
            ]
        );
    }

    #[test]
    fn an_unshared_guid_prefers_its_exact_row_over_a_plain_row_of_another_item() {
        // `guid:same` holds b (adopted when the guid was shared); a is now
        // listed alone and must keep its own row, not overwrite b's.
        let keys = settled(
            rss(&[("same", A)]),
            &[
                ("guid:same", B),
                ("guid+link:same\u{1f}https://example.com/a", A),
            ],
        );
        assert_eq!(keys, ["guid+link:same\u{1f}https://example.com/a"]);
        // With no exact row, the plain key stands: a guid is the item's
        // identity, even when its link has changed.
        let plain = settled(rss(&[("same", A)]), &[("guid:same", B)]);
        assert_eq!(plain, ["guid:same"]);
    }

    #[test]
    fn a_plain_row_holding_another_link_is_not_adopted() {
        let keys = settled(
            rss(&[("same", A), ("same", B)]),
            &[("guid:same", "https://example.com/gone")],
        );
        assert_eq!(
            keys,
            [
                "guid+link:same\u{1f}https://example.com/a",
                "guid+link:same\u{1f}https://example.com/b"
            ]
        );
    }

    #[test]
    fn link_and_hash_tier_entries_are_never_settled() {
        let xml = br#"<?xml version="1.0"?><rss version="2.0"><channel><title>t</title>
<item><title>l</title><link>https://example.com/l</link></item>
<item><title>bare</title></item></channel></rss>"#;
        let mut feed = parse_feed(xml, None).unwrap();
        assert_eq!(feed.settle_lookups(), Vec::<String>::new());
        let before: Vec<String> = feed.entries.iter().map(|e| e.dedupe_key.clone()).collect();
        // Every key but the entries' own is "stored": nothing may be adopted.
        feed.settle_keys(FEED_URL, |key| {
            (!before.iter().any(|own| own == key)).then_some(FEED_URL)
        });
        let after: Vec<String> = feed.entries.iter().map(|e| e.dedupe_key.clone()).collect();
        assert_eq!(after, before);
    }

    #[test]
    fn settle_lookups_lists_each_guid_key_and_its_alternate() {
        let feed = rss(&[("same", A), ("same", B), ("own", A)]);
        assert_eq!(
            feed.settle_lookups(),
            [
                "guid+link:same\u{1f}https://example.com/a",
                "guid:same",
                "guid+link:same\u{1f}https://example.com/b",
                "guid:same",
                "guid:own",
                "guid+link:own\u{1f}https://example.com/a",
            ]
        );
    }
}
