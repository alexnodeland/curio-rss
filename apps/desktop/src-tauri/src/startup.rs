//! Two-phase startup and the recovery path.
//!
//! A bundled app has no visible stderr, so a profile that will not open
//! (a corrupt `curio.db`, a schema written by a newer build, a lock held
//! by another process, an unwritable data dir, a failed upgrade) must
//! never end in `exit(1)`. Instead:
//!
//! 1. **Open** — [`boot`] resolves the profile and opens the core. On
//!    failure it logs the cause with `tracing::error!` (so it lands in
//!    `logs/curio.log.*`) and returns a structured [`StartupFailure`].
//! 2. **Install** — on success, [`install`] manages the core and its
//!    dependents and starts the background work (the refresh scheduler,
//!    then the keychain read *off* the main thread). On failure, setup
//!    records the failure in the [`StartupGate`] and the window comes up
//!    on the recovery screen instead of the reader.
//!
//! The recovery screen's actions ([`recover`]) never delete anything:
//! restoring a pre-migration backup or starting fresh first moves the
//! database (and its `-wal`/`-shm` sidecars) into
//! `<profile>/set-aside/<UTC timestamp>/`. After a successful recovery
//! the core is installed in-process and the frontend reloads into the app.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use curio_core::fetch::{FetchConfig, PolicedClient};
use curio_core::storage::{OpenError, PreMigrationBackup, pre_migration_backups};
use curio_core::{CoreError, CoreHandle, CoreOptions};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager as _};

use crate::commands::{self, SharedCore};
use crate::error::{CommandError, ErrorCode};
use crate::{image_cache, paths, refresh_scheduler};

/// The database file inside a profile, followed by the `SQLite` sidecars
/// that must travel with it.
const DB_FILES: [&str; 3] = ["curio.db", "curio.db-wal", "curio.db-shm"];

/// Where recovery moves a database it replaces, inside the profile.
pub const SET_ASIDE_DIR: &str = "set-aside";

/// Why startup failed — the class the recovery screen branches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum StartupFailureClass {
    /// `curio.db` is not a readable `SQLite` database.
    Corrupt,
    /// The database was written by a newer build of Curio.
    TooNew,
    /// Another process holds the database.
    Locked,
    /// The OS refused a read or write (permissions, disk full, read-only
    /// media), for the database or the event log.
    Io,
    /// The schema upgrade (or its pre-migration backup) failed and was
    /// rolled back.
    Upgrade,
    /// The platform reported no home directory: there is no profile.
    NoProfile,
    /// Anything else.
    Other,
}

impl StartupFailureClass {
    /// Sorts a core open failure into its class.
    #[must_use]
    pub fn of(error: &CoreError) -> Self {
        match error {
            CoreError::Open(open) => match open {
                OpenError::Corrupt { .. } => Self::Corrupt,
                OpenError::TooNew { .. } => Self::TooNew,
                OpenError::Locked { .. } => Self::Locked,
                OpenError::Io { .. } => Self::Io,
                OpenError::Backup { .. } | OpenError::MigrationFailed { .. } => Self::Upgrade,
                // `Storage`, and any class a later core adds.
                _ => Self::Other,
            },
            CoreError::Io { .. } | CoreError::Events(_) => Self::Io,
            _ => Self::Other,
        }
    }

    /// Whether recovery may move the database aside. Never while another
    /// process holds it, and never without a profile.
    #[must_use]
    pub fn may_replace_database(self) -> bool {
        !matches!(self, Self::Locked | Self::NoProfile)
    }
}

/// A failed startup: what went wrong, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupFailure {
    /// The failure class.
    pub class: StartupFailureClass,
    /// The underlying error, as text.
    pub message: String,
    /// The profile directory, when one was resolved.
    pub profile: Option<PathBuf>,
}

impl StartupFailure {
    /// A failure to resolve paths at all (no home directory).
    fn no_profile(error: &CommandError) -> Self {
        Self {
            class: StartupFailureClass::NoProfile,
            message: error.message.clone(),
            profile: None,
        }
    }

