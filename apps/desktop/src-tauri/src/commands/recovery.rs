//! The recovery screen's commands: what failed at startup, and the
//! actions that act on it (see [`crate::startup`]). These need no core —
//! they are the only commands that work when the profile did not open.
//! Every action except reveal and quit is refused once the app is running
//! normally, so a healthy database can never be moved aside.

use tauri::{AppHandle, Manager as _, State};

use super::run_blocking;
use crate::error::{CommandError, ErrorCode};
use crate::startup::{self, RecoveryAction, RecoveryOutcomeDto, StartupFailureDto, StartupGate};

/// Why startup failed, or `None` when the app started normally — the
/// frontend asks this first, to pick the reader or the recovery screen.
#[tauri::command]
#[specta::specta]
pub async fn get_startup_failure(
    gate: State<'_, StartupGate>,
) -> Result<Option<StartupFailureDto>, CommandError> {
    gate.current()
}

/// Opens the profile again unchanged (after quitting another copy, or
/// freeing disk space).
#[tauri::command]
#[specta::specta]
pub async fn retry_startup(app: AppHandle) -> Result<RecoveryOutcomeDto, CommandError> {
    run_recovery(app, RecoveryAction::Retry).await
}

/// Moves the database aside and restores the newest pre-migration backup
/// this build can open, then opens the profile.
#[tauri::command]
#[specta::specta]
pub async fn restore_startup_backup(app: AppHandle) -> Result<RecoveryOutcomeDto, CommandError> {
    run_recovery(app, RecoveryAction::RestoreBackup).await
}

/// Moves the database aside (never deletes it) and opens an empty one.
#[tauri::command]
#[specta::specta]
pub async fn start_fresh_profile(app: AppHandle) -> Result<RecoveryOutcomeDto, CommandError> {
    run_recovery(app, RecoveryAction::StartFresh).await
}

/// Shows the failed profile's database (or folder) in the file manager.
#[tauri::command]
#[specta::specta]
pub async fn reveal_profile_folder(gate: State<'_, StartupGate>) -> Result<(), CommandError> {
    let target = gate.reveal_target()?;
    run_blocking(move || {
        tauri_plugin_opener::reveal_item_in_dir(&target)
            .map_err(|error| CommandError::user(ErrorCode::Io, error.to_string(), true))
    })
    .await
}

/// Quits the app (the recovery screen's last resort).
#[tauri::command]
#[specta::specta]
pub async fn quit_app(app: AppHandle) -> Result<(), CommandError> {
    app.exit(0);
    Ok(())
}

/// Runs `action` off the IPC reactor (it moves files and opens the
/// database), installing the core if the profile now opens.
async fn run_recovery(
    app: AppHandle,
    action: RecoveryAction,
) -> Result<RecoveryOutcomeDto, CommandError> {
    run_blocking(move || {
        let gate = app.try_state::<StartupGate>().ok_or_else(|| {
            CommandError::internal(ErrorCode::Internal, "the startup gate is not managed")
        })?;
        let recovery = startup::recover(&gate, action)?;
        startup::finish(&app, &gate, recovery)
    })
    .await
}
