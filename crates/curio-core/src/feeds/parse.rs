//! feed-rs wiring: RSS 0.x/1.0/2.0, Atom and JSON Feed → normalized
//! entries with total dedupe keys.

use std::time::Duration;

use curio_types::Timestamp;
use feed_rs::model::{Entry, Feed, Link, MediaObject};
use url::Url;

use crate::fetch::FetchResponse;
use crate::fetch::decode::feed_to_utf8;

use super::dedupe_key;
use super::image::feed_image;
use super::keys::unique_keys;

/// A feed's own metadata, as parsed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedFeedMeta {
    /// Feed title.
    pub title: Option<String>,
    /// The feed's website: `None` or an absolute `http(s)` URL (see
    /// [`ParsedEntry::link`] for the screen).
    pub site_url: Option<String>,
    /// Feed self-description.
    pub description: Option<String>,
    /// How often the feed says it changes: RSS `<ttl>` (minutes), else
    /// the syndication module's `sy:updatePeriod` / `sy:updateFrequency`.
    /// A hint the refresh schedule may stretch to, never a command.
    pub refresh_hint: Option<Duration>,
}

/// One normalized feed item. `content_html` is **raw source HTML** — it
/// must pass through [`crate::content::process`] before storage
/// (sanitize-at-ingest is a pipeline invariant, D6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedEntry {
    /// Total dedupe key (`guid → link → hash(title+published)`), unique
    /// within its [`ParsedFeed`]: a guid the feed reuses for items with
    /// different links is disambiguated to a `guid+link:` key, and an
    /// exact repeat of an earlier item is dropped. That choice sees only
    /// this fetch; [`ParsedFeed::settle_keys`] reconciles it with the keys
    /// earlier fetches stored before anything is upserted.
    pub dedupe_key: String,
    /// The source's own id/guid, when it had one.
    pub guid: Option<String>,
    /// Entry title (`(untitled)` when the source omitted it).
    pub title: String,
    /// Canonical entry URL: `None` or an absolute `http(s)` URL with a host
    /// and no userinfo — the policed client's own screen
    /// ([`crate::fetch::screen_url`]), which the opener's scheme gate passes
    /// too. A relative href was already resolved against `xml:base`, else
    /// the feed URL, while parsing. Any other declared link (`javascript:`,
    /// a custom scheme, a relative href with no base) is dropped, so the
    /// entry takes the link-less path: the feed URL as its source and
    /// content base. It never becomes the stored `source_url`, the reader's
    /// source link or the base relative content resolves against.
    pub link: Option<String>,
    /// The declared link before that screen, exactly as the feed gave it:
    /// the identity the dedupe chain's link tiers are built from. Keying on
    /// it keeps every key stable across the screen's introduction (#262) —
    /// a `link:foo://…` row stored before it keeps matching its item — and
    /// keeps items whose only link is a custom-scheme one apart.
    pub(crate) key_link: Option<String>,
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
    /// this is absent. Always an absolute `http(s)` URL. A relative
    /// `media:content` or image enclosure is resolved by feed-rs while
    /// parsing, against its `xml:base`, else the feed URL; a relative
    /// `media:thumbnail` (which feed-rs leaves raw) against the entry link,
    /// else the feed URL — the same base the entry's inline images resolve
    /// against. The content pipeline's URL policing does not run over it,
    /// so consumers treat it as untrusted and load it through the policed
    /// image cache, never directly.
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
/// The bytes are first brought to clean UTF-8 (see
/// [`crate::fetch::decode::feed_to_utf8`]): a BOM or whitespace ahead of the
/// XML declaration is dropped and a declared legacy encoding is transcoded.
/// A fetched feed should go through [`parse_fetched_feed`] instead, so the
/// HTTP `charset` gets its say too.
///
/// # Errors
///
/// [`FeedParseError`] when the bytes are not a recognizable feed.
pub fn parse_feed(bytes: &[u8], base_uri: Option<&str>) -> Result<ParsedFeed, FeedParseError> {
    parse_utf8_feed(&feed_to_utf8(bytes, None), base_uri)
}