    /// Writes the failure to the log. Startup failures are the one error
    /// a user can never see in a terminal, so the log is their record.
    fn log(&self) {
        let profile = self
            .profile
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        tracing::error!(class = ?self.class, %profile, "startup failed: {}", self.message);
    }

    /// The IPC shape, including the backup "Restore" would bring back.
    #[must_use]
    pub fn to_dto(&self) -> StartupFailureDto {
        let backup = self
            .profile
            .as_deref()
            .filter(|_| self.class.may_replace_database())
            .and_then(restorable_backup)
            .map(|backup| backup.path.display().to_string());
        StartupFailureDto {
            class: self.class,
            message: self.message.clone(),
            profile_dir: self
                .profile
                .as_deref()
                .map(|path| path.display().to_string()),
            backup,
        }
    }
}

/// A failed startup, as the recovery screen sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct StartupFailureDto {
    /// The failure class.
    pub class: StartupFailureClass,
    /// The underlying error (for "Copy error" and the details disclosure).
    pub message: String,
    /// The profile directory, when one was resolved.
    pub profile_dir: Option<String>,
    /// The pre-migration backup "Restore" would bring back: the newest one
    /// this build can open. `None` = nothing to restore.
    pub backup: Option<String>,
}

/// What a recovery action did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
pub struct RecoveryOutcomeDto {
    /// `None`: the profile opened and the app is running — reload into
    /// it. `Some`: it still fails, for this reason.
    pub failure: Option<StartupFailureDto>,
    /// Where the replaced database was moved, when anything was moved.
    pub set_aside: Option<String>,
}

/// Phase 1: resolve the profile and open the core. Every failure is
/// logged and returned as data — this never exits the process.
///
/// # Errors
///
/// A [`StartupFailure`] when there is no profile or it will not open.
pub fn boot(profile: Result<PathBuf, CommandError>) -> Result<CoreHandle, StartupFailure> {
    match profile {
        Ok(profile) => open_profile(&profile),
        Err(error) => {
            let failure = StartupFailure::no_profile(&error);
            failure.log();
            Err(failure)
        }
    }
}

/// Opens the core over `profile`, logging and classifying any failure.
///
/// # Errors
///
/// A [`StartupFailure`] carrying the failure class.
pub fn open_profile(profile: &Path) -> Result<CoreHandle, StartupFailure> {
    CoreHandle::open_with(profile, CoreOptions::default()).map_err(|error| {
        let failure = StartupFailure {
            class: StartupFailureClass::of(&error),
            message: error.to_string(),
            profile: Some(profile.to_path_buf()),
        };
        failure.log();
        failure
    })
}

/// Phase 2: manages the core and its dependents, then starts background
/// work. The keychain read runs on a blocking worker, not in `setup`: on
/// an unsigned build it can raise a macOS access prompt, which must not
/// hold up the window. Until it finishes, Reddit requests use the
/// unauthenticated tier (the same as having no credentials).
///
/// # Errors
///
/// A [`StartupFailure`] when the image cache directory cannot be resolved.
pub fn install(app: &AppHandle, core: CoreHandle) -> Result<(), StartupFailure> {
    let images = paths::image_cache_dir().map_err(|error| {
        let failure = StartupFailure {
            profile: Some(core.profile_dir().to_path_buf()),
            ..StartupFailure::no_profile(&error)
        };
        failure.log();
        failure
    })?;
    let core: SharedCore = Arc::new(core);
    app.manage(Arc::clone(&core));
    app.manage(image_cache::ImageCache::new(
        images,
        image_cache::DEFAULT_MAX_CACHE_BYTES,
        PolicedClient::new(FetchConfig::default()),
    ));
    // Background refresh: keep the reader current without a click. The
    // scheduler is shared so a manual `refresh_all` resets its timer.
    let scheduler = Arc::new(refresh_scheduler::RefreshScheduler::new());
    app.manage(Arc::clone(&scheduler));
    refresh_scheduler::spawn(app.clone(), Arc::clone(&core), scheduler);
    // Optional Reddit API credentials from the OS keychain (D15) —
    // best-effort; absence just means the unauthenticated tier.
    tauri::async_runtime::spawn_blocking(move || {
        commands::reddit::load_credentials_at_startup(&core);
    });
    Ok(())
}

