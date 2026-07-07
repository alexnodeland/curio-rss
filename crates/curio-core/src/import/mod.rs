//! Refugee importers — parse third-party "read it later" and subscription
//! exports into a neutral [`ImportedItem`] stream that
//! [`crate::CoreHandle::import_items`] applies to the library.
//!
//! Article items (Pocket / Instapaper / Readwise saves) become feedless
//! read-later articles; feed items (OPML) become subscriptions. Every CSV
//! parser matches column *names* case-insensitively against a candidate
//! list, so it tolerates the header casing and column-order drift that real
//! exports show. CSV parsing is delegated to the `csv` crate — titles and
//! highlight excerpts routinely embed commas, quotes and newlines that a
//! naive split would corrupt.
//!
//! Parsing does zero I/O and no network work: [`parse`] turns bytes already
//! in hand into [`ImportedItem`]s. Fetching the imported articles' bodies
//! (readability) stays the caller's job, through the policed client.

use chrono::{DateTime, Utc};
use curio_types::Timestamp;

use crate::feeds::OpmlError;

/// Whether an imported row is a subscription or a saved article.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportKind {
    /// A feed subscription (an OPML `xmlUrl`).
    Feed,
    /// A saved article — becomes a feedless, read-later library item.
    Article,
}

/// One neutral, source-agnostic import row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportedItem {
    /// The subscription URL (feeds) or the saved article's URL (articles).
    pub url: String,
    /// Display title, if the export carried one.
    pub title: Option<String>,
    /// Folder path / tags, deduplicated in first-seen order.
    pub tags: Vec<String>,
    /// Feed vs. article.
    pub kind: ImportKind,
    /// When the user saved it, if the export carried a timestamp — kept as
    /// the article's `source_updated_at` so imports sort by save time.
    pub saved_at: Option<Timestamp>,
}

/// The recognized export formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSource {
    /// OPML 2.0 subscription list (Feedly, Reeder, Readwise feeds, …).
    Opml,
    /// Pocket CSV export (`title,url,time_added,tags,status`).
    PocketCsv,
    /// Instapaper CSV export (`URL,Title,Selection,Folder,Timestamp`).
    InstapaperCsv,
    /// Readwise Reader CSV export (`Title,URL,…,Document tags,Saved date`).
    ReadwiseCsv,
}

/// Import-parse failures.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The OPML document was malformed.
    #[error(transparent)]
    Opml(#[from] OpmlError),
    /// The CSV could not be read.
    #[error("csv parse: {0}")]
    Csv(#[from] csv::Error),
    /// The CSV had no column matching any known alias for the URL field —
    /// almost always the wrong source was chosen for the file.
    #[error("{format} export is missing a recognizable URL column")]
    MissingUrlColumn {
        /// The human name of the source whose parser ran.
        format: &'static str,
    },
}

/// Parses `content` (the raw export file, already read into memory) as
/// `source`, yielding the neutral import rows in document order.
///
/// # Errors
///
/// [`ImportError`] if the OPML is malformed, the CSV is unreadable, or the
/// CSV lacks a URL column (the usual sign of a source/file mismatch).
pub fn parse(source: ImportSource, content: &str) -> Result<Vec<ImportedItem>, ImportError> {
    match source {
        ImportSource::Opml => Ok(crate::feeds::import_opml(content)?
            .into_iter()
            .map(|feed| ImportedItem {
                url: feed.xml_url,
                title: feed.title,
                tags: feed.tags,
                kind: ImportKind::Feed,
                saved_at: None,
            })
            .collect()),
        ImportSource::PocketCsv => parse_csv(content, &POCKET),
        ImportSource::InstapaperCsv => parse_csv(content, &INSTAPAPER),
        ImportSource::ReadwiseCsv => parse_csv(content, &READWISE),
    }
}

/// The column-name aliases one CSV format matches against. All matching is
/// case-insensitive and trims surrounding whitespace, so `Time Added`,
/// `time_added` and `TIME ADDED` all hit.
struct CsvSpec {
    /// Human name of the source, for error messages.
    source: &'static str,
    /// Candidate header names for the article URL (required).
    url: &'static [&'static str],
    /// Candidate header names for the title (optional).
    title: &'static [&'static str],
    /// Candidate header names for the tags/folder field (optional).
    tags: &'static [&'static str],
    /// Candidate header names for the save timestamp (optional).
    time: &'static [&'static str],
    /// Tag values that are pseudo-folders, not real tags — dropped
    /// case-insensitively (Instapaper's `Unread`/`Archive`/`Starred`).
    tag_denylist: &'static [&'static str],
}

const POCKET: CsvSpec = CsvSpec {
    source: "Pocket",
    url: &["url"],
    title: &["title"],
    tags: &["tags"],
    time: &["time_added", "time added"],
    tag_denylist: &[],
};

const INSTAPAPER: CsvSpec = CsvSpec {
    source: "Instapaper",
    // Instapaper's `Folder` is the closest thing it has to a tag; custom
    // folders survive as tags, its three reserved pseudo-folders don't.
    url: &["url"],
    title: &["title"],
    tags: &["folder", "tags"],
    time: &["timestamp", "time"],
    tag_denylist: &["unread", "archive", "starred"],
};

const READWISE: CsvSpec = CsvSpec {
    source: "Readwise",
    url: &["url"],
    title: &["title"],
    tags: &["document tags", "tags"],
    time: &["saved date", "saved_date", "saved at"],
    tag_denylist: &[],
};

