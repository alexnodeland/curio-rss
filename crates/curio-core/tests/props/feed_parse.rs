//! `feeds::parse_feed`: never panics on any bytes, and a generated RSS /
//! Atom / JSON Feed document comes back as exactly the items it was built
//! from — titles, links and the total dedupe chain intact.

use curio_core::feeds::parse_feed;
use std::fmt::Write as _;

use proptest::prelude::*;

use crate::generators::{escape, text};

/// One generated feed item.
#[derive(Debug, Clone)]
struct Item {
    title: String,
    guid: Option<String>,
    link: Option<String>,
    body: String,
}

fn item() -> impl Strategy<Value = Item> {
    (
        text(),
        prop::option::of("[a-zA-Z0-9:_.-]{1,24}"),
        prop::option::of("https://feed\\.example/[a-z0-9]{1,12}"),
        text(),
    )
        .prop_map(|(title, guid, link, body)| Item {
            title,
            guid,
            link,
            body,
        })
}

fn rss(items: &[Item]) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><rss version="2.0"><channel><title>p</title><link>https://feed.example/</link><description>d</description>"#,
    );
    for item in items {
        out.push_str("<item>");
        write!(out, "<title>{}</title>", escape(&item.title)).unwrap();
        if let Some(guid) = &item.guid {
            write!(out, "<guid isPermaLink=\"false\">{guid}</guid>").unwrap();
        }
        if let Some(link) = &item.link {
            write!(out, "<link>{link}</link>").unwrap();
        }
        write!(out, "<description>{}</description>", escape(&item.body)).unwrap();
        out.push_str("</item>");
    }
    out.push_str("</channel></rss>");
    out
}

fn atom(items: &[Item]) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?><feed xmlns="http://www.w3.org/2005/Atom"><title>p</title><id>urn:p</id><updated>2026-07-01T00:00:00Z</updated>"#,
    );
    for item in items {
        out.push_str("<entry>");
        write!(out, "<title type=\"text\">{}</title>", escape(&item.title)).unwrap();
        if let Some(guid) = &item.guid {
            write!(out, "<id>{guid}</id>").unwrap();
        }
        if let Some(link) = &item.link {
            write!(out, "<link href=\"{link}\"/>").unwrap();
        }
        out.push_str("<updated>2026-07-01T00:00:00Z</updated>");
        write!(
            out,
            "<content type=\"html\">{}</content>",
            escape(&item.body)
        )
        .unwrap();
        out.push_str("</entry>");
    }
    out.push_str("</feed>");
    out
}

fn json_feed(items: &[Item]) -> String {
    let items: Vec<serde_json::Value> = items
        .iter()
        .map(|item| {
            let mut value = serde_json::json!({
                "title": item.title,
                "content_html": item.body,
            });
            if let Some(guid) = &item.guid {
                value["id"] = guid.clone().into();
            }
            if let Some(link) = &item.link {
                value["url"] = link.clone().into();
            }
            value
        })
        .collect();
    serde_json::json!({
        "version": "https://jsonfeed.org/version/1.1",
        "title": "p",
        "items": items,
    })
    .to_string()
}

/// The parsed feed holds exactly `items`, in order.
fn assert_round_trip(document: &str, items: &[Item]) -> Result<(), TestCaseError> {
    let feed = parse_feed(document.as_bytes(), Some("https://feed.example/feed"))
        .map_err(|err| TestCaseError::fail(format!("{err}\n{document}")))?;
    prop_assert_eq!(feed.entries.len(), items.len());
    for (entry, item) in feed.entries.iter().zip(items) {
        let title = item.title.trim();
        if title.is_empty() {
            prop_assert_eq!(entry.title.as_str(), "(untitled)");
        } else {
            prop_assert_eq!(entry.title.trim(), title);
        }
        prop_assert_eq!(entry.link.as_deref(), item.link.as_deref());
        match (&item.guid, &item.link) {
            (Some(guid), _) => prop_assert_eq!(&entry.dedupe_key, &format!("guid:{guid}")),
            (None, Some(link)) => prop_assert_eq!(&entry.dedupe_key, &format!("link:{link}")),
            (None, None) => prop_assert!(entry.dedupe_key.starts_with("hash:")),
        }
    }
    Ok(())
}

proptest! {
    #[test]
    fn never_panics_on_arbitrary_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0..4096),
    ) {
        let _ = parse_feed(&bytes, Some("https://feed.example/feed"));
    }

    #[test]
    fn never_panics_on_markup_shaped_text(
        body in "(<\\?xml[^>]{0,30}\\?>)?(<(rss|feed|rdf:RDF|channel|item|entry|title|link|guid)[^>]{0,20}>|</[a-z]{1,8}>|<!\\[CDATA\\[|\\]\\]>|&[a-z#0-9]{0,8};?|[^<&]{0,30}|\\{\"[a-z_]{1,10}\":|\\[|\\]|\\}){0,40}",
    ) {
        let _ = parse_feed(body.as_bytes(), None);
    }

    #[test]
    fn generated_rss_round_trips(items in prop::collection::vec(item(), 0..12)) {
        assert_round_trip(&rss(&items), &items)?;
    }

    #[test]
    fn generated_atom_round_trips(items in prop::collection::vec(item(), 0..12)) {
        assert_round_trip(&atom(&items), &items)?;
    }

    #[test]
    fn generated_json_feed_round_trips(items in prop::collection::vec(item(), 0..12)) {
        assert_round_trip(&json_feed(&items), &items)?;
    }

    #[test]
    fn dedupe_keys_are_stable_across_reparses(items in prop::collection::vec(item(), 0..12)) {
        let document = rss(&items);
        let keys = |doc: &str| -> Vec<String> {
            parse_feed(doc.as_bytes(), None)
                .map(|feed| feed.entries.into_iter().map(|entry| entry.dedupe_key).collect())
                .unwrap_or_default()
        };
        prop_assert_eq!(keys(&document), keys(&document));
    }
}
