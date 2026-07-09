//! Head-local platform commands: dialog picks (the only source of
//! [`crate::ipc_policy`] tokens), the policed image cache, and the
//! redacted diagnostics bundle (zero telemetry — nothing leaves the
//! machine; the user copies the bundle by hand).

use std::path::PathBuf;
use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::storage::Storage;
use tauri::{AppHandle, State};
use tauri_plugin_dialog::DialogExt as _;

use super::{SharedCore, run_blocking};
use crate::dto::PathTokenDto;
use crate::error::{CommandError, ErrorCode};
use crate::image_cache::ImageCache;
use crate::ipc_policy::{PathIntent, PathRegistry};

/// The local path of a cached copy of `url`, fetched through the policed
/// client on a miss. The frontend serves it via the `asset:` protocol —
/// this is how images load without opening the CSP to `https:`.
#[tauri::command]
#[specta::specta]
pub async fn get_cached_image(
    cache: State<'_, ImageCache>,
    url: String,
) -> Result<String, CommandError> {
    let path = cache.get(&url).await?;
    Ok(path.display().to_string())
}

/// Builds a redacted diagnostics bundle (app facts + any rotating log
/// files) in the OS cache dir and returns a display/reveal token. Nothing
/// is transmitted anywhere.
#[tauri::command]
#[specta::specta]
pub async fn copy_diagnostics_bundle(
    core: State<'_, SharedCore>,
    registry: State<'_, PathRegistry>,
) -> Result<PathTokenDto, CommandError> {
    let core = Arc::clone(core.inner());
    let bundle_root = crate::paths::diagnostics_dir()?;
    let bundle = run_blocking(move || build_diagnostics_bundle(&core, &bundle_root)).await?;
    registry.mint(&bundle, PathIntent::Reveal)
}

/// Opens a native open-file dialog for a subscription/refugee import
/// (OPML or a Pocket/Instapaper/Readwise CSV); `None` = cancelled.
#[tauri::command]
#[specta::specta]
pub async fn pick_import_file(
    app: AppHandle,
    registry: State<'_, PathRegistry>,
) -> Result<Option<PathTokenDto>, CommandError> {
    let picked = run_blocking(move || {
        Ok(app
            .dialog()
            .file()
            .add_filter("Subscriptions & saves", &["opml", "xml", "csv"])
            .add_filter("OPML", &["opml", "xml"])
            .add_filter("CSV", &["csv"])
            .blocking_pick_file())
    })
    .await?;
    let Some(path) = picked.map(file_path_to_path).transpose()? else {
        return Ok(None);
    };
    registry.mint(&path, PathIntent::ImportFile).map(Some)
}

/// Opens a native save dialog for OPML export; `None` = cancelled.
#[tauri::command]
#[specta::specta]
pub async fn pick_export_path(
    app: AppHandle,
    registry: State<'_, PathRegistry>,
) -> Result<Option<PathTokenDto>, CommandError> {
    let picked = run_blocking(move || {
        Ok(app
            .dialog()
            .file()
            .add_filter("OPML", &["opml"])
            .set_file_name("curio-subscriptions.opml")
            .blocking_save_file())
    })
    .await?;
    let Some(path) = picked.map(file_path_to_path).transpose()? else {
        return Ok(None);
    };
    registry.mint(&path, PathIntent::ExportFile).map(Some)
}

/// Opens a native save dialog for a theme YAML export; `None` = cancelled.
#[tauri::command]
#[specta::specta]
pub async fn pick_theme_export_path(
    app: AppHandle,
    registry: State<'_, PathRegistry>,
) -> Result<Option<PathTokenDto>, CommandError> {
    let picked = run_blocking(move || {
        Ok(app
            .dialog()
            .file()
            .add_filter("Theme", &["yaml", "yml"])
            .set_file_name("curio-theme.yaml")
            .blocking_save_file())
    })
    .await?;
    let Some(path) = picked.map(file_path_to_path).transpose()? else {
        return Ok(None);
    };
    registry.mint(&path, PathIntent::ExportFile).map(Some)
}

/// Opens a native open-file dialog for a theme YAML import; `None` =
/// cancelled.
#[tauri::command]
#[specta::specta]
pub async fn pick_theme_import_file(
    app: AppHandle,
    registry: State<'_, PathRegistry>,
) -> Result<Option<PathTokenDto>, CommandError> {
    let picked = run_blocking(move || {
        Ok(app
            .dialog()
            .file()
            .add_filter("Theme", &["yaml", "yml"])
            .blocking_pick_file())
    })
    .await?;
    let Some(path) = picked.map(file_path_to_path).transpose()? else {
        return Ok(None);
    };
    registry.mint(&path, PathIntent::ImportFile).map(Some)
}

