//! OPML + refugee (Pocket / Instapaper / Readwise) import, and OPML
//! export. Rust does the file IO; every path arrives as a
//! [`crate::ipc_policy`] token from a Rust-side dialog pick — never a raw
//! string. Import files are read through the core's size-capped,
//! encoding-detecting reader (Latin-1 OPML, UTF-16 CSV); the export lands
//! atomically, so a failed write never truncates the user's previous file.

use std::path::Path;
use std::sync::Arc;

use curio_core::export::write_file_atomic;
use curio_core::import::read_import_file;
use curio_core::{CoreHandle, ImportSource};
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::dto::{ImportOutcomeDto, ImportSourceDto, OpmlImportOutcomeDto};
use crate::error::{CommandError, ErrorCode};
use crate::events::{ArticlesChanged, FeedsChanged, emit_or_log};
use crate::ipc_policy::{PathIntent, PathRegistry};

/// Imports subscriptions from a dialog-picked OPML file. Already
/// subscribed URLs are skipped; OPML folders land as feed tags.
#[tauri::command]
#[specta::specta]
pub async fn import_opml(
    app: AppHandle,
    core: State<'_, SharedCore>,
    registry: State<'_, PathRegistry>,
    path_token: String,
) -> Result<OpmlImportOutcomeDto, CommandError> {
    let path = registry.redeem(&path_token, PathIntent::ImportFile)?;
    let core = Arc::clone(core.inner());
    let outcome = run_blocking(move || import_opml_impl(&core, &path)).await?;
    if outcome.added > 0 {
        emit_or_log(&app, &FeedsChanged);
    }
    Ok(outcome)
}

/// Imports a dialog-picked refugee export (OPML, or a Pocket / Instapaper
/// / Readwise CSV). Feeds become subscriptions; saved articles become
/// feedless read-later items carrying their source tags. Re-importing the
/// same file is idempotent — already-known URLs are skipped.
#[tauri::command]
#[specta::specta]
pub async fn import_file(
    app: AppHandle,
    core: State<'_, SharedCore>,
    registry: State<'_, PathRegistry>,
    path_token: String,
    source: ImportSourceDto,
) -> Result<ImportOutcomeDto, CommandError> {
    let path = registry.redeem(&path_token, PathIntent::ImportFile)?;
    let core = Arc::clone(core.inner());
    let outcome = run_blocking(move || import_file_impl(&core, &path, source.into())).await?;
    if outcome.feeds_added > 0 {
        emit_or_log(&app, &FeedsChanged);
    }
    if outcome.articles_added > 0 {
        emit_or_log(&app, &ArticlesChanged { feed_id: None });
    }
    Ok(outcome)
}

/// Exports every subscription as OPML 2.0 to a dialog-picked path.
#[tauri::command]
#[specta::specta]
pub async fn export_opml(
    core: State<'_, SharedCore>,
    registry: State<'_, PathRegistry>,
    path_token: String,
) -> Result<(), CommandError> {
    let path = registry.redeem(&path_token, PathIntent::ExportFile)?;
    let core = Arc::clone(core.inner());
    run_blocking(move || export_opml_impl(&core, &path)).await
}

// ------------------------------------------------------------------ impls

fn import_opml_impl(core: &CoreHandle, path: &Path) -> Result<OpmlImportOutcomeDto, CommandError> {
    let xml = read_import_file(path).map_err(|error| {
        CommandError::user(
            ErrorCode::Io,
            format!("could not read the OPML file: {error}"),
            true,
        )
    })?;
    Ok(core.import_opml(&xml)?.into())
}

fn import_file_impl(
    core: &CoreHandle,
    path: &Path,
    source: ImportSource,
) -> Result<ImportOutcomeDto, CommandError> {
    let content = read_import_file(path).map_err(|error| {
        CommandError::user(
            ErrorCode::Io,
            format!("could not read the import file: {error}"),
            true,
        )
    })?;
    Ok(core.import_file(source, &content)?.into())
}

