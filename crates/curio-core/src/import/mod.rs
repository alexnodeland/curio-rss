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
//! Real exports are messy, so importing is lenient end to end: heads read
//! the file through [`read_import_file`] (size-capped, BOM / `encoding=` /
//! UTF-16 aware — see [`decode_text`]), [`parse`] skips a broken OPML
//! outline or CSV row instead of failing the file, and
//! [`crate::CoreHandle::import_items`] normalizes every URL with
//! [`normalize_url`] (trim, `feed://` → `https://`, http(s) only) and
//! dedupes on that normalized form. Every dropped row is counted and
//! reported as a [`Rejected`] entry with its [`RejectReason`].
//!
//! Parsing does zero I/O and no network work: [`parse`] turns text already
//! in hand into [`ImportedItem`]s. Fetching the imported articles' bodies
//! (readability) stays the caller's job, through the policed client.

mod decode;

use chrono::{DateTime, Utc};
use curio_types::Timestamp;

use crate::feeds::OpmlError;

pub use decode::{MAX_IMPORT_BYTES, decode_text, read_import_file};

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

/// Why an import row was dropped instead of imported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    /// The row itself could not be read: an OPML outline whose attributes
    /// do not tokenize, or the unreadable tail of a document that breaks
    /// off mid-way.
    Malformed,
    /// The URL is not a parseable absolute URL.
    InvalidUrl,
    /// The URL parses but is not `http(s)` (`webcal:`, `javascript:`,
    /// `file:`, …) — nothing Curio can fetch.
    UnsupportedScheme,
}

/// One import row that was dropped, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rejected {
    /// Whether the row would have been a feed or an article.
    pub kind: ImportKind,
    /// Why it was dropped.
    pub reason: RejectReason,
    /// The offending value (the raw URL) or a description of the damage.
    pub detail: String,
}

/// What [`parse`] read from an export: the importable rows plus the rows it
/// had to drop.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedImport {
    /// The rows to import, in document order.
    pub items: Vec<ImportedItem>,
    /// The rows that could not be read at all.
    pub rejected: Vec<Rejected>,
}

/// Normalizes an imported feed or article URL to the one form Curio stores
/// and dedupes on: surrounding whitespace is trimmed, the `feed:` pseudo-
/// scheme some readers export is unwrapped (`feed://host/x` →
/// `https://host/x`, `feed:https://host/x` → `https://host/x`), and the
/// result must parse as an absolute `http(s)` URL with a host. The return
/// value is the [`url::Url`] serialization (lower-cased scheme and host,
/// default port dropped, empty path → `/`), so near-identical spellings of
/// one URL compare equal.
///
/// # Errors
///
/// [`RejectReason::InvalidUrl`] for something that is not a URL, and
/// [`RejectReason::UnsupportedScheme`] for a non-http(s) one.
pub fn normalize_url(raw: &str) -> Result<String, RejectReason> {
    let trimmed = raw.trim();
    let unwrapped = match strip_prefix_ignore_case(trimmed, "feed:") {
        Some(rest) if rest.starts_with("//") => format!("https:{rest}"),
        Some(rest) => rest.to_owned(),
        None => trimmed.to_owned(),
    };
    let parsed = url::Url::parse(&unwrapped).map_err(|_| RejectReason::InvalidUrl)?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(RejectReason::UnsupportedScheme);
    }
    if parsed.host_str().is_none_or(str::is_empty) {
        return Err(RejectReason::InvalidUrl);
    }
    Ok(parsed.into())
}

/// `text` without `prefix`, matched ASCII-case-insensitively.
fn strip_prefix_ignore_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
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

