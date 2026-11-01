//! `feeds::import_opml` / `export_opml`: import never panics on any text,
//! and for any normalized subscription list export → import is lossless
//! (modulo feed order, which OPML does not make meaningful).

use curio_core::feeds::{OpmlFeed, export_opml, import_opml};
use std::fmt::Write as _;

use proptest::prelude::*;

use crate::generators::escape;

/// A label as import normalizes it: trimmed, non-empty, no `,` (the
/// `category` separator) or `/` (the folder-path separator). XML specials
/// are in on purpose — the writer must escape them.
fn label() -> impl Strategy<Value = String> {
    "[a-zA-Z0-9 &<>\"'.日本🎌-]{1,16}"
        .prop_map(|raw| raw.trim().to_owned())
        .prop_filter("labels are non-empty once trimmed", |label| {
            !label.is_empty()
        })
}

/// Normalized tags: an optional folder path (`/`-joined labels) first,
/// then unique category labels.
fn tags() -> impl Strategy<Value = Vec<String>> {
    (
        prop::collection::vec(label(), 0..4),
        prop::collection::vec(label(), 0..4),
    )
        .prop_map(|(path, categories)| {
            let mut tags: Vec<String> = Vec::new();
            let first = path.join("/");
            for tag in std::iter::once(first).chain(categories) {
                if !tag.is_empty() && !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
            tags
        })
}

fn feeds() -> impl Strategy<Value = Vec<OpmlFeed>> {
    prop::collection::vec(
        (
            prop::option::of(label()),
            prop::option::of("https://site\\.example/[a-z0-9]{0,8}"),
            tags(),
        ),
        0..10,
    )
    .prop_map(|rows| {
        rows.into_iter()
            .enumerate()
            .map(|(index, (title, html_url, tags))| OpmlFeed {
                // Unique URLs: the comparison sorts by them.
                xml_url: format!("https://feeds.example/{index}?a=1&b=<2>"),
                title,
                html_url,
                tags,
            })
            .collect()
    })
}

fn by_url(mut feeds: Vec<OpmlFeed>) -> Vec<OpmlFeed> {
    feeds.sort_by(|a, b| a.xml_url.cmp(&b.xml_url));
    feeds
}

proptest! {
    #[test]
    fn export_then_import_is_lossless(feeds in feeds(), title in label()) {
        let document = export_opml(&title, &feeds).map_err(|err| TestCaseError::fail(err.to_string()))?;
        let back = import_opml(&document)
            .map_err(|err| TestCaseError::fail(format!("{err}\n{document}")))?;
        prop_assert_eq!(by_url(back), by_url(feeds));
    }

    #[test]
    fn import_never_panics_on_arbitrary_text(text in "\\PC{0,2000}") {
        let _ = import_opml(&text);
    }

    #[test]
    fn import_never_panics_on_outline_soup(
        outlines in prop::collection::vec(
            (
                prop::sample::select(&["outline", "OUTLINE", "x:outline", "body", "head"][..]),
                prop::collection::vec(
                    (
                        prop::sample::select(&["xmlUrl", "xmlurl", "text", "title", "htmlUrl", "category", "type"][..]),
                        "\\PC{0,24}",
                    ),
                    0..5,
                ),
                0usize..3,
            ),
            0..24,
        ),
    ) {
        // Random open / self-closing / close events, so nesting is ragged.
        let mut body = String::from("<?xml version=\"1.0\"?><opml version=\"2.0\"><body>");
        for (name, attrs, shape) in &outlines {
            let attrs = attrs
                .iter()
                .map(|(key, value)| format!(" {key}=\"{}\"", escape(value)))
                .collect::<Vec<_>>()
                .concat();
            match shape {
                0 => write!(body, "<{name}{attrs}>").unwrap(),
                1 => write!(body, "<{name}{attrs}/>").unwrap(),
                _ => write!(body, "</{name}>").unwrap(),
            }
        }
        body.push_str("</body></opml>");
        if let Ok(feeds) = import_opml(&body) {
            for feed in feeds {
                prop_assert!(!feed.xml_url.trim().is_empty(), "a feed without a URL was imported");
            }
        }
    }
}
