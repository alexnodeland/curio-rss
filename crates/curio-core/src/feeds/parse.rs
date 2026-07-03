//! feed-rs wiring: RSS 0.x/1.0/2.0, Atom and JSON Feed → normalized
//! entries with total dedupe keys.

use curio_types::Timestamp;
use feed_rs::model::{Entry, Feed, Link};

use super::dedupe_key;

/// A feed's own metadata, as parsed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFeedMeta {
    /// Feed title.
    pub title: Option<String>,
    /// The feed's website.
    pub site_url: Option<String>,
    /// Feed self-description.
    pub description: Option<String>,
}

/// One normalized feed item. `content_html` is **raw source HTML** — it
/// must pass through [`crate::content::process`] before storage
/// (sanitize-at-ingest is a pipeline invariant, D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    /// Total dedupe key (`guid → link → hash(title+published)`).
    pub dedupe_key: String,
    /// The source's own id/guid, when it had one.
    pub guid: Option<String>,
    /// Entry title (`(untitled)` when the source omitted it).
    pub title: String,
    /// Canonical entry URL, if any.
    pub link: Option<String>,
    /// First author name, if any.
    pub author: Option<String>,
    /// Publication time, if the source carried one.
    pub published: Option<Timestamp>,
    /// The source's own last-update time, if distinct.
    pub updated: Option<Timestamp>,
    /// Raw content HTML (content body, else summary, else empty).
    pub content_html: String,
    /// Entry language, if declared.
    pub lang: Option<String>,
}

/// A parsed feed: metadata plus normalized entries in document order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFeed {
    /// Feed-level metadata.
    pub meta: ParsedFeedMeta,
    /// Normalized entries.
    pub entries: Vec<ParsedEntry>,
}

/// Feed parse failures.
#[derive(Debug, thiserror::Error)]
#[error("feed parse: {0}")]
pub struct FeedParseError(#[from] feed_rs::parser::ParseFeedError);

/// Parses feed bytes (RSS/Atom/JSON Feed — format auto-detected) into a
/// [`ParsedFeed`]. `base_uri` (the fetch URL) resolves relative links.
///
/// The dedupe chain is *ours*, not feed-rs's: its id generator is
/// disabled (it falls back to random UUIDs, which would break dedupe
/// stability across fetches), so a missing guid falls through to the
/// `link → hash` tiers deterministically.
///
/// # Errors
///
/// [`FeedParseError`] when the bytes are not a recognizable feed.
pub fn parse_feed(bytes: &[u8], base_uri: Option<&str>) -> Result<ParsedFeed, FeedParseError> {
    let parser = feed_rs::parser::Builder::new()
        .base_uri(base_uri)
        // Keep absent ids absent: our dedupe chain owns the fallback.
        .id_generator(|_links, _title, _uri| String::new())
        .build();
    let feed = parser.parse(bytes)?;
    Ok(normalize(feed))
}

fn normalize(feed: Feed) -> ParsedFeed {
    let meta = ParsedFeedMeta {
        title: feed.title.map(|t| t.content),
        site_url: best_link(&feed.links).map(ToOwned::to_owned),
        description: feed.description.map(|t| t.content),
    };
    let entries = feed.entries.into_iter().map(normalize_entry).collect();
    ParsedFeed { meta, entries }
}

fn normalize_entry(entry: Entry) -> ParsedEntry {
    let guid = Some(entry.id.trim())
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned);
    let title = entry
        .title
        .map(|t| t.content)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "(untitled)".to_owned());
    let link = best_link(&entry.links).map(ToOwned::to_owned);
    let author = entry
        .authors
        .into_iter()
        .map(|person| person.name)
        .find(|name| !name.trim().is_empty());
    let published = entry.published.map(Timestamp::new);
    let updated = entry.updated.map(Timestamp::new);
    let content_html = entry
        .content
        .and_then(|content| content.body)
        .or(entry.summary.map(|summary| summary.content))
        .unwrap_or_default();
    let dedupe_key = dedupe_key(guid.as_deref(), link.as_deref(), &title, published);
    ParsedEntry {
        dedupe_key,
        guid,
        title,
        link,
        author,
        published,
        updated,
        content_html,
        lang: entry.language,
    }
}