/// Parses `content` (the export file, already decoded to text — see
/// [`read_import_file`]) as `source`, yielding the neutral import rows in
/// document order plus the rows it had to drop. URLs are passed through as
/// written; [`crate::CoreHandle::import_items`] normalizes them.
///
/// # Errors
///
/// [`ImportError`] if the file is not OPML at all, the CSV header cannot be
/// read, or the CSV lacks a URL column (the usual sign of a source/file
/// mismatch). A single bad outline or row is never an error.
pub fn parse(source: ImportSource, content: &str) -> Result<ParsedImport, ImportError> {
    match source {
        ImportSource::Opml => {
            let opml = crate::feeds::parse_opml(content)?;
            Ok(ParsedImport {
                items: opml
                    .feeds
                    .into_iter()
                    .map(|feed| ImportedItem {
                        url: feed.xml_url,
                        title: feed.title,
                        tags: feed.tags,
                        kind: ImportKind::Feed,
                        saved_at: None,
                    })
                    .collect(),
                rejected: opml
                    .skipped
                    .into_iter()
                    .map(|detail| Rejected {
                        kind: ImportKind::Feed,
                        reason: RejectReason::Malformed,
                        detail,
                    })
                    .collect(),
            })
        }
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
/// (trailing export blanks), ragged rows are read as far as they go, and a
/// missing URL *column* is a hard error.
fn parse_csv(content: &str, spec: &CsvSpec) -> Result<ParsedImport, ImportError> {
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

    let mut parsed = ParsedImport::default();
    for record in reader.records() {
        // A flexible reader over in-memory UTF-8 has no per-record failure
        // mode left (ragged rows are allowed, the text is already valid), so
        // a bad *row* surfaces later, as an unusable URL.
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
        parsed.items.push(ImportedItem {
            url: url.to_owned(),
            title,
            tags,
            kind: ImportKind::Article,
            saved_at,
        });
    }
    Ok(parsed)
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
        let items = parse(ImportSource::PocketCsv, csv).unwrap().items;
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, ImportKind::Article);
        assert_eq!(items[0].url, "https://example.com/rust");
        // The embedded comma in the quoted title survives the CSV parser.
        assert_eq!(items[0].title.as_deref(), Some("Rust, revisited"));
        assert_eq!(items[0].tags, vec!["rust", "systems"]);
        assert!(items[0].saved_at.is_some());
        assert_eq!(items[1].tags, [] as [std::string::String; 0]);
    }

    #[test]
    fn instapaper_folder_becomes_a_tag_but_pseudo_folders_do_not() {
        let csv = "URL,Title,Selection,Folder,Timestamp\n\
                   https://example.com/a,Kept,,Reading,1700000000\n\
                   https://example.com/b,Loose,,Unread,1700000001\n";
        let items = parse(ImportSource::InstapaperCsv, csv).unwrap().items;
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
        let items = parse(ImportSource::ReadwiseCsv, csv).unwrap().items;
        assert_eq!(items[0].tags, vec!["focus", "habits"]);
        assert_eq!(
            items[0].saved_at.unwrap().as_datetime().to_rfc3339(),
            "2024-01-02T03:04:05+00:00"
        );
    }

    #[test]
    fn blank_url_rows_are_skipped() {
        let csv = "url,title\nhttps://example.com/a,A\n,orphan\n   ,spaces\n";
        let items = parse(ImportSource::PocketCsv, csv).unwrap().items;
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
    fn normalize_url_unwraps_feed_trims_and_canonicalizes() {
        for (raw, want) in [
            (
                "https://example.com/feed.xml",
                "https://example.com/feed.xml",
            ),
            (
                "  https://example.com/feed.xml\n",
                "https://example.com/feed.xml",
            ),
            ("feed://example.com/rss", "https://example.com/rss"),
            ("FEED://example.com/rss", "https://example.com/rss"),
            ("feed:https://example.com/atom", "https://example.com/atom"),
            ("feed:http://example.com/atom", "http://example.com/atom"),
            ("HTTPS://Example.COM:443", "https://example.com/"),
            (
                "http://example.com:80/a?b=1#c",
                "http://example.com/a?b=1#c",
            ),
        ] {
            assert_eq!(normalize_url(raw).as_deref(), Ok(want), "{raw:?}");
        }
    }

    #[test]
    fn normalize_url_refuses_non_http_and_non_urls() {
        for raw in [
            "webcal://calendar.example/x.ics",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "mailto:a@example.com",
            "feed:webcal://calendar.example/x.ics",
        ] {
            assert_eq!(
                normalize_url(raw),
                Err(RejectReason::UnsupportedScheme),
                "{raw:?}"
            );
        }
        for raw in [
            "",
            "   ",
            "not a url",
            "/relative/path",
            "https://",
            "feed:",
        ] {
            assert_eq!(normalize_url(raw), Err(RejectReason::InvalidUrl), "{raw:?}");
        }
    }

    #[test]
    fn a_broken_opml_outline_is_a_rejected_row_not_an_error() {
        let opml = r#"<opml version="2.0"><body>
            <outline xmlUrl=https://unquoted.example/feed/>
            <outline xmlUrl="https://ok.example/feed"/>
            </body></opml>"#;
        let parsed = parse(ImportSource::Opml, opml).unwrap();
        assert_eq!(parsed.items.len(), 1);
        assert_eq!(parsed.rejected.len(), 1);
        assert_eq!(parsed.rejected[0].kind, ImportKind::Feed);
        assert_eq!(parsed.rejected[0].reason, RejectReason::Malformed);
    }

    #[test]
    fn opml_rows_are_feeds_not_articles() {
        let opml = r#"<?xml version="1.0"?><opml version="2.0"><body>
            <outline text="Tech"><outline xmlUrl="https://example.com/feed.xml" title="Feed"/></outline>
            </body></opml>"#;
        let items = parse(ImportSource::Opml, opml).unwrap().items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].kind, ImportKind::Feed);
        assert_eq!(items[0].url, "https://example.com/feed.xml");
        assert_eq!(items[0].tags, vec!["Tech"]);
    }
}
