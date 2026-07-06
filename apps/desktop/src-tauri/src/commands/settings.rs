//! Settings K/V + app info.
//!
//! UI preferences ride the same store under namespaced keys (`ui.theme`,
//! `ui.typography`, …) — no separate prefs commands. The facade-reserved
//! `"destinations"` key is refused in both directions: that registry is
//! owned by `add_destination`/`remove_destination`.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::storage::Storage;
use tauri::State;

use super::{SharedCore, run_blocking};
use crate::dto::AppInfoDto;
use crate::error::CommandError;

/// The settings key owned by the core facade (its destination registry).
const RESERVED_KEY: &str = "destinations";

/// Reads a setting.
#[tauri::command]
#[specta::specta]
pub async fn get_setting(
    core: State<'_, SharedCore>,
    key: String,
) -> Result<Option<String>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_setting_impl(&core, &key)).await
}

/// Writes a setting.
#[tauri::command]
#[specta::specta]
pub async fn set_setting(
    core: State<'_, SharedCore>,
    key: String,
    value: String,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_setting_impl(&core, &key, &value)).await
}

/// Version / profile / schema facts for the about box and doctor panel.
#[tauri::command]
#[specta::specta]
pub async fn get_app_info(core: State<'_, SharedCore>) -> Result<AppInfoDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || get_app_info_impl(&core)).await
}

// ------------------------------------------------------------------ impls

fn reject_reserved(key: &str) -> Result<(), CommandError> {
    if key == RESERVED_KEY {
        return Err(CommandError::invalid_input(
            "the \"destinations\" settings key is facade-owned — use the destination commands",
        ));
    }
    Ok(())
}

fn get_setting_impl(core: &CoreHandle, key: &str) -> Result<Option<String>, CommandError> {
    reject_reserved(key)?;
    Ok(core.storage().get_setting(key)?)
}

fn set_setting_impl(core: &CoreHandle, key: &str, value: &str) -> Result<(), CommandError> {
    reject_reserved(key)?;
    Ok(core.storage().set_setting(key, value)?)
}

fn get_app_info_impl(core: &CoreHandle) -> Result<AppInfoDto, CommandError> {
    Ok(AppInfoDto {
        version: CoreHandle::version().to_owned(),
        profile_dir: core.profile_dir().display().to_string(),
        db_schema_version: core.storage().db_schema_version()?,
        schema_supported: Storage::schema_version(),
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::temp_core;
    use crate::error::{ErrorCode, ErrorKind};

    #[test]
    fn settings_round_trip_under_namespaced_keys() {
        let (_dir, core) = temp_core();
        assert!(get_setting_impl(&core, "ui.theme").unwrap().is_none());
        set_setting_impl(&core, "ui.theme", "nord").unwrap();
        assert_eq!(
            get_setting_impl(&core, "ui.theme").unwrap().as_deref(),
            Some("nord")
        );
        set_setting_impl(&core, "ui.theme", "dark").unwrap();
        assert_eq!(
            get_setting_impl(&core, "ui.theme").unwrap().as_deref(),
            Some("dark")
        );
    }

    #[test]
    fn the_destinations_key_is_refused_both_ways() {
        let (_dir, core) = temp_core();
        for result in [
            set_setting_impl(&core, "destinations", "{}").unwrap_err(),
            get_setting_impl(&core, "destinations").unwrap_err(),
        ] {
            assert_eq!(result.kind, ErrorKind::User);
            assert_eq!(result.code, ErrorCode::InvalidInput);
        }
    }

    #[test]
    fn app_info_reports_matching_schema_versions_on_a_fresh_profile() {
        let (_dir, core) = temp_core();
        let info = get_app_info_impl(&core).unwrap();
        assert_eq!(info.version, CoreHandle::version());
        assert_eq!(info.db_schema_version, info.schema_supported);
        assert!(info.profile_dir.ends_with("profile"));
    }
}
