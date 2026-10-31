//! OPML subscription-list exchange (quick-xml).
//!
//! Import walks `<outline>` elements: an outline with an `xmlUrl` is a
//! feed; an outline without one is a folder, and the enclosing folder path
//! collapses into ONE hierarchical tag on the feed (`Tech/Databases`), so
//! arbitrary nesting survives import as a single `/`-joined tag rather than
//! a flat bag of sibling names. Export mirrors this: a feed's first tag is
//! its folder **path** (`/`-split back into nested `<outline>` folders) and
//! its remaining tags ride the `category` attribute, so folders render as
//! folders in other readers too. Import → export → import is lossless
//! (modulo feed order, which OPML does not make meaningful).

use std::collections::BTreeMap;
use std::io::Cursor;

use quick_xml::Reader;
use quick_xml::Writer;
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};

/// One subscription in an OPML document.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpmlFeed {
    /// The feed URL (`xmlUrl`) — the one required attribute.
    pub xml_url: String,
    /// Human-readable name (`title`, falling back to `text`).
    pub title: Option<String>,
    /// The feed's website (`htmlUrl`).
    pub html_url: Option<String>,
    /// Tags: the enclosing folder path as one `/`-joined hierarchical tag
    /// (outermost first), plus any `category` attribute entries, deduplicated
    /// in first-seen order.
    pub tags: Vec<String>,
}

