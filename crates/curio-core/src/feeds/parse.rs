//! feed-rs wiring: RSS 0.x/1.0/2.0, Atom and JSON Feed → normalized
//! entries with total dedupe keys.

use curio_types::Timestamp;
use feed_rs::model::{Entry, Feed, Link, MediaObject};

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
    /// A lead image URL declared in the feed metadata itself
    /// (`media:thumbnail` → `media:content` image → image `enclosure`),
    /// when the source carried one. This is the *first-class* RSS image
    /// signal; ingest falls back to the first inline `<img>` only when
    /// this is absent. Left as the source declared it (absolute or
    /// relative); the content pipeline's URL policing does not run over
    /// it, so consumers treat it as untrusted and load it through the
    /// policed image cache, never directly.
    pub lead_image: Option<String>,
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
    let lead_image = feed_image(&entry.media, &entry.links);
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
        lead_image,
    }
}

/// The feed's own declared lead image for an entry, if any, preferring the
/// most explicit signal: a `media:thumbnail`, then an image `media:content`,
/// then an image `enclosure`. Only absolute `http(s)` URLs qualify — a
/// relative or scheme-less reference can't be fetched by the image cache
/// (which has no base to resolve against), so it's dropped here rather than
/// stored as an un-loadable value. The inline-`<img>` fallback lives in
/// ingest, over the base-resolved, sanitized body.
fn feed_image(media: &[MediaObject], links: &[Link]) -> Option<String> {
    let thumbnail = || {
        media
            .iter()
            .flat_map(|object| object.thumbnails.iter())
            .map(|thumb| thumb.image.uri.trim())
            .find(|uri| is_http_url(uri))
            .map(ToOwned::to_owned)
    };
    let content = || {
        media
            .iter()
            .flat_map(|object| object.content.iter())
            .filter(|content| {
                content
                    .content_type
                    .as_ref()
                    .is_some_and(|mime| mime.ty().as_str() == "image")
            })
            .filter_map(|content| content.url.as_ref())
            .map(ToString::to_string)
            .find(|uri| is_http_url(uri))
    };
    let enclosure = || {
        links
            .iter()
            .filter(|link| link.rel.as_deref() == Some("enclosure"))
            .filter(|link| {
                link.media_type
                    .as_deref()
                    .is_some_and(|mime| mime.starts_with("image/"))
            })
            .map(|link| link.href.trim())
            .find(|uri| is_http_url(uri))
            .map(ToOwned::to_owned)
    };
    thumbnail().or_else(content).or_else(enclosure)
}

/// Whether `uri` is an absolute `http`/`https` URL — the only shape the
/// image cache can fetch.
fn is_http_url(uri: &str) -> bool {
    uri.starts_with("http://") || uri.starts_with("https://")
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
    fn lead_image_prefers_media_thumbnail_over_enclosure() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
<channel><title>t</title>
<item><title>a</title><guid>g1</guid>
<media:thumbnail url="https://cdn.example.com/thumb.jpg"/>
<enclosure url="https://cdn.example.com/clip.mp3" type="audio/mpeg" length="1"/>
</item></channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://cdn.example.com/thumb.jpg"),
            "a media:thumbnail is the strongest signal"
        );
    }

    #[test]
    fn lead_image_takes_an_image_enclosure_when_no_thumbnail() {
        // feed-rs models an RSS <enclosure> as a MediaContent, so an image
        // enclosure surfaces through the media:content path.
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0">
<channel><title>t</title>
<item><title>a</title><guid>g2</guid>
<enclosure url="https://cdn.example.com/pic.jpg" type="image/jpeg" length="1"/>
</item></channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://cdn.example.com/pic.jpg")
        );
    }

    #[test]
    fn lead_image_ignores_a_non_image_enclosure() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0">
<channel><title>t</title>
<item><title>a</title><guid>g3</guid>
<enclosure url="https://cdn.example.com/clip.mp3" type="audio/mpeg" length="1"/>
</item></channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        assert_eq!(
            feed.entries[0].lead_image, None,
            "an audio enclosure is not a lead image"
        );
    }

    #[test]
    fn lead_image_takes_an_atom_image_enclosure_link() {
        let xml = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
<title>t</title>
<entry><id>a1</id><title>a</title>
<link rel="alternate" href="https://example.com/a"/>
<link rel="enclosure" type="image/png" href="https://cdn.example.com/a.png"/>
</entry></feed>"#;
        let feed = parse_feed(xml, None).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://cdn.example.com/a.png")
        );
    }

    #[test]
    fn is_http_url_requires_an_absolute_http_scheme() {
        assert!(is_http_url("https://example.com/x.jpg"));
        assert!(is_http_url("http://example.com/x.jpg"));
        assert!(!is_http_url("/relative/x.jpg"));
        assert!(!is_http_url("data:image/png;base64,AAAA"));
        assert!(!is_http_url("//protocol.relative/x.jpg"));
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