/// The entry/site link: prefer an explicit `alternate` (or rel-less)
/// link over `self`/`enclosure`/anything else.
fn best_link(links: &[Link]) -> Option<&str> {
    links
        .iter()
        .find(|link| link.rel.is_none() || link.rel.as_deref() == Some("alternate"))
        .or_else(|| links.first())
        .map(|link| link.href.as_str())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn fixture(name: &str) -> Vec<u8> {
        let path = format!("{}/../../fixtures/feeds/{name}", env!("CARGO_MANIFEST_DIR"));
        std::fs::read(path).unwrap()
    }

    #[test]
    fn parses_rss2_with_the_full_dedupe_chain() {
        let feed = parse_feed(&fixture("rss2.xml"), Some("https://example.com/feed.xml")).unwrap();
        assert_eq!(feed.meta.title.as_deref(), Some("Example Blog"));
        assert_eq!(feed.meta.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(
            feed.meta.description.as_deref(),
            Some("Writing about examples.")
        );
        assert_eq!(feed.entries.len(), 3);

        let [with_guid, no_guid, bare] = feed.entries.as_slice() else {
            unreachable!("asserted 3 entries above");
        };
        assert_eq!(with_guid.dedupe_key, "guid:urn:example:post-1");
        assert!(
            with_guid.content_html.contains("<strong>content</strong>"),
            "content:encoded wins over description"
        );
        assert_eq!(no_guid.dedupe_key, "link:https://example.com/posts/no-guid");
        assert!(bare.dedupe_key.starts_with("hash:"), "hash tier is total");
        assert_eq!(bare.title, "Neither guid nor link");
        assert!(with_guid.published.is_some());
    }

    #[test]
    fn rss2_dedupe_keys_are_stable_across_reparses() {
        let bytes = fixture("rss2.xml");
        let a = parse_feed(&bytes, None).unwrap();
        let b = parse_feed(&bytes, None).unwrap();
        let keys = |feed: &ParsedFeed| {
            feed.entries
                .iter()
                .map(|e| e.dedupe_key.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(keys(&a), keys(&b), "no random ids may leak into dedupe");
    }

    #[test]
    fn parses_atom() {
        let feed = parse_feed(&fixture("atom.xml"), None).unwrap();
        assert_eq!(feed.meta.title.as_deref(), Some("Atom Example"));
        assert_eq!(
            feed.meta.site_url.as_deref(),
            Some("https://atom.example.org/"),
            "rel=self must not win"
        );
        assert_eq!(feed.entries.len(), 2);
        let one = &feed.entries[0];
        assert_eq!(one.dedupe_key, "guid:urn:example:atom-1");
        assert_eq!(one.author.as_deref(), Some("Ada Lovelace"));
        assert!(one.content_html.contains("<em>content</em>"));
        assert!(one.published.is_some());
        let two = &feed.entries[1];
        assert_eq!(two.content_html, "Only a summary here.");
    }

    #[test]
    fn parses_json_feed() {
        let feed = parse_feed(&fixture("feed.json"), None).unwrap();
        assert_eq!(feed.meta.title.as_deref(), Some("JSON Feed Example"));
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[0].dedupe_key, "guid:jf-1");
        assert!(
            feed.entries[0]
                .content_html
                .contains("<strong>html</strong>")
        );
        assert_eq!(feed.entries[0].author.as_deref(), Some("Grace Hopper"));
    }

    #[test]
    fn garbage_is_a_parse_error_not_a_panic() {
        assert!(parse_feed(b"not a feed at all", None).is_err());
        assert!(parse_feed(b"", None).is_err());
    }
}