/// The current startup failure, if any. Managed from the first line of
/// setup, so the frontend can always ask.
#[derive(Debug, Default)]
pub struct StartupGate {
    failure: Mutex<Option<StartupFailure>>,
}

impl StartupGate {
    /// Records `failure` as the reason the app is on the recovery screen.
    ///
    /// # Errors
    ///
    /// Internal error if the gate's lock is poisoned.
    pub fn record(&self, failure: StartupFailure) -> Result<(), CommandError> {
        *self.lock()? = Some(failure);
        Ok(())
    }

    /// The current failure, or `None` when the app started normally.
    ///
    /// # Errors
    ///
    /// Internal error if the gate's lock is poisoned.
    pub fn current(&self) -> Result<Option<StartupFailureDto>, CommandError> {
        Ok(self.lock()?.as_ref().map(StartupFailure::to_dto))
    }

    /// What "Reveal profile folder" shows: the database file when it
    /// exists, else the profile directory.
    ///
    /// # Errors
    ///
    /// User error when startup did not fail or there is no profile.
    pub fn reveal_target(&self) -> Result<PathBuf, CommandError> {
        let guard = self.lock()?;
        let profile = failed_profile(guard.as_ref())?;
        let db = profile.join(DB_FILES[0]);
        Ok(if db.exists() { db } else { profile })
    }

    fn lock(&self) -> Result<MutexGuard<'_, Option<StartupFailure>>, CommandError> {
        self.failure.lock().map_err(|_| {
            CommandError::internal(ErrorCode::Internal, "the startup gate lock is poisoned")
        })
    }
}

/// The profile of the current failure — recovery is refused when startup
/// did not fail (so a healthy profile can never be moved aside) or when
/// there is no profile to act on.
fn failed_profile(failure: Option<&StartupFailure>) -> Result<PathBuf, CommandError> {
    let failure = failure.ok_or_else(|| {
        CommandError::invalid_input("Curio started normally; there is nothing to recover")
    })?;
    failure.profile.clone().ok_or_else(|| {
        CommandError::user(
            ErrorCode::Io,
            "there is no profile folder to recover (the system reports no home directory)",
            false,
        )
    })
}

/// A recovery-screen action.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Open the profile again unchanged (e.g. after quitting the other
    /// copy that held the lock).
    Retry,
    /// Move the database aside and put the newest openable pre-migration
    /// backup in its place.
    RestoreBackup,
    /// Move the database aside and start with an empty one.
    StartFresh,
}

/// The result of [`recover`]: the opened core (to [`install`]) and what
/// to tell the frontend.
#[derive(Debug)]
pub struct Recovery {
    /// The core, when the profile now opens.
    pub core: Option<CoreHandle>,
    /// What happened, for the recovery screen.
    pub outcome: RecoveryOutcomeDto,
}