/// [`parse_feed`] for a policed fetch's response: the body is decoded by
/// BOM, then the response's HTTP `charset`, then the XML declaration, and
/// relative URLs resolve against the response's final URL.
///
/// # Errors
///
/// [`FeedParseError`] when the body is not a recognizable feed.
pub fn parse_fetched_feed(response: &FetchResponse) -> Result<ParsedFeed, FeedParseError> {
    parse_utf8_feed(&response.feed_bytes(), Some(&response.final_url))
}

fn parse_utf8_feed(bytes: &[u8], base_uri: Option<&str>) -> Result<ParsedFeed, FeedParseError> {
    let parser = feed_rs::parser::Builder::new()
        .base_uri(base_uri)
        // Keep absent ids absent: our dedupe chain owns the fallback.
        .id_generator(|_links, _title, _uri| String::new())
        .build();
    let feed = parser.parse(bytes)?;
    let base = base_uri.and_then(|uri| Url::parse(uri).ok());
    let mut parsed = normalize(feed, base.as_ref());
    if parsed.meta.refresh_hint.is_none() {
        parsed.meta.refresh_hint = syndication_period(bytes);
    }
    Ok(parsed)
}

fn normalize(feed: Feed, base: Option<&Url>) -> ParsedFeed {
    let meta = ParsedFeedMeta {
        title: feed.title.map(|t| t.content),
        site_url: best_link(&feed.links).and_then(web_link),
        description: feed.description.map(|t| t.content),
        refresh_hint: feed
            .ttl
            .filter(|&minutes| minutes > 0)
            .map(|minutes| Duration::from_secs(u64::from(minutes) * 60)),
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

fn normalize_entry(entry: Entry, feed_base: Option<&Url>) -> ParsedEntry {
    let guid = Some(entry.id.trim())
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned);
    let title = entry
        .title
        .map(|t| t.content)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| "(untitled)".to_owned());
    let key_link = best_link(&entry.links).map(ToOwned::to_owned);
    let link = key_link.as_deref().and_then(web_link);
    let author = entry
        .authors
        .into_iter()
        .map(|person| person.name)
        .find(|name| !name.trim().is_empty());
    let source_published = entry.published.map(Timestamp::new);
    let updated = entry.updated.map(Timestamp::new);
    let published = source_published.or(updated);
    // A relative `media:thumbnail` resolves against the entry's own link,
    // else the feed URL (feed-rs has already resolved the other image
    // signals — see `feed_image`).
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
    let dedupe_key = dedupe_key(
        guid.as_deref(),
        key_link.as_deref(),
        &title,
        source_published,
    );
    ParsedEntry {
        dedupe_key,
        guid,
        title,
        link,
        key_link,
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

/// The entry/site link as declared — unscreened; [`web_link`] screens it:
/// prefer an explicit `alternate` (or rel-less) link over
/// `self`/`enclosure`/anything else.
fn best_link(links: &[Link]) -> Option<&str> {
    links
        .iter()
        .find(|link| link.rel.is_none() || link.rel.as_deref() == Some("alternate"))
        .or_else(|| links.first())
        .map(|link| link.href.as_str())
}

/// `href` when the app may store, link to and fetch it: an absolute
/// `http(s)` URL with a host and no userinfo ([`crate::fetch::screen_url`],
/// the policed client's own screen — pure, no I/O). feed-rs has already
/// resolved a relative href against `xml:base` / the feed URL, so one still
/// relative here had no base and is dropped too. The href is kept as given
/// (feed-rs serialized any it could parse), so a link that passes is stored
/// byte-for-byte as before the screen existed.
fn web_link(href: &str) -> Option<String> {
    crate::fetch::screen_url(href)
        .is_ok()
        .then(|| href.to_owned())
}

/// The syndication module's namespace.
const SYNDICATION_NS: &str = "http://purl.org/rss/1.0/modules/syndication/";

/// The RSS 1.0 syndication module's update period (`sy:updatePeriod`,
/// divided by `sy:updateFrequency`, default 1), which feed-rs does not
/// surface. Read from the channel head only — before the first item —
/// under whatever prefix the document binds the module's namespace to
/// (`sy` when it binds none).
fn syndication_period(bytes: &[u8]) -> Option<Duration> {
    let head = channel_head(bytes);
    let head = String::from_utf8_lossy(head);
    let prefix = namespace_prefix(&head, SYNDICATION_NS).unwrap_or("sy");
    let element = |name: &str| {
        let open = format!("<{prefix}:{name}>");
        let start = head.find(&open)? + open.len();
        let len = head.get(start..)?.find('<')?;
        head.get(start..start + len).map(str::trim)
    };
    let period_secs: u64 = match element("updatePeriod")? {
        "hourly" => 3_600,
        "daily" => 86_400,
        "weekly" => 604_800,
        "monthly" => 2_592_000,
        "yearly" => 31_536_000,
        _ => return None,
    };
    let frequency = element("updateFrequency")
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(1);
    Some(Duration::from_secs(period_secs / frequency))
}

/// The document up to its first `<item>` / `<entry>` start tag — the
/// channel's own elements. An element that merely starts with those
/// letters (RSS 1.0's channel-level `<items>`) does not end it.
fn channel_head(bytes: &[u8]) -> &[u8] {
    let ends_name = |byte: Option<&u8>| {
        byte.is_none_or(|&byte| byte == b'>' || byte == b'/' || byte.is_ascii_whitespace())
    };
    (0..bytes.len())
        .find(|&at| {
            [b"<item".as_slice(), b"<entry".as_slice()]
                .iter()
                .any(|tag| {
                    bytes.get(at..).is_some_and(|rest| rest.starts_with(tag))
                        && ends_name(bytes.get(at + tag.len()))
                })
        })
        .and_then(|end| bytes.get(..end))
        .unwrap_or(bytes)
}

/// The prefix `xml` binds to `namespace` (`xmlns:p="…"`), if any.
fn namespace_prefix<'a>(xml: &'a str, namespace: &str) -> Option<&'a str> {
    xml.match_indices("xmlns:").find_map(|(at, _)| {
        let rest = xml.get(at + "xmlns:".len()..)?;
        let (prefix, value) = rest.split_once('=')?;
        let value = value.trim_start();
        let quote = value.chars().next().filter(|&c| c == '"' || c == '\'')?;
        let value = value.get(1..)?;
        let uri = value.get(..value.find(quote)?)?;
        (uri == namespace).then(|| prefix.trim())
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn ttl_and_the_syndication_module_are_refresh_hints() {
        let rss = |channel_extra: &str| {
            format!(
                r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:sy="http://purl.org/rss/1.0/modules/syndication/">
<channel><title>t</title><link>https://example.com/</link>{channel_extra}
<item><title>a</title><link>https://example.com/a</link>
<description>&lt;sy:updatePeriod&gt;daily&lt;/sy:updatePeriod&gt;</description></item>
</channel></rss>"#
            )
        };
        let hint = |extra: &str| {
            parse_feed(rss(extra).as_bytes(), None)
                .unwrap()
                .meta
                .refresh_hint
        };
        assert_eq!(hint("<ttl>90</ttl>"), Some(Duration::from_secs(90 * 60)));
        assert_eq!(
            hint(
                "<sy:updatePeriod>hourly</sy:updatePeriod><sy:updateFrequency>2</sy:updateFrequency>"
            ),
            Some(Duration::from_secs(1_800))
        );
        assert_eq!(
            hint("<sy:updatePeriod> daily </sy:updatePeriod>"),
            Some(Duration::from_secs(86_400))
        );
        assert_eq!(
            hint("<ttl>60</ttl><sy:updatePeriod>weekly</sy:updatePeriod>"),
            Some(Duration::from_secs(3_600)),
            "<ttl> wins"
        );
        assert_eq!(hint("<ttl>0</ttl>"), None);
        assert_eq!(hint("<sy:updatePeriod>sometimes</sy:updatePeriod>"), None);
        assert_eq!(hint(""), None, "an item's text is not the channel's hint");
    }

    #[test]
    fn the_syndication_hint_follows_the_bound_prefix_past_items() {
        // RSS 1.0: the channel's `<items>` list comes before the
        // syndication elements, and the namespace has its own prefix.
        let rdf = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"
  xmlns="http://purl.org/rss/1.0/" xmlns:syn = 'http://purl.org/rss/1.0/modules/syndication/'>
<channel rdf:about="https://example.com/"><title>t</title><link>https://example.com/</link>
<description>d</description>
<items><rdf:Seq><rdf:li rdf:resource="https://example.com/a"/></rdf:Seq></items>
<syn:updatePeriod>daily</syn:updatePeriod><syn:updateFrequency>4</syn:updateFrequency>
</channel>
<item rdf:about="https://example.com/a"><title>a</title><link>https://example.com/a</link></item>
</rdf:RDF>"#;
        assert_eq!(
            parse_feed(rdf.as_bytes(), None).unwrap().meta.refresh_hint,
            Some(Duration::from_secs(21_600))
        );
        assert_eq!(
            namespace_prefix("<a xmlns:x=\"other\">", SYNDICATION_NS),
            None
        );
        assert_eq!(channel_head(b"<a><item>"), b"<a>");
        assert_eq!(channel_head(b"<a><entry/>"), b"<a>");
        assert_eq!(channel_head(b"<a><itemize>"), b"<a><itemize>");
    }

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

    /// #262: only an absolute http(s) link survives as the entry's link or
    /// the feed's site; everything else reads as no link.
    #[test]
    fn entry_and_channel_links_are_screened_to_http() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title><link>javascript:alert(1)</link>
<item><title>js</title><link>javascript:alert(1)</link></item>
<item><title>spaced js</title><guid>g-js</guid><link>  JaVaScRiPt:alert(2)</link></item>
<item><title>custom</title><guid>g-custom</guid><link>foo://h/dir/three</link></item>
<item><title>lookalike</title><guid>g-like</guid><link>httpsupdated://xss.example.com/one</link></item>
<item><title>ftp</title><guid>g-ftp</guid><link>ftp://files.example/pub/two</link></item>
<item><title>data</title><guid>g-data</guid><link>data:text/html,x</link></item>
<item><title>userinfo</title><guid>g-user</guid><link>https://user:pw@example.com/u</link></item>
<item><title>hostless</title><guid>g-hostless</guid><link>file:///etc/passwd</link></item>
<item><title>relative</title><guid>g-rel</guid><link>/posts/rel</link></item>
<item><title>web</title><guid>g-web</guid><link>https://example.com/ok</link></item>
<item><title>plain web</title><guid>g-plain</guid><link>http://example.com/plain</link></item>
</channel></rss>"#;
        let feed = parse_feed(xml, None).unwrap();
        assert_eq!(feed.meta.site_url, None);
        let links: Vec<(&str, Option<&str>)> = feed
            .entries
            .iter()
            .map(|entry| (entry.title.as_str(), entry.link.as_deref()))
            .collect();
        assert_eq!(
            links,
            [
                ("js", None),
                ("spaced js", None),
                ("custom", None),
                ("lookalike", None),
                ("ftp", None),
                ("data", None),
                ("userinfo", None),
                ("hostless", None),
                ("relative", None),
                ("web", Some("https://example.com/ok")),
                ("plain web", Some("http://example.com/plain")),
            ]
        );
    }

    #[test]
    fn a_relative_link_resolved_against_the_feed_url_is_kept() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title><link>/</link>
<item><title>rel</title><link>/posts/rel</link></item>
</channel></rss>"#;
        let feed = parse_feed(xml, Some("https://example.com/feed.xml")).unwrap();
        assert_eq!(feed.meta.site_url.as_deref(), Some("https://example.com/"));
        assert_eq!(
            feed.entries[0].link.as_deref(),
            Some("https://example.com/posts/rel")
        );
        assert_eq!(
            feed.entries[0].dedupe_key,
            "link:https://example.com/posts/rel"
        );
    }

    /// The screen never moves a dedupe key: a row stored under a link the
    /// screen now drops keeps matching its item, and items told apart only
    /// by such links stay apart.
    #[test]
    fn a_screened_out_link_still_keys_its_entry() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title>
<item><title>js</title><link>javascript:alert(1)</link></item>
<item><title>one</title><guid>same</guid><link>foo://h/1</link></item>
<item><title>two</title><guid>same</guid><link>foo://h/2</link></item>
</channel></rss>"#;
        let feed = parse_feed(xml, Some("https://example.com/feed.xml")).unwrap();
        let keys: Vec<&str> = feed
            .entries
            .iter()
            .map(|entry| entry.dedupe_key.as_str())
            .collect();
        assert_eq!(
            keys,
            [
                "link:javascript:alert(1)",
                "guid+link:same\u{1f}foo://h/1",
                "guid+link:same\u{1f}foo://h/2",
            ]
        );
        assert!(feed.entries.iter().all(|entry| entry.link.is_none()));
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