/// OPML read/write failures.
#[derive(Debug, thiserror::Error)]
pub enum OpmlError {
    /// The document is not well-formed XML.
    #[error("opml parse: {0}")]
    Xml(#[from] quick_xml::Error),
    /// An attribute failed to decode.
    #[error("opml attribute: {0}")]
    Attr(#[from] quick_xml::events::attributes::AttrError),
    /// The writer's sink failed (unreachable for in-memory export).
    #[error("opml write: {0}")]
    Io(#[from] std::io::Error),
    /// The document contained no `<opml>` element at all.
    #[error("not an opml document (no <opml> element)")]
    NotOpml,
}

/// A leniently parsed OPML document: the feeds it yields plus a reason for
/// every outline that had to be dropped.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OpmlImport {
    /// The feeds, in document order.
    pub feeds: Vec<OpmlFeed>,
    /// One human-readable reason per dropped outline (or per unreadable
    /// document tail) — counted and reported, never fatal.
    pub skipped: Vec<String>,
}

/// Parses an OPML document into its feeds, in document order.
///
/// Shorthand for [`parse_opml`] that drops the skip reasons.
///
/// # Errors
///
/// As [`parse_opml`].
pub fn import_opml(xml: &str) -> Result<Vec<OpmlFeed>, OpmlError> {
    parse_opml(xml).map(|import| import.feeds)
}

/// Parses an OPML document into its feeds, in document order, reporting
/// every outline it had to skip.
///
/// Tolerant by design, because real exports are messy: attribute names are
/// matched case-insensitively (`xmlUrl` vs `xmlurl`), attribute values are
/// unescaped leniently (HTML entities such as `&nbsp;` resolve, a bare `&`
/// or an unknown entity is kept as raw text), a bare `&` in element text is
/// allowed, mismatched or stray closing tags are tolerated, and folders
/// without feeds and non-outline elements are ignored. An outline whose
/// attributes cannot be read at all is skipped (and reported), not fatal;
/// XML that breaks off after the `<opml>` root keeps every feed read up to
/// that point and reports the rest as one skip.
///
/// # Errors
///
/// [`OpmlError`] for a document that never reaches an `<opml>` element —
/// not XML at all, or not OPML.
pub fn parse_opml(xml: &str) -> Result<OpmlImport, OpmlError> {
    let mut reader = Reader::from_str(xml);
    let config = reader.config_mut();
    config.trim_text(true);
    config.allow_dangling_amp = true;
    config.allow_unmatched_ends = true;
    config.check_end_names = false;

    let mut import = OpmlImport::default();
    let mut folders: Vec<String> = Vec::new();
    let mut saw_opml = false;

    loop {
        let event = match reader.read_event() {
            Ok(event) => event,
            Err(error) if saw_opml => {
                import.skipped.push(format!(
                    "unreadable XML from byte {}: {error}",
                    reader.error_position()
                ));
                break;
            }
            Err(error) => return Err(error.into()),
        };
        match event {
            Event::Start(el) => {
                let name = local_name(el.name().as_ref());
                if name.eq_ignore_ascii_case("opml") {
                    saw_opml = true;
                } else if name.eq_ignore_ascii_case("outline") {
                    // Every opened outline pushes exactly one level, so its
                    // `</outline>` pops the right one. A feed (or a skipped
                    // outline) names no folder: it pushes an empty level.
                    let level = match read_outline(&el, &folders) {
                        Ok(Outline::Folder(name)) => name,
                        Ok(Outline::Feed(feed)) => {
                            import.feeds.push(feed);
                            String::new()
                        }
                        Err(reason) => {
                            import.skipped.push(reason);
                            String::new()
                        }
                    };
                    folders.push(level);
                }
            }
            Event::Empty(el) => {
                let name = local_name(el.name().as_ref());
                if name.eq_ignore_ascii_case("outline") {
                    match read_outline(&el, &folders) {
                        Ok(Outline::Feed(feed)) => import.feeds.push(feed),
                        Ok(Outline::Folder(_)) => {}
                        Err(reason) => import.skipped.push(reason),
                    }
                }
            }
            Event::End(el) => {
                if local_name(el.name().as_ref()).eq_ignore_ascii_case("outline") {
                    folders.pop();
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !saw_opml {
        return Err(OpmlError::NotOpml);
    }
    Ok(import)
}

/// Renders feeds as a **nested** OPML 2.0 document: a feed's first tag is its
/// folder path (`/`-split into nested `<outline>` folders), its remaining tags
/// the `category` attribute. Feeds with no tags sit at the root.
///
/// # Errors
///
/// [`OpmlError::Xml`] on a writer failure (practically unreachable for
/// an in-memory writer).
pub fn export_opml(title: &str, feeds: &[OpmlFeed]) -> Result<String, OpmlError> {
    let mut root = ExportFolder::default();
    for feed in feeds {
        // The first tag is the feed's folder path (`/`-split into nesting);
        // the remaining tags become its `category` attribute. No tags → root.
        let segments: Vec<&str> = feed.tags.first().map_or_else(Vec::new, |tag| {
            tag.split('/')
                .map(str::trim)
                .filter(|segment| !segment.is_empty())
                .collect()
        });
        let category: Vec<&str> = feed.tags.iter().skip(1).map(String::as_str).collect();
        let mut node = &mut root;
        for segment in segments {
            node = node.subfolders.entry(segment.to_owned()).or_default();
        }
        node.feeds.push((feed, category));
    }

    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);
    writer.write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))?;
    writer.write_event(Event::Start(
        BytesStart::new("opml").with_attributes([("version", "2.0")]),
    ))?;
    writer.write_event(Event::Start(BytesStart::new("head")))?;
    writer.write_event(Event::Start(BytesStart::new("title")))?;
    writer.write_event(Event::Text(BytesText::new(title)))?;
    writer.write_event(Event::End(BytesEnd::new("title")))?;
    writer.write_event(Event::End(BytesEnd::new("head")))?;
    writer.write_event(Event::Start(BytesStart::new("body")))?;
    write_folder(&mut writer, &root)?;
    writer.write_event(Event::End(BytesEnd::new("body")))?;
    writer.write_event(Event::End(BytesEnd::new("opml")))?;
    let bytes = writer.into_inner().into_inner();
    let mut out = String::from_utf8_lossy(&bytes).into_owned();
    out.push('\n');
    Ok(out)
}

/// A node of the export folder tree, keyed on `/`-path-tag segments.
#[derive(Default)]
struct ExportFolder<'a> {
    /// Subfolders by segment name — a `BTreeMap` for deterministic output.
    subfolders: BTreeMap<String, ExportFolder<'a>>,
    /// Feeds directly in this folder, each with its remaining (non-folder)
    /// tags to write as the `category` attribute.
    feeds: Vec<(&'a OpmlFeed, Vec<&'a str>)>,
}

/// Emits a folder's feeds as `<outline type="rss">` leaves, then recurses
/// into each subfolder wrapped in a titled `<outline>`.
fn write_folder<W: std::io::Write>(
    writer: &mut Writer<W>,
    folder: &ExportFolder<'_>,
) -> Result<(), OpmlError> {
    for (feed, category) in &folder.feeds {
        let mut outline = BytesStart::new("outline");
        outline.push_attribute(("type", "rss"));
        let text = feed.title.as_deref().unwrap_or(feed.xml_url.as_str());
        outline.push_attribute(("text", text));
        if let Some(feed_title) = &feed.title {
            outline.push_attribute(("title", feed_title.as_str()));
        }
        outline.push_attribute(("xmlUrl", feed.xml_url.as_str()));
        if let Some(html_url) = &feed.html_url {
            outline.push_attribute(("htmlUrl", html_url.as_str()));
        }
        if !category.is_empty() {
            outline.push_attribute(("category", category.join(",").as_str()));
        }
        writer.write_event(Event::Empty(outline))?;
    }
    for (name, subfolder) in &folder.subfolders {
        let mut folder_outline = BytesStart::new("outline");
        folder_outline.push_attribute(("text", name.as_str()));
        writer.write_event(Event::Start(folder_outline))?;
        write_folder(writer, subfolder)?;
        writer.write_event(Event::End(BytesEnd::new("outline")))?;
    }
    Ok(())
}

/// How much of a skipped outline's raw tag a skip reason quotes.
const SKIP_SNIPPET_CHARS: usize = 120;

enum Outline {
    Feed(OpmlFeed),
    Folder(String),
}

/// Reads one `<outline>`. An `Err` carries the human-readable reason the
/// outline was skipped: its attribute list could not be tokenized at all.
fn read_outline(el: &BytesStart<'_>, folders: &[String]) -> Result<Outline, String> {
    let mut xml_url = None;
    let mut title = None;
    let mut text = None;
    let mut html_url = None;
    let mut category = None;
    // Duplicate attributes are tolerated (the last one wins); only an
    // attribute list that cannot be tokenized skips the outline.
    for attr in el.attributes().with_checks(false) {
        let attr = attr.map_err(|error| {
            let tag = String::from_utf8_lossy(el.as_ref());
            let snippet: String = tag.chars().take(SKIP_SNIPPET_CHARS).collect();
            format!("malformed outline <{snippet}>: {error}")
        })?;
        let key = local_name(attr.key.as_ref());
        let value = super::entities::unescape_lenient(&String::from_utf8_lossy(&attr.value));
        if key.eq_ignore_ascii_case("xmlUrl") {
            xml_url = Some(value);
        } else if key.eq_ignore_ascii_case("title") {
            title = Some(value);
        } else if key.eq_ignore_ascii_case("text") {
            text = Some(value);
        } else if key.eq_ignore_ascii_case("htmlUrl") {
            html_url = Some(value);
        } else if key.eq_ignore_ascii_case("category") {
            category = Some(value);
        }
    }
    let display = title.or(text).filter(|t| !t.trim().is_empty());
    match xml_url.filter(|u| !u.trim().is_empty()) {
        Some(xml_url) => {
            // OPML requires `text`; exporters (ours included) fall back
            // to the feed URL for unnamed feeds. That placeholder is not
            // a title — drop it so round-trips stay lossless.
            let display = display.filter(|t| t != &xml_url);
            let mut tags: Vec<String> = Vec::new();
            let mut push = |tag: &str| {
                let tag = tag.trim();
                if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
                    tags.push(tag.to_owned());
                }
            };
            // The enclosing folder path collapses into ONE hierarchical tag
            // ("Tech/Databases"): join the non-empty levels outermost-first so
            // nesting survives as a single `/`-joined tag. Empty levels (a
            // feed outline that also nests children pushes "") are skipped.
            let folder_path = folders
                .iter()
                .map(|segment| segment.trim())
                .filter(|segment| !segment.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            push(&folder_path);
            for tag in category.as_deref().unwrap_or_default().split(',') {
                push(tag);
            }
            Ok(Outline::Feed(OpmlFeed {
                xml_url,
                title: display,
                html_url,
                tags,
            }))
        }
        None => Ok(Outline::Folder(display.unwrap_or_default())),
    }
}

/// Strips any namespace prefix from an element/attribute name.
fn local_name(raw: &[u8]) -> String {
    let raw = raw.rsplit(|&b| b == b':').next().unwrap_or(raw);
    String::from_utf8_lossy(raw).into_owned()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// The crate dir is read at run time (cargo sets it for test runs): a
    /// compile-time path goes stale when a shared target dir reuses this
    /// binary from another worktree (issue #183).
    fn fixture(name: &str) -> String {
        let crate_dir = std::env::var_os("CARGO_MANIFEST_DIR")
            .unwrap_or_else(|| panic!("CARGO_MANIFEST_DIR unset: run the tests through cargo"));
        std::fs::read_to_string(
            std::path::Path::new(&crate_dir)
                .join("../../fixtures/opml")
                .join(name),
        )
        .unwrap()
    }

    fn roundtrip(feeds: &[OpmlFeed]) -> Vec<OpmlFeed> {
        import_opml(&export_opml("roundtrip", feeds).unwrap()).unwrap()
    }

    /// Sorted by feed URL — nested export reorders feeds by folder, and OPML
    /// makes feed order no more meaningful than folder order, so round-trip
    /// equality is compared as a set.
    fn sorted(mut feeds: Vec<OpmlFeed>) -> Vec<OpmlFeed> {
        feeds.sort_by(|a, b| a.xml_url.cmp(&b.xml_url));
        feeds
    }

    #[test]
    fn imports_a_flat_document() {
        let feeds = import_opml(&fixture("simple.opml")).unwrap();
        assert_eq!(feeds.len(), 3);
        assert_eq!(feeds[0].xml_url, "https://example.com/feed.xml");
        assert_eq!(feeds[0].title.as_deref(), Some("Example Blog"));
        assert_eq!(feeds[0].html_url.as_deref(), Some("https://example.com/"));
        assert_eq!(feeds[1].title.as_deref(), Some("Atom Example"));
        assert_eq!(
            feeds[2].title.as_deref(),
            Some("Ampersands & Quotes \"here\""),
            "entities must unescape"
        );
        assert_eq!(feeds[2].xml_url, "https://example.com/feed.xml?a=1&b=2");
    }

    #[test]
    fn imports_nested_folders_as_path_tags() {
        let feeds = import_opml(&fixture("nested.opml")).unwrap();
        assert_eq!(feeds.len(), 4);
        assert_eq!(feeds[0].title.as_deref(), Some("Rust Blog"));
        assert_eq!(
            feeds[0].tags,
            vec!["Tech"],
            "single-level folder → bare tag"
        );
        assert_eq!(feeds[1].title.as_deref(), Some("SQLite News"));
        assert_eq!(
            feeds[1].tags,
            vec!["Tech/Databases"],
            "nesting collapses to one `/`-joined path tag, not a flat bag"
        );
        assert_eq!(feeds[2].tags, vec!["Cooking"]);
        assert_eq!(feeds[3].tags, [] as [std::string::String; 0]);
    }

    #[test]
    fn deep_nesting_joins_every_level_into_one_path_tag() {
        // Four levels deep — the whole path rides as a single tag, and a
        // `category` on the leaf is kept as a separate flat tag.
        let xml = r#"<?xml version="1.0"?>
<opml version="2.0"><body>
  <outline text="A">
    <outline text="B">
      <outline text="C">
        <outline text="D">
          <outline type="rss" text="Deep" xmlUrl="https://deep.example/feed" category="starred"/>
        </outline>
      </outline>
    </outline>
  </outline>
</body></opml>"#;
        let feeds = import_opml(xml).unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].tags, vec!["A/B/C/D", "starred"]);
    }

