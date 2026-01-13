//! OPML import/export with support for extended Curio attributes.

use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use uuid::Uuid;

use crate::core::models::ViewMode;
use crate::error::InfraError;

/// Parsed OPML document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpmlDocument {
    pub title: String,
    pub profile: Option<String>,
    pub outlines: Vec<OpmlOutline>,
}

/// OPML outline element (folder or feed)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpmlOutline {
    pub text: String,
    pub outline_type: Option<String>,
    pub xml_url: Option<String>,
    pub html_url: Option<String>,

    // Extended Curio attributes
    pub view_mode: Option<ViewMode>,
    pub tags: Option<Vec<String>>,
    pub icon: Option<String>,
    pub color: Option<String>,
    pub refresh_interval: Option<i32>,
    pub notify: Option<bool>,
    pub channel_id: Option<String>,
    pub subreddit: Option<String>,

    // Child outlines
    pub children: Vec<OpmlOutline>,
}

impl OpmlOutline {
    /// Check if this outline represents a feed
    pub fn is_feed(&self) -> bool {
        self.xml_url.is_some()
            || self
                .outline_type
                .as_ref()
                .map(|t| t == "rss" || t == "atom")
                .unwrap_or(false)
    }

    /// Check if this outline represents a folder
    pub fn is_folder(&self) -> bool {
        !self.is_feed()
    }

    /// Count total feeds in this outline and children
    pub fn count_feeds(&self) -> usize {
        let self_count = if self.is_feed() { 1 } else { 0 };
        let children_count: usize = self.children.iter().map(|c| c.count_feeds()).sum();
        self_count + children_count
    }
}

/// Parse an OPML document from XML string
pub fn parse_opml(xml: &str) -> Result<OpmlDocument, InfraError> {
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);

    let mut title = String::new();
    let mut profile = None;
    let mut outlines = Vec::new();
    let mut stack: Vec<OpmlOutline> = Vec::new();
    let mut in_head = false;
    let mut in_body = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => match e.name().as_ref() {
                b"head" => in_head = true,
                b"body" => in_body = true,
                b"title" if in_head => {
                    if let Ok(Event::Text(t)) = reader.read_event() {
                        title = t.unescape().unwrap_or_default().to_string();
                    }
                }
                b"outline" if in_body => {
                    let outline = parse_outline_attributes(&e)?;
                    stack.push(outline);
                }
                _ => {}
            },
            Ok(Event::Empty(e)) if e.name().as_ref() == b"outline" && in_body => {
                let outline = parse_outline_attributes(&e)?;
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(outline);
                } else {
                    outlines.push(outline);
                }
            }
            Ok(Event::End(e)) => match e.name().as_ref() {
                b"head" => in_head = false,
                b"body" => in_body = false,
                b"outline" if in_body => {
                    if let Some(outline) = stack.pop() {
                        if let Some(parent) = stack.last_mut() {
                            parent.children.push(outline);
                        } else {
                            outlines.push(outline);
                        }
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(InfraError::FeedParse(format!("OPML parse error: {}", e))),
            _ => {}
        }
    }

    Ok(OpmlDocument {
        title,
        profile,
        outlines,
    })
}

/// Parse attributes from an outline element
fn parse_outline_attributes(element: &BytesStart) -> Result<OpmlOutline, InfraError> {
    let mut outline = OpmlOutline {
        text: String::new(),
        outline_type: None,
        xml_url: None,
        html_url: None,
        view_mode: None,
        tags: None,
        icon: None,
        color: None,
        refresh_interval: None,
        notify: None,
        channel_id: None,
        subreddit: None,
        children: Vec::new(),
    };

    for attr in element.attributes().filter_map(|a| a.ok()) {
        let key = std::str::from_utf8(attr.key.as_ref()).unwrap_or_default();
        let value = attr.unescape_value().unwrap_or_default().to_string();

        match key {
            "text" => outline.text = value,
            "type" => outline.outline_type = Some(value),
            "xmlUrl" => outline.xml_url = Some(value),
            "htmlUrl" => outline.html_url = Some(value),
            "curio:viewMode" => outline.view_mode = ViewMode::from_str_loose(&value),
            "curio:tags" => {
                outline.tags = Some(value.split(',').map(|s| s.trim().to_string()).collect())
            }
            "curio:icon" => outline.icon = Some(value),
            "curio:color" => outline.color = Some(value),
            "curio:refreshInterval" => outline.refresh_interval = value.parse().ok(),
            "curio:notify" => outline.notify = Some(value == "true"),
            "curio:channelId" => outline.channel_id = Some(value),
            "curio:subreddit" => outline.subreddit = Some(value),
            _ => {}
        }
    }

    Ok(outline)
}