fn export_opml_impl(core: &CoreHandle, path: &Path) -> Result<(), CommandError> {
    let xml = core.export_opml()?;
    write_file_atomic(path, xml.as_bytes()).map_err(|error| {
        CommandError::user(
            ErrorCode::Io,
            format!("could not write the OPML file: {error}"),
            true,
        )
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::temp_core;
    use crate::dto::{ImportKindDto, ImportRejectReasonDto};
    use crate::error::ErrorKind;

    const OPML: &str = r#"<?xml version="1.0"?>
<opml version="2.0"><head><title>subs</title></head><body>
<outline text="News"><outline type="rss" text="A" xmlUrl="https://a.test/feed.xml"/></outline>
<outline type="rss" text="B" xmlUrl="https://b.test/feed.xml"/>
</body></opml>"#;

    #[test]
    fn import_subscribes_and_skips_duplicates_on_reimport() {
        let (dir, core) = temp_core();
        let file = dir.path().join("subs.opml");
        std::fs::write(&file, OPML).unwrap();

        let first = import_opml_impl(&core, &file).unwrap();
        assert_eq!(first.added, 2);
        assert_eq!(first.skipped, 0);

        let again = import_opml_impl(&core, &file).unwrap();
        assert_eq!(again.added, 0);
        assert_eq!(again.skipped, 2);

        let feeds = core.list_feeds().unwrap();
        assert_eq!(feeds.len(), 2);
        let folder_tagged = feeds
            .iter()
            .find(|f| f.url == "https://a.test/feed.xml")
            .unwrap();
        assert_eq!(folder_tagged.tags, vec!["News".to_owned()]);
    }

    #[test]
    fn export_round_trips_through_import() {
        let (dir, core) = temp_core();
        let source = dir.path().join("in.opml");
        std::fs::write(&source, OPML).unwrap();
        import_opml_impl(&core, &source).unwrap();

        let out = dir.path().join("out.opml");
        export_opml_impl(&core, &out).unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert!(written.contains("https://a.test/feed.xml"));
        assert!(written.contains("https://b.test/feed.xml"));

        let (dir2, core2) = temp_core();
        let copy = dir2.path().join("copy.opml");
        std::fs::write(&copy, &written).unwrap();
        assert_eq!(import_opml_impl(&core2, &copy).unwrap().added, 2);
    }

    #[test]
    fn export_over_an_existing_file_replaces_it_whole() {
        let (dir, core) = temp_core();
        let source = dir.path().join("in.opml");
        std::fs::write(&source, OPML).unwrap();
        import_opml_impl(&core, &source).unwrap();

        // A previous, longer export: a non-truncating write would leave
        // its tail behind. Atomic replace leaves exactly the new document.
        let out = dir.path().join("out.opml");
        std::fs::write(&out, "x".repeat(64 * 1024)).unwrap();
        export_opml_impl(&core, &out).unwrap();
        let written = std::fs::read_to_string(&out).unwrap();
        assert_eq!(written, core.export_opml().unwrap());
        let strays = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|entry| {
                entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains(".curio-tmp-")
            })
            .count();
        assert_eq!(strays, 0, "no temp file left behind");
    }

    #[test]
    fn a_failed_export_is_a_user_error_and_writes_nothing() {
        let (dir, core) = temp_core();
        let out = dir.path().join("no-such-folder").join("out.opml");
        let error = export_opml_impl(&core, &out).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert!(!out.exists());
    }

    #[test]
    fn latin1_opml_and_utf16_csv_files_import() {
        let (dir, core) = temp_core();
        let latin1 = dir.path().join("latin1.opml");
        std::fs::write(
            &latin1,
            b"<?xml version=\"1.0\" encoding=\"ISO-8859-1\"?><opml><body>\
              <outline text=\"Caf\xE9\" xmlUrl=\"feed://cafe.test/rss\"/>\
              <outline text=\"Cal\" xmlUrl=\"webcal://cal.test/x.ics\"/></body></opml>",
        )
        .unwrap();
        let outcome = import_opml_impl(&core, &latin1).unwrap();
        assert_eq!((outcome.added, outcome.invalid), (1, 1));
        let feeds = core.list_feeds().unwrap();
        assert_eq!(feeds[0].url, "https://cafe.test/rss");
        assert_eq!(feeds[0].title.as_deref(), Some("Café"));

        let csv = dir.path().join("excel.csv");
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend(
            "url,title\nhttps://a.test/x,Été\njavascript:void(0),Bad\n"
                .encode_utf16()
                .flat_map(u16::to_le_bytes),
        );
        std::fs::write(&csv, bytes).unwrap();
        let outcome = import_file_impl(&core, &csv, ImportSource::PocketCsv).unwrap();
        assert_eq!(outcome.articles_added, 1);
        assert_eq!(outcome.articles_invalid, 1);
        assert_eq!(outcome.rejected.len(), 1);
        assert_eq!(
            outcome.rejected[0].reason,
            ImportRejectReasonDto::UnsupportedScheme
        );
        assert_eq!(outcome.rejected[0].kind, ImportKindDto::Article);
        assert_eq!(outcome.rejected[0].detail, "javascript:void(0)");
    }

    #[test]
    fn unreadable_paths_are_user_errors() {
        let (dir, core) = temp_core();
        let error = import_opml_impl(&core, &dir.path().join("missing.opml")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
    }

    #[test]
    fn csv_import_goes_through_the_generic_file_path() {
        let (dir, core) = temp_core();
        let file = dir.path().join("pocket.csv");
        std::fs::write(
            &file,
            "title,url,time_added,tags,status\nA,https://a.test/x,1700000000,rust,unread\n",
        )
        .unwrap();

        let outcome = import_file_impl(&core, &file, ImportSource::PocketCsv).unwrap();
        assert_eq!(outcome.articles_added, 1);
        assert_eq!(outcome.feeds_added, 0);
        assert_eq!(core.list_feeds().unwrap().len(), 0, "a CSV adds no feeds");
    }

    #[test]
    fn a_missing_url_column_is_a_user_error() {
        let (dir, core) = temp_core();
        let file = dir.path().join("wrong.csv");
        std::fs::write(&file, "a,b\n1,2\n").unwrap();
        // The Pocket parser can't find a URL column — surfaced, not panicked.
        let error = import_file_impl(&core, &file, ImportSource::PocketCsv).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
    }
}
