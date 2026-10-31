//! feed-rs wiring: RSS 0.x/1.0/2.0, Atom and JSON Feed → normalized
//! entries with total dedupe keys.

use std::collections::{HashMap, HashSet};

use curio_types::Timestamp;
use feed_rs::model::{Entry, Feed, Link, MediaObject};
use url::Url;

use super::image::feed_image;
use super::{dedupe_key, guid_link_key};

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
    /// Total dedupe key (`guid → link → hash(title+published)`), unique
    /// within its [`ParsedFeed`]: a guid the feed reuses for items with
    /// different links is disambiguated to a `guid+link:` key, and an
    /// exact repeat of an earlier item is dropped.
    pub dedupe_key: String,
    /// The source's own id/guid, when it had one.
    pub guid: Option<String>,
    /// Entry title (`(untitled)` when the source omitted it).
    pub title: String,
    /// Canonical entry URL, if any.
    pub link: Option<String>,
    /// First author name, if any.
    pub author: Option<String>,
    /// Publication time: the source's `published`, else its `updated`
    /// (an Atom entry may legally carry only `<updated>`). This is the
    /// source's raw value — a future date is not clamped here; storage
    /// clamps it to the article's first-seen time.
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
    /// this is absent. Always an absolute `http(s)` URL: a relative
    /// reference is resolved against its `xml:base` (where feed-rs applies
    /// one), else the entry link, else the feed URL. The content
    /// pipeline's URL policing does not run over it, so consumers treat it
    /// as untrusted and load it through the policed image cache, never
    /// directly.
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
/// [`ParsedFeed`]. `base_uri` (the fetch URL) resolves relative links and
/// relative lead-image references.
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
    let base = base_uri.and_then(|uri| Url::parse(uri).ok());
    Ok(normalize(feed, base.as_ref()))
}

fn normalize(feed: Feed, base: Option<&Url>) -> ParsedFeed {
    let meta = ParsedFeedMeta {
        title: feed.title.map(|t| t.content),
        site_url: best_link(&feed.links).map(ToOwned::to_owned),
        description: feed.description.map(|t| t.content),
    };
    let entries = feed
        .entries
        .into_iter()
        .map(|entry| normalize_entry(entry, base))
        .collect();
    ParsedFeed {
        meta,
        entries: unique_keys(entries),
    }
}

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
fn unique_keys(entries: Vec<ParsedEntry>) -> Vec<ParsedEntry> {
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

fn normalize_entry(entry: Entry, feed_base: Option<&Url>) -> ParsedEntry {
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
    let source_published = entry.published.map(Timestamp::new);
    let updated = entry.updated.map(Timestamp::new);
    let published = source_published.or(updated);
    // A relative image reference resolves against the entry's own link,
    // else the feed URL.
    let entry_base = link.as_deref().and_then(|link| Url::parse(link).ok());
    let lead_image = feed_image(
        &entry.media,
        &entry.links,
        entry_base.as_ref().or(feed_base),
    );
    // content → summary → media:description. YouTube (and other media feeds)
    // carry the human text only in `media:description`, leaving content and
    // summary empty, so without the last fallback the reader body is blank.
    let content_html = entry
        .content
        .and_then(|content| content.body)
        .filter(|body| !body.trim().is_empty())
        .or_else(|| entry.summary.map(|summary| summary.content))
        .filter(|body| !body.trim().is_empty())
        .or_else(|| media_description_html(&entry.media))
        .unwrap_or_default();
    // The hash tier keeps hashing the source's own `published`: folding the
    // `updated` fallback in would re-key every date-less, link-less item —
    // once now, and again on each edit — duplicating it in the list.
    let dedupe_key = dedupe_key(guid.as_deref(), link.as_deref(), &title, source_published);
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

/// The first non-empty `media:description` across the entry's media objects,
/// HTML-escaped and wrapped in a `<p>`. It's plain text (PCDATA), so it is
/// escaped rather than trusted as markup — and it still passes the ingest
/// sanitizer (D6) unchanged, since escaped text carries no live HTML.
fn media_description_html(media: &[MediaObject]) -> Option<String> {
    media
        .iter()
        .filter_map(|object| object.description.as_ref())
        .map(|text| text.content.trim())
        .find(|content| !content.is_empty())
        .map(|content| format!("<p>{}</p>", escape_html_text(content)))
}

/// Escapes plain text for safe inclusion as HTML element content.
fn escape_html_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
    }
    out
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

    /// The crate dir is read at run time (cargo sets it for test runs): a
    /// compile-time path goes stale when a shared target dir reuses this
    /// binary from another worktree (issue #183).
    fn fixture(name: &str) -> Vec<u8> {
        let crate_dir = std::env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("CARGO_MANIFEST_DIR unset: run the tests through cargo"));
        std::fs::read(
            std::path::Path::new(&crate_dir)
                .join("../../fixtures/feeds")
                .join(name),
        )
        .unwrap()
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
    fn youtube_atom_falls_back_to_media_description() {
        let feed = parse_feed(&fixture("youtube-atom.xml"), None).unwrap();
        assert_eq!(feed.entries.len(), 2);

        // Entry 1: no content/summary, so media:description fills the body —
        // wrapped in <p>, HTML-escaped (the & and <tag> survive as entities).
        let with_desc = &feed.entries[0];
        assert!(
            !with_desc.content_html.is_empty(),
            "media:description must fill an otherwise-empty YouTube body"
        );
        assert!(with_desc.content_html.starts_with("<p>"));
        assert!(
            with_desc
                .content_html
                .contains("human-written video description")
        );
        assert!(
            with_desc.content_html.contains("&amp; ampersand")
                && with_desc.content_html.contains("&lt;tag-like&gt;"),
            "plain-text description must be HTML-escaped, not trusted as markup"
        );
        // The lead image still comes from media:thumbnail.
        assert_eq!(
            with_desc.lead_image.as_deref(),
            Some("https://i4.ytimg.com/vi/VIDEO0000001/hqdefault.jpg")
        );

        // Entry 2: no description anywhere → body stays empty (no phantom <p>).
        let no_desc = &feed.entries[1];
        assert_eq!(no_desc.content_html, "");
    }

    #[test]
    fn media_description_survives_the_ingest_sanitizer() {
        // The escaped fallback must pass ammonia unchanged — escaped entities
        // are inert text, so the description text survives to the reader (D6).
        let feed = parse_feed(&fixture("youtube-atom.xml"), None).unwrap();
        let raw = &feed.entries[0].content_html;
        let clean = crate::content::sanitize(raw, None);
        assert!(
            clean.contains("human-written video description"),
            "the description text must survive sanitization"
        );
    }

    #[test]
    fn an_updated_only_atom_entry_takes_updated_as_its_date() {
        // atom.xml entry 2 carries only <updated> — spec-legal Atom.
        let feed = parse_feed(&fixture("atom.xml"), None).unwrap();
        let two = &feed.entries[1];
        let updated: Timestamp = "2026-07-02T11:00:00Z".parse().unwrap();
        assert_eq!(
            two.published,
            Some(updated),
            "published falls back to updated"
        );
        assert_eq!(two.updated, Some(updated));
        // An entry carrying both keeps its own published.
        let one = &feed.entries[0];
        assert_eq!(one.published, Some("2026-07-01T09:00:00Z".parse().unwrap()));
    }

    #[test]
    fn the_updated_fallback_never_moves_a_hash_tier_key() {
        // No guid, no link, no published: the hash tier must hash the
        // source's (absent) published, not the updated fallback — or every
        // edit of the entry would re-key it into a duplicate article.
        let atom = |updated: &str| {
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title>
<entry><title>Bare</title><updated>{updated}</updated></entry></feed>"#
            )
        };
        let first = parse_feed(atom("2026-07-01T00:00:00Z").as_bytes(), None).unwrap();
        let edited = parse_feed(atom("2026-07-09T00:00:00Z").as_bytes(), None).unwrap();
        let entry = &first.entries[0];
        assert!(entry.dedupe_key.starts_with("hash:"));
        assert!(entry.published.is_some());
        assert_eq!(entry.dedupe_key, dedupe_key(None, None, "Bare", None));
        assert_eq!(entry.dedupe_key, edited.entries[0].dedupe_key);
    }

    #[test]
    fn a_guid_reused_across_different_links_keeps_every_item() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title>