/// Runs a recovery action and re-opens the profile. The gate is cleared
/// when the profile opens and updated with the new failure when it does
/// not. Holding the gate's lock throughout serializes concurrent clicks.
///
/// # Errors
///
/// User error when startup did not fail, there is no profile, the action
/// would move a database another process holds, or there is no backup
/// to restore; internal error when moving files fails.
pub fn recover(gate: &StartupGate, action: RecoveryAction) -> Result<Recovery, CommandError> {
    let mut slot = gate.lock()?;
    let profile = failed_profile(slot.as_ref())?;
    let replaces_database = action != RecoveryAction::Retry;
    if replaces_database
        && slot
            .as_ref()
            .is_some_and(|f| !f.class.may_replace_database())
    {
        return Err(CommandError::user(
            ErrorCode::Storage,
            "the database is in use by another copy of Curio; quit it and try again",
            true,
        ));
    }
    let set_aside = match action {
        RecoveryAction::Retry => None,
        RecoveryAction::RestoreBackup => restore_backup(&profile)?,
        RecoveryAction::StartFresh => set_aside(&profile)?,
    };
    let set_aside = set_aside.map(|dir| dir.display().to_string());
    match open_profile(&profile) {
        Ok(core) => {
            tracing::info!(?action, "startup recovered");
            *slot = None;
            Ok(Recovery {
                core: Some(core),
                outcome: RecoveryOutcomeDto {
                    failure: None,
                    set_aside,
                },
            })
        }
        Err(failure) => {
            let dto = failure.to_dto();
            *slot = Some(failure);
            Ok(Recovery {
                core: None,
                outcome: RecoveryOutcomeDto {
                    failure: Some(dto),
                    set_aside,
                },
            })
        }
    }
}

/// Installs a recovered core; an install failure goes back into the gate.
///
/// # Errors
///
/// Internal error if the gate's lock is poisoned.
pub fn finish(
    app: &AppHandle,
    gate: &StartupGate,
    recovery: Recovery,
) -> Result<RecoveryOutcomeDto, CommandError> {
    let Recovery { core, mut outcome } = recovery;
    if let Some(core) = core
        && let Err(failure) = install(app, core)
    {
        outcome.failure = Some(failure.to_dto());
        gate.record(failure)?;
    }
    Ok(outcome)
}

/// The newest pre-migration backup of `profile`'s database that this
/// build can open.
fn restorable_backup(profile: &Path) -> Option<PreMigrationBackup> {
    pre_migration_backups(&profile.join(DB_FILES[0]))
        .into_iter()
        .find(PreMigrationBackup::opens_in_this_build)
}

/// Moves the database and its sidecars into a new
/// `<profile>/set-aside/<timestamp>/` folder. Returns that folder, or
/// `None` when there was nothing to move.
fn set_aside(profile: &Path) -> Result<Option<PathBuf>, CommandError> {
    let present: Vec<&str> = DB_FILES
        .into_iter()
        .filter(|name| profile.join(name).symlink_metadata().is_ok())
        .collect();
    if present.is_empty() {
        return Ok(None);
    }
    let dest = claim_set_aside_dir(profile)?;
    for name in present {
        let from = profile.join(name);
        std::fs::rename(&from, dest.join(name)).map_err(|error| {
            io_error(&format!(
                "moving {} into {}: {error}",
                from.display(),
                dest.display()
            ))
        })?;
    }
    tracing::warn!(set_aside = %dest.display(), "moved the profile database aside");
    Ok(Some(dest))
}

/// Creates (atomically claims) a fresh timestamped folder under
/// `<profile>/set-aside/`, suffixing `-2`, `-3`… on a same-second clash.
fn claim_set_aside_dir(profile: &Path) -> Result<PathBuf, CommandError> {
    let root = profile.join(SET_ASIDE_DIR);
    std::fs::create_dir_all(&root)
        .map_err(|error| io_error(&format!("creating {}: {error}", root.display())))?;
    let stamp = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    for attempt in 1..=100_u32 {
        let name = if attempt == 1 {
            stamp.clone()
        } else {
            format!("{stamp}-{attempt}")
        };
        let candidate = root.join(name);
        match std::fs::create_dir(&candidate) {
            Ok(()) => return Ok(candidate),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(io_error(&format!(
                    "creating {}: {error}",
                    candidate.display()
                )));
            }
        }
    }
    Err(io_error(&format!(
        "no free folder name under {}",
        root.display()
    )))
}

