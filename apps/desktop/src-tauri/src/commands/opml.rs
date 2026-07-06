//! OPML import/export. Rust does the file IO; both paths arrive as
//! [`crate::ipc_policy`] tokens from Rust-side dialog picks — never raw
//! strings.

use std::path::Path;
use std::sync::Arc;

use curio_core::CoreHandle;
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::dto::OpmlImportOutcomeDto;
use crate::error::{CommandError, ErrorCode};
use crate::events::{FeedsChanged, emit_or_log};
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
    let xml = std::fs::read_to_string(path).map_err(|error| {
        CommandError::user(
            ErrorCode::Io,
            format!("could not read the OPML file: {error}"),
            true,
        )
    })?;
    Ok(core.import_opml(&xml)?.into())
}

fn export_opml_impl(core: &CoreHandle, path: &Path) -> Result<(), CommandError> {
    let xml = core.export_opml()?;
    std::fs::write(path, xml).map_err(|error| {
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
    fn unreadable_paths_are_user_errors() {
        let (dir, core) = temp_core();
        let error = import_opml_impl(&core, &dir.path().join("missing.opml")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
    }
}
