//! Destination registry + promote-to-destination.
//!
//! The contract rule (D6/D13): promotion is always "promote article X to
//! destination NAME" — raw filesystem paths never cross IPC. New roots
//! arrive only as [`crate::ipc_policy`] tokens from a Rust-side folder
//! pick; core re-asserts containment on every write underneath.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::model::ArticleId;
use curio_types::DestinationName;
use tauri::State;

use super::{SharedCore, run_blocking};
use crate::dto::{BulkSaveOutcomeDto, DestinationDto, ListArticlesDto, SaveOutcomeDto};
use crate::error::CommandError;
use crate::ipc_policy::{PathIntent, PathRegistry};

/// Registers (or re-roots) a named destination. The root arrives as a
/// dialog-pick token, never a raw path string.
#[tauri::command]
#[specta::specta]
pub async fn add_destination(
    core: State<'_, SharedCore>,
    registry: State<'_, PathRegistry>,
    name: String,
    path_token: String,
) -> Result<(), CommandError> {
    let root = registry.redeem(&path_token, PathIntent::DestinationRoot)?;
    let core = Arc::clone(core.inner());
    run_blocking(move || add_destination_impl(&core, &name, root)).await
}

/// Every registered destination.
#[tauri::command]
#[specta::specta]
pub async fn list_destinations(
    core: State<'_, SharedCore>,
) -> Result<Vec<DestinationDto>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || Ok(list_destinations_impl(&core))).await
}

/// Unregisters a destination name. Exported notes are untouched.
#[tauri::command]
#[specta::specta]
pub async fn remove_destination(
    core: State<'_, SharedCore>,
    name: String,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || remove_destination_impl(&core, &name)).await
}

/// Promotes an article into a destination by NAME: writes the
/// `curio.frontmatter.v1` note + manifest, emits `article.saved` /
/// `article.updated` (or nothing on the idempotency hit).
#[tauri::command]
#[specta::specta]
pub async fn promote_article(
    core: State<'_, SharedCore>,
    article_id: i64,
    destination: String,
) -> Result<SaveOutcomeDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || promote_article_impl(&core, article_id, &destination)).await
}

/// Bulk-promotes every article matching `filter` into a destination —
/// the export-everything path. Idempotent per note (manifest checksums);
/// `filter.before`/`limit` are ignored (pagination is internal).
#[tauri::command]
#[specta::specta]
pub async fn promote_all(
    core: State<'_, SharedCore>,
    destination: String,
    filter: ListArticlesDto,
) -> Result<BulkSaveOutcomeDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || promote_all_impl(&core, &destination, filter)).await
}

// ------------------------------------------------------------------ impls

fn add_destination_impl(
    core: &CoreHandle,
    name: &str,
    root: std::path::PathBuf,
) -> Result<(), CommandError> {
    let name: DestinationName = name.parse()?;
    Ok(core.add_destination(name, root)?)
}

fn list_destinations_impl(core: &CoreHandle) -> Vec<DestinationDto> {
    core.destinations()
        .into_iter()
        .map(DestinationDto::from)
        .collect()
}

fn remove_destination_impl(core: &CoreHandle, name: &str) -> Result<(), CommandError> {
    let name: DestinationName = name.parse()?;
    Ok(core.remove_destination(&name)?)
}

fn promote_article_impl(
    core: &CoreHandle,
    article_id: i64,
    destination: &str,
) -> Result<SaveOutcomeDto, CommandError> {
    let name: DestinationName = destination.parse()?;
    Ok(core
        .save_to_destination(ArticleId(article_id), &name)?
        .into())
}

fn promote_all_impl(
    core: &CoreHandle,
    destination: &str,
    filter: ListArticlesDto,
) -> Result<BulkSaveOutcomeDto, CommandError> {
    let name: DestinationName = destination.parse()?;
    Ok(core.save_all_to_destination(&filter.into(), &name)?.into())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};
    use crate::dto::ExportDispositionDto;
    use crate::error::{ErrorCode, ErrorKind};

    #[test]
    fn register_list_remove_round_trip() {
        let (dir, core) = temp_core();
        let root = dir.path().join("vault");
        add_destination_impl(&core, "vault", root.clone()).unwrap();
        let listed = list_destinations_impl(&core);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "vault");
        assert!(root.is_dir(), "registration creates the root");

        remove_destination_impl(&core, "vault").unwrap();
        assert!(list_destinations_impl(&core).is_empty());
        let error = remove_destination_impl(&core, "vault").unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownDestination);
    }

    #[test]
    fn invalid_destination_names_are_user_input_errors() {
        let (dir, core) = temp_core();
        let error = add_destination_impl(&core, "../escape", dir.path().to_path_buf()).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::InvalidInput);
    }

    #[test]
    fn promote_writes_a_note_and_is_idempotent() {
        let (dir, core) = temp_core();
        let id = seed_article(&core, "promoted");
        add_destination_impl(&core, "notes", dir.path().join("notes")).unwrap();

        let first = promote_article_impl(&core, id, "notes").unwrap();
        assert_eq!(first.disposition, ExportDispositionDto::Created);
        assert!(first.checksum.starts_with("sha256:"));
        let note = dir.path().join("notes").join(&first.path);
        assert!(note.is_file(), "note written under the destination root");

        let second = promote_article_impl(&core, id, "notes").unwrap();
        assert_eq!(second.disposition, ExportDispositionDto::Unchanged);
    }

    #[test]
    fn promote_all_walks_the_filtered_set_idempotently() {
        let (dir, core) = temp_core();
        let first_id = seed_article(&core, "one");
        seed_article(&core, "two");
        core.set_read_later(curio_core::model::ArticleId(first_id), true)
            .unwrap();
        add_destination_impl(&core, "notes", dir.path().join("notes")).unwrap();

        let filter = ListArticlesDto {
            feed_id: None,
            before: None,
            limit: 50,
            read: None,
            starred: None,
            read_later: Some(true),
            archived: None,
            tag: None,
            feed_tag: None,
        };
        let later_only = promote_all_impl(&core, "notes", filter.clone()).unwrap();
        assert_eq!(later_only.created, 1);

        let all = promote_all_impl(
            &core,
            "notes",
            ListArticlesDto {
                read_later: None,
                ..filter
            },
        )
        .unwrap();
        assert_eq!(all.created, 1, "only the not-yet-exported article");
        assert_eq!(all.unchanged, 1);

        let error = promote_all_impl(
            &core,
            "nowhere",
            ListArticlesDto {
                feed_id: None,
                before: None,
                limit: 50,
                read: None,
                starred: None,
                read_later: None,
                archived: None,
                tag: None,
                feed_tag: None,
            },
        )
        .unwrap_err();
        assert_eq!(error.code, ErrorCode::UnknownDestination);
    }

    #[test]
    fn promoting_to_an_unknown_name_is_a_user_error() {
        let (_dir, core) = temp_core();
        let id = seed_article(&core, "lost");
        let error = promote_article_impl(&core, id, "nowhere").unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
        assert_eq!(error.code, ErrorCode::UnknownDestination);
    }
}
