//! The recovery screen's commands: what failed at startup, and the
//! actions that act on it (see [`crate::startup`]). These need no core —
//! they are the only commands that work when the profile did not open.
//! Every action except reveal and quit is refused once the app is running
//! normally, so a healthy database can never be moved aside.
//!
//! A recovery action holds the [`StartupGate`]'s lock while it moves files
//! and re-opens the profile (which can run a whole upgrade), so every
//! command that touches the gate does so on a blocking worker, never on an
//! async runtime thread.

use tauri::{AppHandle, Manager as _};

use super::run_blocking;
use crate::error::{CommandError, ErrorCode};
use crate::startup::{self, RecoveryAction, RecoveryOutcomeDto, StartupFailureDto, StartupGate};

/// Why startup failed, or `None` when the app started normally — the
/// frontend asks this first, to pick the reader or the recovery screen.
#[tauri::command]
#[specta::specta]
pub async fn get_startup_failure(
    app: AppHandle,
) -> Result<Option<StartupFailureDto>, CommandError> {
    with_gate("get_startup_failure", app, |_, gate| gate.current()).await
}

/// Opens the profile again unchanged (after quitting another copy, or
/// freeing disk space).
#[tauri::command]
#[specta::specta]
pub async fn retry_startup(app: AppHandle) -> Result<RecoveryOutcomeDto, CommandError> {
    run_recovery("retry_startup", app, RecoveryAction::Retry).await
}

/// Moves the database aside and restores the newest pre-migration backup
/// this build can open, then opens the profile.
#[tauri::command]
#[specta::specta]
pub async fn restore_startup_backup(app: AppHandle) -> Result<RecoveryOutcomeDto, CommandError> {
    run_recovery("restore_startup_backup", app, RecoveryAction::RestoreBackup).await
}

/// Moves the database aside (never deletes it) and opens an empty one.
#[tauri::command]
#[specta::specta]
pub async fn start_fresh_profile(app: AppHandle) -> Result<RecoveryOutcomeDto, CommandError> {
    run_recovery("start_fresh_profile", app, RecoveryAction::StartFresh).await
}

/// Shows the failed profile's database (or folder) in the file manager.
#[tauri::command]
#[specta::specta]
pub async fn reveal_profile_folder(app: AppHandle) -> Result<(), CommandError> {
    with_gate("reveal_profile_folder", app, |_, gate| {
        let target = gate.reveal_target()?;
        tauri_plugin_opener::reveal_item_in_dir(&target)
            .map_err(|error| CommandError::user(ErrorCode::Io, error.to_string(), true))
    })
    .await
}

/// Quits the app (the recovery screen's last resort). Waits for a
/// recovery action in flight to finish first, so a quit never lands
/// between two file moves.
#[tauri::command]
#[specta::specta]
pub async fn quit_app(app: AppHandle) -> Result<(), CommandError> {
    with_gate("quit_app", app, |app, gate| {
        let _idle = gate.wait_idle()?;
        app.exit(0);
        Ok(())
    })
    .await
}

/// Runs `action` off the IPC reactor (it moves files and opens the
/// database), installing the core if the profile now opens.
async fn run_recovery(
    command: &'static str,
    app: AppHandle,
    action: RecoveryAction,
) -> Result<RecoveryOutcomeDto, CommandError> {
    with_gate(command, app, move |app, gate| {
        let recovery = startup::recover(gate, action)?;
        startup::finish(app, gate, recovery)
    })
    .await
}

/// Runs `job` against the managed [`StartupGate`] on a blocking worker:
/// the gate's lock can be held for as long as a recovery action takes.
/// `command` names the calling command for [`run_blocking`]'s log.
async fn with_gate<T, F>(command: &'static str, app: AppHandle, job: F) -> Result<T, CommandError>
where
    F: FnOnce(&AppHandle, &StartupGate) -> Result<T, CommandError> + Send + 'static,
    T: Send + 'static,
{
    run_blocking(command, move || {
        let gate = app.try_state::<StartupGate>().ok_or_else(|| {
            CommandError::internal(ErrorCode::Internal, "the startup gate is not managed")
        })?;
        job(&app, &gate)
    })
    .await
}
