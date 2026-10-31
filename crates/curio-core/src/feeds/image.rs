//! The feed's own declared lead image for an entry: `media:thumbnail` →
//! image `media:content` → image `enclosure`, resolved to an absolute
//! `http(s)` URL the policed image cache can load.

use feed_rs::model::{Link, MediaObject};
use url::Url;

/// The feed's own declared lead image for an entry, if any, preferring the
/// most explicit signal: a `media:thumbnail`, then an image `media:content`,
/// then an image `enclosure`. A relative reference is resolved against
/// `base` (the entry link, else the feed URL); feed-rs has already applied
/// any `xml:base` to `media:content` URLs and links, but hands
/// `media:thumbnail` over raw. Only an absolute `http(s)` result qualifies:
/// anything else can't be fetched by the image cache, so it's dropped here
/// rather than stored as an un-loadable value. The inline-`<img>` fallback
/// lives in ingest, over the base-resolved, sanitized body.
pub(super) fn feed_image(
    media: &[MediaObject],
    links: &[Link],
    base: Option<&Url>,
) -> Option<String> {
    let thumbnail = || {
        media
            .iter()
            .flat_map(|object| object.thumbnails.iter())
            .find_map(|thumb| resolve_http_url(&thumb.image.uri, base))
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
            .find_map(|uri| resolve_http_url(uri.as_str(), base))
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
            .find_map(|link| resolve_http_url(&link.href, base))
    };
    thumbnail().or_else(content).or_else(enclosure)
}

/// `uri` as an absolute `http(s)` URL — itself when already absolute, else
/// joined onto `base` — or `None` when it can't become one (no base, a
/// `data:` URI, unparseable).
fn resolve_http_url(uri: &str, base: Option<&Url>) -> Option<String> {
    let uri = uri.trim();
    if uri.is_empty() {
        return None;
    }
    let resolved = match Url::parse(uri) {
        Ok(absolute) => absolute,
        Err(url::ParseError::RelativeUrlWithoutBase) => base?.join(uri).ok()?,
        Err(_) => return None,
    };
    let resolved = String::from(resolved);
    is_http_url(&resolved).then_some(resolved)
}

/// Whether `uri` is an absolute `http`/`https` URL — the only shape the
/// image cache can fetch.
fn is_http_url(uri: &str) -> bool {
    uri.starts_with("http://") || uri.starts_with("https://")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::feeds::parse_feed;

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
    fn a_relative_thumbnail_resolves_against_the_entry_link() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
<channel><title>t</title>
<item><title>a</title><guid>g1</guid><link>https://blog.example.com/posts/1/</link>
<media:thumbnail url="thumb.jpg"/></item>
<item><title>b</title><guid>g2</guid><link>https://blog.example.com/posts/2/</link>
<media:thumbnail url="/img/root.jpg"/></item>
</channel></rss>"#;
        let feed = parse_feed(xml, Some("https://feeds.example.net/rss.xml")).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://blog.example.com/posts/1/thumb.jpg")
        );
        assert_eq!(
            feed.entries[1].lead_image.as_deref(),
            Some("https://blog.example.com/img/root.jpg")
        );
    }

    #[test]
    fn a_relative_thumbnail_of_a_link_less_item_resolves_against_the_feed_url() {
        let xml = br#"<?xml version="1.0"?>
<rss version="2.0" xmlns:media="http://search.yahoo.com/mrss/">
<channel><title>t</title>
<item><title>a</title><guid>g1</guid><media:thumbnail url="/img/t.jpg"/></item>
</channel></rss>"#;
        let feed = parse_feed(xml, Some("https://feeds.example.net/rss.xml")).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://feeds.example.net/img/t.jpg")
        );
        // With no base anywhere, a relative reference is still dropped.
        let unresolvable = parse_feed(xml, None).unwrap();
        assert_eq!(unresolvable.entries[0].lead_image, None);
    }

    #[test]
    fn relative_enclosures_resolve_honouring_xml_base() {
        // Atom: an entry-level xml:base wins over the feed URL.
        let atom = br#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom"><title>t</title>
<entry xml:base="https://cdn.example.org/media/"><id>a1</id><title>a</title>
<link rel="enclosure" type="image/png" href="a.png"/>
</entry></feed>"#;
        let feed = parse_feed(atom, Some("https://example.com/feed.atom")).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://cdn.example.org/media/a.png")
        );
        // RSS: a relative image enclosure resolves against the feed URL.
        let rss = br#"<?xml version="1.0"?>
<rss version="2.0"><channel><title>t</title>
<item><title>a</title><guid>g</guid>
<enclosure url="/pics/p.jpg" type="image/jpeg" length="1"/>
</item></channel></rss>"#;
        let feed = parse_feed(rss, Some("https://example.com/feed.xml")).unwrap();
        assert_eq!(
            feed.entries[0].lead_image.as_deref(),
            Some("https://example.com/pics/p.jpg")
        );
    }

    #[test]
    fn resolve_http_url_only_yields_absolute_http() {
        let base = Url::parse("https://example.com/a/b").unwrap();
        assert_eq!(
            resolve_http_url(" https://x.example/i.jpg ", None).as_deref(),
            Some("https://x.example/i.jpg")
        );
        assert_eq!(
            resolve_http_url("//cdn.example/i.jpg", Some(&base)).as_deref(),
            Some("https://cdn.example/i.jpg"),
            "protocol-relative takes the base's scheme"
        );
        assert_eq!(resolve_http_url("i.jpg", None), None);
        assert_eq!(resolve_http_url("", Some(&base)), None);
        assert_eq!(
            resolve_http_url("data:image/png;base64,AAAA", Some(&base)),
            None
        );
        assert_eq!(
            resolve_http_url("ftp://example.com/i.jpg", Some(&base)),
            None
        );
        assert_eq!(resolve_http_url("http://[bad", Some(&base)), None);
    }
}