/// Parses a CSV export against `spec`. Rows with a blank URL are skipped
/// (trailing export blanks); a missing URL *column* is a hard error.
fn parse_csv(content: &str, spec: &CsvSpec) -> Result<Vec<ImportedItem>, ImportError> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(content.as_bytes());
    let headers = reader.headers()?.clone();
    let find = |candidates: &[&str]| -> Option<usize> {
        headers.iter().position(|header| {
            let header = header.trim().to_ascii_lowercase();
            candidates.iter().any(|candidate| *candidate == header)
        })
    };

    let url_col = find(spec.url).ok_or(ImportError::MissingUrlColumn {
        format: spec.source,
    })?;
    let title_col = find(spec.title);
    let tags_col = find(spec.tags);
    let time_col = find(spec.time);

    let mut items = Vec::new();
    for record in reader.records() {
        let record = record?;
        let url = record.get(url_col).map(str::trim).unwrap_or_default();
        if url.is_empty() {
            continue;
        }
        let title = title_col
            .and_then(|i| record.get(i))
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned);
        let tags = tags_col
            .and_then(|i| record.get(i))
            .map(|raw| split_tags(raw, spec.tag_denylist))
            .unwrap_or_default();
        let saved_at = time_col
            .and_then(|i| record.get(i))
            .and_then(parse_timestamp);
        items.push(ImportedItem {
            url: url.to_owned(),
            title,
            tags,
            kind: ImportKind::Article,
            saved_at,
        });
    }
    Ok(items)
}

/// Splits a tag/folder cell on `,` and `|`, trims each, drops empties and
/// denylisted pseudo-folders (case-insensitive), and deduplicates in
/// first-seen order.
fn split_tags(raw: &str, denylist: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in raw.split([',', '|']) {
        let tag = tag.trim();
        if tag.is_empty() {
            continue;
        }
        let lower = tag.to_ascii_lowercase();
        if denylist.contains(&lower.as_str()) {
            continue;
        }
        if !out.iter().any(|seen| seen.eq_ignore_ascii_case(tag)) {
            out.push(tag.to_owned());
        }
    }
    out
}

/// Parses a save timestamp from either Unix seconds (Pocket/Instapaper) or
/// an RFC 3339 datetime (Readwise). Returns `None` for anything else —
/// the timestamp is fidelity, never correctness, so an unparseable one is
/// silently dropped rather than failing the whole import.
fn parse_timestamp(raw: &str) -> Option<Timestamp> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    if let Ok(seconds) = raw.parse::<i64>() {
        return Timestamp::from_unix_millis(seconds.checked_mul(1000)?);
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|dt| Timestamp::new(dt.with_timezone(&Utc)))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn pocket_csv_becomes_read_later_articles_with_tags() {
        let csv = "title,url,time_added,tags,status\n\
                   \"Rust, revisited\",https://example.com/rust,1700000000,\"rust|systems\",unread\n\
                   Empty Tags,https://example.com/empty,1700000001,,archive\n";
        let items = parse(ImportSource::PocketCsv, csv).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, ImportKind::Article);
        assert_eq!(items[0].url, "https://example.com/rust");
        // The embedded comma in the quoted title survives the CSV parser.
        assert_eq!(items[0].title.as_deref(), Some("Rust, revisited"));
        assert_eq!(items[0].tags, vec!["rust", "systems"]);
        assert!(items[0].saved_at.is_some());
        assert!(items[1].tags.is_empty());
    }

    #[test]
    fn instapaper_folder_becomes_a_tag_but_pseudo_folders_do_not() {
        let csv = "URL,Title,Selection,Folder,Timestamp\n\
                   https://example.com/a,Kept,,Reading,1700000000\n\
                   https://example.com/b,Loose,,Unread,1700000001\n";
        let items = parse(ImportSource::InstapaperCsv, csv).unwrap();
        assert_eq!(items[0].tags, vec!["Reading"], "a real folder is a tag");
        assert!(
            items[1].tags.is_empty(),
            "the Unread pseudo-folder is not a tag"
        );
    }

    #[test]
    fn readwise_matches_headers_case_insensitively_and_parses_rfc3339() {
        let csv = "Title,URL,Document tags,Saved Date\n\
                   Deep Work,https://example.com/dw,\"focus, habits\",2024-01-02T03:04:05Z\n";
        let items = parse(ImportSource::ReadwiseCsv, csv).unwrap();
        assert_eq!(items[0].tags, vec!["focus", "habits"]);
        assert_eq!(
            items[0].saved_at.unwrap().as_datetime().to_rfc3339(),
            "2024-01-02T03:04:05+00:00"
        );
    }

    #[test]
    fn blank_url_rows_are_skipped() {
        let csv = "url,title\nhttps://example.com/a,A\n,orphan\n   ,spaces\n";
        let items = parse(ImportSource::PocketCsv, csv).unwrap();
        assert_eq!(items.len(), 1);
    }

    #[test]
    fn a_wrong_source_for_the_file_is_a_clear_error() {
        // An OPML file fed to the Pocket parser has no `url` column.
        let not_pocket = "col_a,col_b\n1,2\n";
        let err = parse(ImportSource::PocketCsv, not_pocket).unwrap_err();
        assert!(matches!(
            err,
            ImportError::MissingUrlColumn { format: "Pocket" }
        ));
    }

    #[test]
    fn opml_rows_are_feeds_not_articles() {
        let opml = r#"<?xml version="1.0"?><opml version="2.0"><body>
            <outline text="Tech"><outline xmlUrl="https://example.com/feed.xml" title="Feed"/></outline>
            </body></opml>"#;
        let items = parse(ImportSource::Opml, opml).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ImportKind::Feed);
        assert_eq!(items[0].url, "https://example.com/feed.xml");
        assert_eq!(items[0].tags, vec!["Tech"]);
    }
}