    #[test]
    fn imports_sparse_and_odd_case_attributes() {
        let feeds = import_opml(&fixture("sparse.opml")).unwrap();
        assert_eq!(feeds.len(), 2);
        assert_eq!(feeds[0].xml_url, "https://lowercase-attr.example/feed.xml");
        assert_eq!(feeds[0].title, None);
        assert_eq!(feeds[1].title.as_deref(), Some("Title Only"));
    }

    #[test]
    fn html_entities_bare_ampersands_and_stray_end_tags_are_tolerated() {
        // Every line here aborted the whole import under a strict parser.
        let xml = "<opml version=\"1.0\"><head><title>Tom & Jerry&nbsp;</title></head><body>\
            <outline text=\"Caf&eacute; &amp; Co\"><outline text=\"A & B\" xmlUrl=\"https://a.example/?x=1&y=2\"/></outline>\
            </Outline></body></opml>";
        let import = parse_opml(xml).unwrap();
        assert!(import.skipped.is_empty(), "{:?}", import.skipped);
        assert_eq!(import.feeds.len(), 1);
        assert_eq!(import.feeds[0].title.as_deref(), Some("A & B"));
        assert_eq!(import.feeds[0].xml_url, "https://a.example/?x=1&y=2");
        assert_eq!(import.feeds[0].tags, vec!["Café & Co"]);
    }