<item><title>a</title><guid>same</guid><link>https://example.com/a</link></item>
<item><title>b</title><guid>same</guid><link>https://example.com/b</link></item>
<item><title>c</title><guid>same</guid><link>https://example.com/c</link></item>
<item><title>d</title><guid>own</guid><link>https://example.com/d</link></item>
</channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        let keys: Vec<&str> = feed
            .entries
            .iter()
            .map(|entry| entry.dedupe_key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "guid+link:same\u{1f}https://example.com/a",
                "guid+link:same\u{1f}https://example.com/b",
                "guid+link:same\u{1f}https://example.com/c",
                "guid:own",
            ],
            "every item of the reused guid is re-keyed (feed order can't matter); \
             an unshared guid keeps its plain key"
        );
        assert_eq!(feed.entries[1].guid.as_deref(), Some("same"), "guid kept");
    }

    #[test]
    fn an_item_listed_twice_is_kept_once() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title>
<item><title>first</title><guid>g</guid><link>https://example.com/a</link></item>
<item><title>repeat</title><guid>g</guid><link>https://example.com/a</link></item>
<item><title>linked</title><link>https://example.com/l</link></item>
<item><title>linked again</title><link>https://example.com/l</link></item>
<item><title>other</title><guid>h</guid><link>https://example.com/h</link></item>
</channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        let titles: Vec<&str> = feed
            .entries
            .iter()
            .map(|entry| entry.title.as_str())
            .collect();
        assert_eq!(
            titles,
            ["first", "linked", "other"],
            "the first listing wins"
        );
        assert_eq!(feed.entries[0].dedupe_key, "guid:g");
    }

    #[test]
    fn a_reused_guid_with_a_link_less_item_still_disambiguates() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title>
<item><title>a</title><guid>same</guid><link>https://example.com/a</link></item>
<item><title>b</title><guid>same</guid></item>
</channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        assert_eq!(feed.entries.len(), 2);
        assert_eq!(feed.entries[1].dedupe_key, "guid+link:same\u{1f}");
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
