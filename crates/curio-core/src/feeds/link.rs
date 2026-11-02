//! The entry and channel links: which declared link counts
//! ([`best_link`]), and the screen that keeps only web addresses
//! ([`web_link`], #262).

use feed_rs::model::Link;

/// The entry/site link as declared — unscreened; [`web_link`] screens it:
/// prefer an explicit `alternate` (or rel-less) link over
/// `self`/`enclosure`/anything else.
pub(super) fn best_link(links: &[Link]) -> Option<&str> {
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
pub(super) fn web_link(href: &str) -> Option<String> {
    crate::fetch::screen_url(href)
        .is_ok()
        .then(|| href.to_owned())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use crate::feeds::parse_feed;

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
}
