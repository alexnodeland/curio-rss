//! `import::parse`: no export file makes any importer panic, and a
//! generated Pocket CSV — titles full of commas, quotes and newlines —
//! comes back row for row.

use curio_core::{ImportKind, ImportSource, import};
use proptest::prelude::*;

const SOURCES: &[ImportSource] = &[
    ImportSource::Opml,
    ImportSource::PocketCsv,
    ImportSource::InstapaperCsv,
    ImportSource::ReadwiseCsv,
];

#[derive(Debug, Clone)]
struct Row {
    title: String,
    url: String,
    time_added: u32,
    tags: Vec<String>,
}

fn tag_list() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec("[a-zA-Z0-9_-]{1,10}", 0..4).prop_map(|raw| {
        let mut tags: Vec<String> = Vec::new();
        for tag in raw {
            if !tags.iter().any(|seen| seen.eq_ignore_ascii_case(&tag)) {
                tags.push(tag);
            }
        }
        tags
    })
}

fn rows() -> impl Strategy<Value = Vec<Row>> {
    prop::collection::vec(
        (
            "[a-zA-Z0-9 ,;:\"'\n日本🎌-]{0,24}",
            "https://saved\\.example/[a-z0-9]{1,10}",
            any::<u32>(),
            tag_list(),
        )
            .prop_map(|(title, url, time_added, tags)| Row {
                title,
                url,
                time_added,
                tags,
            }),
        0..16,
    )
}

fn pocket_csv(rows: &[Row]) -> String {
    let mut writer = csv::Writer::from_writer(Vec::new());
    writer
        .write_record(["title", "url", "time_added", "tags", "status"])
        .unwrap();
    for row in rows {
        writer
            .write_record([
                row.title.as_str(),
                row.url.as_str(),
                &row.time_added.to_string(),
                &row.tags.join("|"),
                "unread",
            ])
            .unwrap();
    }
    String::from_utf8(writer.into_inner().unwrap()).unwrap()
}

proptest! {
    #[test]
    fn no_importer_panics_on_arbitrary_text(
        text in "\\PC{0,1500}",
        source in prop::sample::select(SOURCES),
    ) {
        let _ = import::parse(source, &text);
    }

    #[test]
    fn no_csv_importer_panics_on_csv_shaped_text(
        header in "((url|URL|title|tags|folder|time_added|timestamp|saved date|document tags| Url )(,|\\|)?){0,6}",
        body in "([a-z0-9:/.\"|,\n -]{0,40}){0,12}",
        source in prop::sample::select(&SOURCES[1..]),
    ) {
        let _ = import::parse(source, &format!("{header}\n{body}"));
    }

    #[test]
    fn generated_pocket_exports_round_trip(rows in rows()) {
        let parsed = import::parse(ImportSource::PocketCsv, &pocket_csv(&rows))
            .map_err(|err| TestCaseError::fail(err.to_string()))?;
        prop_assert!(parsed.rejected.is_empty(), "rejected {:?}", parsed.rejected);
        let items = parsed.items;
        prop_assert_eq!(items.len(), rows.len());
        for (item, row) in items.iter().zip(&rows) {
            prop_assert_eq!(item.kind, ImportKind::Article);
            prop_assert_eq!(&item.url, &row.url);
            let title = Some(row.title.trim()).filter(|title| !title.is_empty());
            prop_assert_eq!(item.title.as_deref(), title);
            prop_assert_eq!(&item.tags, &row.tags);
            prop_assert!(item.saved_at.is_some(), "unix seconds {} dropped", row.time_added);
        }
    }
}