    #[test]
    fn an_unreadable_outline_is_skipped_with_a_reason() {
        let xml = r#"<opml><body>
            <outline text="Folder"><outline xmlUrl=bare/><outline xmlUrl="https://b.example/"/></outline>
            <outline xmlUrl="https://c.example/"/>
        </body></opml>"#;
        let import = parse_opml(xml).unwrap();
        assert_eq!(import.skipped.len(), 1);
        assert!(
            import.skipped[0].starts_with("malformed outline <outline xmlUrl=bare"),
            "{}",
            import.skipped[0]
        );
        // The skip did not unbalance the folder stack.
        assert_eq!(import.feeds[0].tags, vec!["Folder"]);
        assert_eq!(import.feeds[1].tags, [] as [String; 0]);
    }

    #[test]
    fn rejects_non_opml() {
        assert!(matches!(
            import_opml("<rss version=\"2.0\"/>"),
            Err(OpmlError::NotOpml)
        ));
        assert!(import_opml("<opml").is_err());
    }

    #[test]
    fn every_fixture_round_trips_losslessly() {
        for name in ["simple.opml", "nested.opml", "sparse.opml"] {
            let imported = import_opml(&fixture(name)).unwrap();
            assert_eq!(
                sorted(roundtrip(&imported)),
                sorted(imported),
                "{name} must survive import → export → import"
            );
        }
    }

