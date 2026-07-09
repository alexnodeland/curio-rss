//! Theme file IO: read a dialog-picked theme file and write theme text to a
//! dialog-picked path. Like the OPML commands, every path arrives as a
//! [`crate::ipc_policy`] token from a Rust-side dialog pick — never a raw
//! string. These commands are format-agnostic text IO; the frontend owns the
//! `curio.theme/v1` YAML format, its parsing, and its validation.

use std::path::Path;

use tauri::State;

use super::run_blocking;
use crate::error::{CommandError, ErrorCode};
use crate::ipc_policy::{PathIntent, PathRegistry};

/// Writes `contents` verbatim to a dialog-picked path (theme YAML export).
#[tauri::command]
#[specta::specta]
pub async fn export_text_file(
    registry: State<'_, PathRegistry>,
    path_token: String,
    contents: String,
) -> Result<(), CommandError> {
    let path = registry.redeem(&path_token, PathIntent::ExportFile)?;
    run_blocking(move || write_text_impl(&path, &contents)).await
}

/// Reads a dialog-picked text file (theme YAML import) and returns its
/// contents for the frontend to parse and validate.
#[tauri::command]
#[specta::specta]
pub async fn read_text_file(
    registry: State<'_, PathRegistry>,
    path_token: String,
) -> Result<String, CommandError> {
    let path = registry.redeem(&path_token, PathIntent::ImportFile)?;
    run_blocking(move || read_text_impl(&path)).await
}

// ------------------------------------------------------------------ impls

fn write_text_impl(path: &Path, contents: &str) -> Result<(), CommandError> {
    std::fs::write(path, contents).map_err(|error| {
        CommandError::user(
            ErrorCode::Io,
            format!("could not write the file: {error}"),
            true,
        )
    })
}

fn read_text_impl(path: &Path) -> Result<String, CommandError> {
    std::fs::read_to_string(path).map_err(|error| {
        CommandError::user(
            ErrorCode::Io,
            format!("could not read the file: {error}"),
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

    const THEME: &str = "schema: curio.theme/v1\nid: my-theme\nname: My Theme\n";

    #[test]
    fn write_then_read_round_trips_verbatim() {
        let (dir, _core) = temp_core();
        let file = dir.path().join("theme.yaml");
        write_text_impl(&file, THEME).unwrap();
        assert_eq!(read_text_impl(&file).unwrap(), THEME);
    }

    #[test]
    fn reading_a_missing_file_is_a_user_error() {
        let (dir, _core) = temp_core();
        let error = read_text_impl(&dir.path().join("missing.yaml")).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
    }

    #[test]
    fn writing_into_a_missing_directory_is_a_user_error() {
        let (dir, _core) = temp_core();
        let error = write_text_impl(&dir.path().join("nope/theme.yaml"), THEME).unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
    }
}