/// Export an OPML document to XML string
pub fn export_opml(doc: &OpmlDocument, extended: bool) -> Result<String, InfraError> {
    let mut writer = Writer::new_with_indent(Cursor::new(Vec::new()), b' ', 2);

    // XML declaration
    writer
        .write_event(Event::Decl(BytesDecl::new("1.0", Some("UTF-8"), None)))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

    // OPML root
    let mut opml_start = BytesStart::new("opml");
    opml_start.push_attribute(("version", "2.0"));
    if extended {
        opml_start.push_attribute(("xmlns:curio", "http://curio-reader.com/opml"));
    }
    writer
        .write_event(Event::Start(opml_start))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

    // Head section
    writer
        .write_event(Event::Start(BytesStart::new("head")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;
    writer
        .write_event(Event::Start(BytesStart::new("title")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;
    writer
        .write_event(Event::Text(BytesText::new(&doc.title)))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;
    writer
        .write_event(Event::End(BytesEnd::new("title")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;
    writer
        .write_event(Event::End(BytesEnd::new("head")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

    // Body section
    writer
        .write_event(Event::Start(BytesStart::new("body")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

    for outline in &doc.outlines {
        write_outline(&mut writer, outline, extended)?;
    }

    writer
        .write_event(Event::End(BytesEnd::new("body")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

    writer
        .write_event(Event::End(BytesEnd::new("opml")))
        .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

    let result = writer.into_inner().into_inner();
    String::from_utf8(result).map_err(|e| InfraError::FeedParse(format!("UTF-8 error: {}", e)))
}

/// Write a single outline element (recursive)
fn write_outline<W: std::io::Write>(
    writer: &mut Writer<W>,
    outline: &OpmlOutline,
    extended: bool,
) -> Result<(), InfraError> {
    let mut elem = BytesStart::new("outline");
    elem.push_attribute(("text", outline.text.as_str()));

    if let Some(ref t) = outline.outline_type {
        elem.push_attribute(("type", t.as_str()));
    }
    if let Some(ref url) = outline.xml_url {
        elem.push_attribute(("xmlUrl", url.as_str()));
    }
    if let Some(ref url) = outline.html_url {
        elem.push_attribute(("htmlUrl", url.as_str()));
    }

    // Extended attributes (only if extended mode)
    if extended {
        if let Some(ref mode) = outline.view_mode {
            elem.push_attribute(("curio:viewMode", format!("{:?}", mode).to_lowercase().as_str()));
        }
        if let Some(ref tags) = outline.tags {
            elem.push_attribute(("curio:tags", tags.join(",").as_str()));
        }
        if let Some(ref icon) = outline.icon {
            elem.push_attribute(("curio:icon", icon.as_str()));
        }
        if let Some(ref color) = outline.color {
            elem.push_attribute(("curio:color", color.as_str()));
        }
        if let Some(interval) = outline.refresh_interval {
            elem.push_attribute(("curio:refreshInterval", interval.to_string().as_str()));
        }
        if let Some(notify) = outline.notify {
            elem.push_attribute(("curio:notify", if notify { "true" } else { "false" }));
        }
        if let Some(ref channel_id) = outline.channel_id {
            elem.push_attribute(("curio:channelId", channel_id.as_str()));
        }
        if let Some(ref subreddit) = outline.subreddit {
            elem.push_attribute(("curio:subreddit", subreddit.as_str()));
        }
    }

    if outline.children.is_empty() {
        writer
            .write_event(Event::Empty(elem))
            .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;
    } else {
        writer
            .write_event(Event::Start(elem))
            .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;

        for child in &outline.children {
            write_outline(writer, child, extended)?;
        }

        writer
            .write_event(Event::End(BytesEnd::new("outline")))
            .map_err(|e| InfraError::FeedParse(format!("OPML write error: {}", e)))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_OPML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head>
    <title>My Feeds</title>
  </head>
  <body>
    <outline text="Tech">
      <outline text="Rust" xmlUrl="https://this-week-in-rust.org/rss.xml" type="rss"/>
      <outline text="Go" xmlUrl="https://blog.golang.org/feed.atom" type="atom"/>
    </outline>
    <outline text="News" xmlUrl="https://news.ycombinator.com/rss" type="rss"/>
  </body>
</opml>"#;

    const EXTENDED_OPML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0" xmlns:curio="http://curio-reader.com/opml">
  <head>
    <title>Extended Feeds</title>
  </head>
  <body>
    <outline text="YouTube" curio:icon="video" curio:viewMode="youtube">
      <outline text="Tech Channel"
               xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id=UC123"
               type="rss"
               curio:viewMode="youtube"
               curio:channelId="UC123"
               curio:tags="tech,tutorials"/>
    </outline>
    <outline text="Reddit" curio:viewMode="reddit">
      <outline text="r/rust"
               xmlUrl="https://www.reddit.com/r/rust/.rss"
               type="rss"
               curio:viewMode="reddit"
               curio:subreddit="rust"/>
    </outline>
  </body>
</opml>"#;

    #[test]
    fn test_parse_simple_opml() {
        let result = parse_opml(SAMPLE_OPML).unwrap();

        assert_eq!(result.title, "My Feeds");
        assert_eq!(result.outlines.len(), 2);

        // First outline is a folder with 2 feeds
        let tech = &result.outlines[0];
        assert_eq!(tech.text, "Tech");
        assert!(tech.is_folder());
        assert_eq!(tech.children.len(), 2);
        assert_eq!(tech.count_feeds(), 2);

        // Check nested feed
        let rust = &tech.children[0];
        assert_eq!(rust.text, "Rust");
        assert!(rust.is_feed());
        assert_eq!(
            rust.xml_url,
            Some("https://this-week-in-rust.org/rss.xml".to_string())
        );

        // Second outline is a direct feed
        let news = &result.outlines[1];
        assert_eq!(news.text, "News");
        assert!(news.is_feed());
    }

    #[test]
    fn test_parse_extended_opml() {
        let result = parse_opml(EXTENDED_OPML).unwrap();

        assert_eq!(result.title, "Extended Feeds");
        assert_eq!(result.outlines.len(), 2);

        // YouTube folder
        let youtube = &result.outlines[0];
        assert_eq!(youtube.text, "YouTube");
        assert_eq!(youtube.icon, Some("video".to_string()));
        assert_eq!(youtube.view_mode, Some(ViewMode::YouTube));

        // YouTube channel feed
        let channel = &youtube.children[0];
        assert_eq!(channel.channel_id, Some("UC123".to_string()));
        assert_eq!(channel.tags, Some(vec!["tech".to_string(), "tutorials".to_string()]));

        // Reddit folder
        let reddit = &result.outlines[1];
        assert_eq!(reddit.view_mode, Some(ViewMode::Reddit));

        // Reddit feed
        let rust_sub = &reddit.children[0];
        assert_eq!(rust_sub.subreddit, Some("rust".to_string()));
    }

    #[test]
    fn test_outline_is_feed() {
        let feed = OpmlOutline {
            text: "Feed".to_string(),
            xml_url: Some("https://example.com/feed.xml".to_string()),
            outline_type: Some("rss".to_string()),
            ..Default::default()
        };
        assert!(feed.is_feed());
        assert!(!feed.is_folder());
    }

    #[test]
    fn test_outline_is_folder() {
        let folder = OpmlOutline {
            text: "Folder".to_string(),
            ..Default::default()
        };
        assert!(folder.is_folder());
        assert!(!folder.is_feed());
    }

    #[test]
    fn test_count_feeds() {
        let mut folder = OpmlOutline {
            text: "Folder".to_string(),
            ..Default::default()
        };

        folder.children.push(OpmlOutline {
            text: "Feed 1".to_string(),
            xml_url: Some("https://example.com/1.xml".to_string()),
            ..Default::default()
        });

        folder.children.push(OpmlOutline {
            text: "Feed 2".to_string(),
            xml_url: Some("https://example.com/2.xml".to_string()),
            ..Default::default()
        });

        assert_eq!(folder.count_feeds(), 2);
    }

    #[test]
    fn test_export_simple_opml() {
        let doc = OpmlDocument {
            title: "Test".to_string(),
            profile: None,
            outlines: vec![OpmlOutline {
                text: "Feed".to_string(),
                xml_url: Some("https://example.com/feed.xml".to_string()),
                outline_type: Some("rss".to_string()),
                ..Default::default()
            }],
        };

        let xml = export_opml(&doc, false).unwrap();

        assert!(xml.contains("<title>Test</title>"));
        assert!(xml.contains("text=\"Feed\""));
        assert!(xml.contains("xmlUrl=\"https://example.com/feed.xml\""));
        assert!(!xml.contains("curio:"));
    }

    #[test]
    fn test_export_extended_opml() {
        let doc = OpmlDocument {
            title: "Extended Test".to_string(),
            profile: None,
            outlines: vec![OpmlOutline {
                text: "YouTube Feed".to_string(),
                xml_url: Some("https://youtube.com/feeds/videos.xml?channel_id=UC123".to_string()),
                outline_type: Some("rss".to_string()),
                view_mode: Some(ViewMode::YouTube),
                channel_id: Some("UC123".to_string()),
                tags: Some(vec!["tech".to_string()]),
                ..Default::default()
            }],
        };

        let xml = export_opml(&doc, true).unwrap();

        assert!(xml.contains("xmlns:curio"));
        assert!(xml.contains("curio:viewMode=\"youtube\""));
        assert!(xml.contains("curio:channelId=\"UC123\""));
        assert!(xml.contains("curio:tags=\"tech\""));
    }

    #[test]
    fn test_roundtrip_opml() {
        let original = parse_opml(EXTENDED_OPML).unwrap();
        let exported = export_opml(&original, true).unwrap();
        let reparsed = parse_opml(&exported).unwrap();

        assert_eq!(original.title, reparsed.title);
        assert_eq!(original.outlines.len(), reparsed.outlines.len());
    }

    #[test]
    fn test_parse_invalid_opml() {
        let result = parse_opml("not valid xml");
        assert!(result.is_err());
    }
}

impl Default for OpmlOutline {
    fn default() -> Self {
        Self {
            text: String::new(),
            outline_type: None,
            xml_url: None,
            html_url: None,
            view_mode: None,
            tags: None,
            icon: None,
            color: None,
            refresh_interval: None,
            notify: None,
            channel_id: None,
            subreddit: None,
            children: Vec::new(),
        }
    }
}