    #[test]
    fn export_rebuilds_nested_outline_folders_from_path_tags() {
        // First tag = folder path (nested outlines); the rest = category.
        let feeds = vec![
            OpmlFeed {
                xml_url: "https://sqlite.example/news.xml".to_owned(),
                title: Some("SQLite News".to_owned()),
                html_url: None,
                tags: vec!["Tech/Databases".to_owned(), "fav".to_owned()],
            },
            OpmlFeed {
                xml_url: "https://top.example/feed.xml".to_owned(),
                title: None,
                html_url: None,
                tags: vec![],
            },
        ];
        let xml = export_opml("nested", &feeds).unwrap();
        // The path became real nested folders, not a flat `category`.
        assert!(xml.contains(r#"<outline text="Tech">"#), "{xml}");
        assert!(xml.contains(r#"<outline text="Databases">"#), "{xml}");
        assert!(
            xml.contains(r#"category="fav""#),
            "remaining tags stay category"
        );
        assert!(
            !xml.contains("Tech/Databases"),
            "path is folders, not a tag string"
        );
        // …and it still round-trips to the same tags.
        assert_eq!(sorted(roundtrip(&feeds)), sorted(feeds));
    }

    #[test]
    fn export_escapes_special_characters() {
        let feeds = vec![OpmlFeed {
            xml_url: "https://example.com/?a=1&b=<2>".to_owned(),
            title: Some("Quotes \" & <angles>".to_owned()),
            html_url: None,
            tags: vec!["tag one".to_owned(), "tag&two".to_owned()],
        }];
        let xml = export_opml("escapes", &feeds).unwrap();
        assert!(!xml.contains("b=<2>"), "raw angle bracket leaked: {xml}");
        assert_eq!(sorted(roundtrip(&feeds)), sorted(feeds));
    }
}