/// Puts the newest openable backup in place of the database, moving the
/// database aside first. The backup itself is copied, never moved.
fn restore_backup(profile: &Path) -> Result<Option<PathBuf>, CommandError> {
    let backup = restorable_backup(profile).ok_or_else(|| {
        CommandError::user(
            ErrorCode::NotFound,
            "there is no pre-migration backup this version of Curio can open",
            false,
        )
    })?;
    let db = profile.join(DB_FILES[0]);
    // Stage the copy first: if it fails, nothing has been moved yet.
    let staging = profile.join("curio.db.restoring");
    std::fs::copy(&backup.path, &staging)
        .map_err(|error| io_error(&format!("copying {}: {error}", backup.path.display())))?;
    let aside = match set_aside(profile) {
        Ok(aside) => aside,
        Err(error) => {
            // Best-effort cleanup; the error that matters is the move.
            let _ = std::fs::remove_file(&staging);
            return Err(error);
        }
    };
    std::fs::rename(&staging, &db)
        .map_err(|error| io_error(&format!("moving the restored copy into place: {error}")))?;
    tracing::warn!(backup = %backup.path.display(), "restored a pre-migration backup");
    Ok(aside)
}

fn io_error(detail: &str) -> CommandError {
    CommandError::internal(ErrorCode::Io, format!("recovery failed {detail}"))
}

#[cfg(test)]
mod tests {
    // Test code: a failed unwrap is the assertion.
    #![allow(clippy::unwrap_used)]

    use super::*;
    use curio_core::model::NewFeed;
    use curio_core::storage::Storage;

    const GARBAGE: &[u8] = b"definitely not sqlite, just long enough to have a header";