/// Opens a native folder picker for a new destination root; `None` =
/// cancelled.
#[tauri::command]
#[specta::specta]
pub async fn pick_destination_root(
    app: AppHandle,
    registry: State<'_, PathRegistry>,
) -> Result<Option<PathTokenDto>, CommandError> {
    let picked = run_blocking(move || Ok(app.dialog().file().blocking_pick_folder())).await?;
    let Some(path) = picked.map(file_path_to_path).transpose()? else {
        return Ok(None);
    };
    registry.mint(&path, PathIntent::DestinationRoot).map(Some)
}

// ------------------------------------------------------------------ impls

fn file_path_to_path(picked: tauri_plugin_dialog::FilePath) -> Result<PathBuf, CommandError> {
    picked.into_path().map_err(|error| {
        CommandError::invalid_input(format!("the picked location is not a local path: {error}"))
    })
}

/// Writes `<root>/curio-diagnostics-<ulid>/` with an `app-info.txt` and a
/// copy of any `*.log` files from the profile's `logs/` dir. Redaction
/// posture: only allowlisted files are copied, and the info file contains
/// no feed URLs, no article data, no credentials.
fn build_diagnostics_bundle(
    core: &CoreHandle,
    bundle_root: &std::path::Path,
) -> Result<PathBuf, CommandError> {
    let bundle = bundle_root.join(format!("curio-diagnostics-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&bundle).map_err(|error| io_internal(&bundle, &error))?;

    let info = format!(
        "curio desktop diagnostics\n\
         version: {}\n\
         os: {} ({})\n\
         db schema: {} (supported: {})\n\
         article count: {}\n",
        CoreHandle::version(),
        std::env::consts::OS,
        std::env::consts::ARCH,
        core.storage().db_schema_version()?,
        Storage::schema_version(),
        core.storage().count_articles()?,
    );
    let info_path = bundle.join("app-info.txt");
    std::fs::write(&info_path, info).map_err(|error| io_internal(&info_path, &error))?;

    let logs_dir = core.profile_dir().join("logs");
    if let Ok(entries) = std::fs::read_dir(&logs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str())
                && is_log_file(name)
            {
                let target = bundle.join(name);
                if let Err(error) = std::fs::copy(&path, &target) {
                    tracing::warn!(%error, "skipping unreadable log file in diagnostics bundle");
                }
            }
        }
    }
    Ok(bundle)
}

/// The allowlist: our own log files, including day-rotated ones
/// (`curio.log`, `curio.log.2026-07-06`). Anything else in the logs dir —
/// a stray note, an editor swapfile — is never copied.
fn is_log_file(name: &str) -> bool {
    name == "curio.log" || name.starts_with("curio.log.")
}

fn io_internal(path: &std::path::Path, error: &std::io::Error) -> CommandError {
    CommandError::internal(
        ErrorCode::Io,
        format!("diagnostics io at {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::{seed_article, temp_core};

    #[test]
    fn diagnostics_bundle_contains_app_info_and_only_log_files() {
        let (dir, core) = temp_core();
        seed_article(&core, "diag");
        // Log files that must be copied (including a day-rotated one), and a
        // secret that must not.
        let logs = core.profile_dir().join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("curio.log"), "line").unwrap();
        std::fs::write(logs.join("curio.log.2026-07-06"), "rotated").unwrap();
        std::fs::write(logs.join("secrets.txt"), "nope").unwrap();

        let bundle = build_diagnostics_bundle(&core, &dir.path().join("bundles")).unwrap();
        let info = std::fs::read_to_string(bundle.join("app-info.txt")).unwrap();
        assert!(info.contains("article count: 1"));
        assert!(bundle.join("curio.log").is_file());
        assert!(
            bundle.join("curio.log.2026-07-06").is_file(),
            "day-rotated logs are allowlisted too"
        );
        assert!(
            !bundle.join("secrets.txt").exists(),
            "only curio log files are allowlisted into the bundle"
        );
    }

    #[test]
    fn log_allowlist_matches_plain_and_rotated_names_only() {
        assert!(is_log_file("curio.log"));
        assert!(is_log_file("curio.log.2026-07-06"));
        assert!(!is_log_file("secrets.txt"));
        assert!(!is_log_file("curio.db"));
        assert!(!is_log_file("notcurio.log"));
    }
}