    fn garbage_profile() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        std::fs::write(profile.join("curio.db"), GARBAGE).unwrap();
        (dir, profile)
    }

    fn gate_for(profile: &Path) -> StartupGate {
        let gate = StartupGate::default();
        let failure = open_profile(profile).err().unwrap();
        gate.record(failure).unwrap();
        gate
    }

    /// Writes a pre-migration backup holding one feed, as a real upgrade
    /// would have (`VACUUM INTO` a self-contained copy).
    fn write_backup_with_a_feed(profile: &Path) -> PathBuf {
        let source = tempfile::tempdir().unwrap();
        let core = CoreHandle::open(source.path()).unwrap();
        core.add_feed(NewFeed {
            url: "https://example.test/feed.xml".into(),
            title: Some("Kept".into()),
            tags: Vec::new(),
        })
        .unwrap();
        drop(core);
        let version = Storage::schema_version();
        let backup = profile.join(format!("curio.db.pre-v{version}.bak"));
        let conn = rusqlite::Connection::open(source.path().join("curio.db")).unwrap();
        conn.execute("VACUUM INTO ?1", [backup.to_str().unwrap()])
            .unwrap();
        backup
    }

    #[test]
    fn a_garbage_database_is_a_structured_corrupt_failure() {
        let (_dir, profile) = garbage_profile();
        let failure = boot(Ok(profile.clone())).err().unwrap();
        assert_eq!(failure.class, StartupFailureClass::Corrupt);
        assert_eq!(failure.profile.as_deref(), Some(profile.as_path()));
        assert!(
            failure.message.contains("not a readable"),
            "{}",
            failure.message
        );
        // The file was left exactly as it was.
        assert_eq!(std::fs::read(profile.join("curio.db")).unwrap(), GARBAGE);
        let dto = failure.to_dto();
        assert_eq!(dto.backup, None);
        assert_eq!(dto.profile_dir, Some(profile.display().to_string()));
    }

    #[test]
    fn a_too_new_database_is_a_structured_too_new_failure() {
        let dir = tempfile::tempdir().unwrap();
        drop(CoreHandle::open(dir.path()).unwrap());
        let conn = rusqlite::Connection::open(dir.path().join("curio.db")).unwrap();
        conn.pragma_update(None, "user_version", Storage::schema_version() + 1)
            .unwrap();
        drop(conn);
        let failure = boot(Ok(dir.path().to_path_buf())).err().unwrap();
        assert_eq!(failure.class, StartupFailureClass::TooNew);
        assert!(failure.message.contains("newer"), "{}", failure.message);
    }

    #[test]
    fn no_home_directory_is_a_no_profile_failure() {
        let error = CommandError::internal(ErrorCode::Internal, "no home");
        let failure = boot(Err(error)).err().unwrap();
        assert_eq!(failure.class, StartupFailureClass::NoProfile);
        assert_eq!(failure.profile, None);
        let gate = StartupGate::default();
        gate.record(failure).unwrap();
        assert!(gate.reveal_target().is_err());
        assert!(recover(&gate, RecoveryAction::Retry).is_err());
    }

    #[test]
    fn open_errors_sort_into_classes() {
        let class = |open| StartupFailureClass::of(&CoreError::Open(open));
        assert_eq!(
            class(OpenError::TooNew {
                found: 9,
                supported: 1
            }),
            StartupFailureClass::TooNew
        );
        assert_eq!(
            class(OpenError::Backup {
                path: "x".into(),
                detail: "disk full".into()
            }),
            StartupFailureClass::Upgrade
        );
        assert_eq!(
            class(OpenError::Storage(
                curio_core::storage::StorageError::WriterGone
            )),
            StartupFailureClass::Other
        );
        let io = CoreError::Io {
            path: "events".into(),
            source: std::io::Error::other("read-only"),
        };
        assert_eq!(StartupFailureClass::of(&io), StartupFailureClass::Io);
        assert_eq!(
            StartupFailureClass::of(&CoreError::NotFound { entity: "feed" }),
            StartupFailureClass::Other
        );
        assert!(!StartupFailureClass::Locked.may_replace_database());
        assert!(StartupFailureClass::Corrupt.may_replace_database());
    }

    #[test]
    fn a_healthy_start_refuses_every_recovery_action() {
        let gate = StartupGate::default();
        assert_eq!(gate.current().unwrap(), None);
        assert!(gate.reveal_target().is_err());
        for action in [
            RecoveryAction::Retry,
            RecoveryAction::RestoreBackup,
            RecoveryAction::StartFresh,
        ] {
            let error = recover(&gate, action).unwrap_err();
            assert!(error.message.contains("nothing to recover"));
        }
    }

    #[test]
    fn start_fresh_moves_the_database_aside_and_opens_an_empty_one() {
        let (_dir, profile) = garbage_profile();
        let gate = gate_for(&profile);
        // A sidecar left behind travels with the database.
        std::fs::write(profile.join("curio.db-wal"), b"wal bytes").unwrap();
        assert_eq!(
            gate.reveal_target().unwrap(),
            profile.join("curio.db"),
            "reveal points at the database file"
        );

        let recovery = recover(&gate, RecoveryAction::StartFresh).unwrap();
        let core = recovery.core.unwrap();
        assert_eq!(core.list_feeds().unwrap().len(), 0, "a fresh database");
        assert_eq!(recovery.outcome.failure, None);
        assert_eq!(gate.current().unwrap(), None, "the gate is cleared");

        let aside = PathBuf::from(recovery.outcome.set_aside.unwrap());
        assert!(aside.starts_with(profile.join(SET_ASIDE_DIR)));
        assert_eq!(std::fs::read(aside.join("curio.db")).unwrap(), GARBAGE);
        assert_eq!(
            std::fs::read(aside.join("curio.db-wal")).unwrap(),
            b"wal bytes"
        );
    }

    #[test]
    fn restore_brings_back_the_backup_and_keeps_both_copies() {
        let (_dir, profile) = garbage_profile();
        let backup = write_backup_with_a_feed(&profile);
        let gate = gate_for(&profile);
        let shown = gate.current().unwrap().unwrap();
        assert_eq!(shown.class, StartupFailureClass::Corrupt);
        assert_eq!(shown.backup, Some(backup.display().to_string()));

        let recovery = recover(&gate, RecoveryAction::RestoreBackup).unwrap();
        let feeds = recovery.core.unwrap().list_feeds().unwrap();
        assert_eq!(feeds.len(), 1);
        assert_eq!(feeds[0].title.as_deref(), Some("Kept"));
        assert!(backup.exists(), "the backup is copied, not moved");
        assert!(!profile.join("curio.db.restoring").exists());
        let aside = PathBuf::from(recovery.outcome.set_aside.unwrap());
        assert_eq!(std::fs::read(aside.join("curio.db")).unwrap(), GARBAGE);
    }

    #[test]
    fn a_too_new_database_restores_the_backup_its_upgrade_wrote() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().to_path_buf();
        drop(CoreHandle::open(&profile).unwrap());
        // A newer build upgraded the file, backing up the old one first.
        let backup = write_backup_with_a_feed(&profile);
        let newer = Storage::schema_version() + 1;
        std::fs::rename(&backup, profile.join(format!("curio.db.pre-v{newer}.bak"))).unwrap();
        let conn = rusqlite::Connection::open(profile.join("curio.db")).unwrap();
        conn.pragma_update(None, "user_version", newer).unwrap();
        drop(conn);

        let gate = gate_for(&profile);
        assert!(gate.current().unwrap().unwrap().backup.is_some());
        let recovery = recover(&gate, RecoveryAction::RestoreBackup).unwrap();
        assert_eq!(recovery.core.unwrap().list_feeds().unwrap().len(), 1);
    }

    #[test]
    fn restore_without_a_backup_is_refused_and_moves_nothing() {
        let (_dir, profile) = garbage_profile();
        let gate = gate_for(&profile);
        let error = recover(&gate, RecoveryAction::RestoreBackup).unwrap_err();
        assert_eq!(error.code, ErrorCode::NotFound);
        assert_eq!(std::fs::read(profile.join("curio.db")).unwrap(), GARBAGE);
        assert!(!profile.join(SET_ASIDE_DIR).exists());
    }

    #[test]
    fn a_retry_that_still_fails_reports_the_new_failure() {
        let (_dir, profile) = garbage_profile();
        let gate = gate_for(&profile);
        let recovery = recover(&gate, RecoveryAction::Retry).unwrap();
        assert!(recovery.core.is_none());
        let failure = recovery.outcome.failure.unwrap();
        assert_eq!(failure.class, StartupFailureClass::Corrupt);
        assert_eq!(recovery.outcome.set_aside, None);
        assert_eq!(gate.current().unwrap(), Some(failure));
    }

    #[test]
    fn a_locked_database_is_never_moved_aside() {
        let (_dir, profile) = garbage_profile();
        let gate = StartupGate::default();
        gate.record(StartupFailure {
            class: StartupFailureClass::Locked,
            message: "locked".into(),
            profile: Some(profile.clone()),
        })
        .unwrap();
        assert_eq!(gate.current().unwrap().unwrap().backup, None);
        for action in [RecoveryAction::RestoreBackup, RecoveryAction::StartFresh] {
            let error = recover(&gate, action).unwrap_err();
            assert!(error.recoverable);
        }
        assert_eq!(std::fs::read(profile.join("curio.db")).unwrap(), GARBAGE);
    }

    #[test]
    fn set_aside_folders_never_collide() {
        let dir = tempfile::tempdir().unwrap();
        let first = claim_set_aside_dir(dir.path()).unwrap();
        let second = claim_set_aside_dir(dir.path()).unwrap();
        assert_ne!(first, second);
        assert!(first.is_dir() && second.is_dir());
        // Nothing to move is not an error, and leaves no folder behind.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(set_aside(empty.path()).unwrap(), None);
        assert!(!empty.path().join(SET_ASIDE_DIR).exists());
    }

    #[test]
    fn the_dto_serializes_snake_case_classes() {
        let dto = StartupFailureDto {
            class: StartupFailureClass::TooNew,
            message: "newer".into(),
            profile_dir: None,
            backup: None,
        };
        let json = serde_json::to_value(&dto).unwrap();
        assert_eq!(json["class"], "too_new");
    }
}
