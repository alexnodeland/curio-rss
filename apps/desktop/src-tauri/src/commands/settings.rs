//! Settings K/V + app info.
//!
//! UI preferences ride the same store under namespaced keys (`ui.theme`,
//! `ui.typography`, …) — no separate prefs commands. The core facade
//! refuses its reserved `"destinations"` key in both directions (that
//! registry is owned by `add_destination`/`remove_destination`); the
//! refusal surfaces here as an `InvalidInput` user error.

use std::sync::{Arc, Mutex, PoisonError};

use curio_core::CoreHandle;
use curio_core::storage::Storage;
use tauri::{AppHandle, State};

use super::{SharedCore, run_blocking};
use crate::dto::AppInfoDto;
use crate::error::CommandError;
use crate::l10n::{LOCALE_KEY, Locale};

/// Reads a setting.
#[tauri::command]
#[specta::specta]
pub async fn get_setting(
    core: State<'_, SharedCore>,
    key: String,
) -> Result<Option<String>, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking("get_setting", move || get_setting_impl(&core, &key)).await
}

/// Writes a setting. A new `ui.locale` also rebuilds the native menu in
/// that language (the notification body reads it fresh each time).
#[tauri::command]
#[specta::specta]
pub async fn set_setting(
    app: AppHandle,
    core: State<'_, SharedCore>,
    key: String,
    value: String,
) -> Result<(), CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking("set_setting", move || {
        set_setting_and_menu(&core, &key, &value, |locale| {
            crate::menu::apply_locale(&app, locale);
        })
    })
    .await
}

/// Serialises locale writes with their menu rebuilds: two quick language
/// changes must leave the menu in the language that was saved last, not in
/// whichever rebuild happened to finish last.
static LOCALE_TURN: Mutex<()> = Mutex::new(());

/// Writes `key`; for the locale setting, then rebuilds the menu through
/// `apply_menu` — the write and its rebuild as one turn under
/// [`LOCALE_TURN`], so rebuilds land in the order their writes did.
fn set_setting_and_menu(
    core: &CoreHandle,
    key: &str,
    value: &str,
    apply_menu: impl FnOnce(Locale),
) -> Result<(), CommandError> {
    let Some(locale) = menu_locale_change(key, value) else {
        return set_setting_impl(core, key, value);
    };
    // Poisoning only means an earlier rebuild panicked; the guard protects
    // no data, so the turn is still safe to take.
    let _turn = LOCALE_TURN.lock().unwrap_or_else(PoisonError::into_inner);
    set_setting_impl(core, key, value)?;
    apply_menu(locale);
    Ok(())
}

/// The locale to rebuild the menu in, when `key` is the locale setting. An
/// unknown id falls back to English, as the frontend's catalogs do.
fn menu_locale_change(key: &str, value: &str) -> Option<Locale> {
    (key == LOCALE_KEY).then(|| Locale::parse(value).unwrap_or_default())
}

/// Version / profile / schema facts for the about box and doctor panel.
#[tauri::command]
#[specta::specta]
pub async fn get_app_info(core: State<'_, SharedCore>) -> Result<AppInfoDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking("get_app_info", move || get_app_info_impl(&core)).await
}

// ------------------------------------------------------------------ impls

fn get_setting_impl(core: &CoreHandle, key: &str) -> Result<Option<String>, CommandError> {
    Ok(core.get_setting(key)?)
}

fn set_setting_impl(core: &CoreHandle, key: &str, value: &str) -> Result<(), CommandError> {
    Ok(core.set_setting(key, value)?)
}

fn get_app_info_impl(core: &CoreHandle) -> Result<AppInfoDto, CommandError> {
    Ok(AppInfoDto {
        version: CoreHandle::version().to_owned(),
        profile_dir: core.profile_dir().display().to_string(),
        db_schema_version: core.db_schema_version()?,
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

    #[test]
    fn only_the_locale_setting_rebuilds_the_menu() {
        assert_eq!(menu_locale_change("ui.locale", "de"), Some(Locale::De));
        assert_eq!(menu_locale_change("ui.locale", "xx"), Some(Locale::En));
        assert_eq!(menu_locale_change("ui.theme", "de"), None);
    }

    #[test]
    fn a_locale_write_rebuilds_the_menu_after_it_is_saved() {
        let (_dir, core) = crate::commands::testutil::temp_core();
        let mut applied = Vec::new();
        set_setting_and_menu(&core, LOCALE_KEY, "fr", |locale| {
            // The rebuild runs inside the turn, after the write landed.
            assert!(LOCALE_TURN.try_lock().is_err());
            assert_eq!(
                get_setting_impl(&core, LOCALE_KEY).unwrap().as_deref(),
                Some("fr")
            );
            applied.push(locale);
        })
        .unwrap();
        assert_eq!(applied, vec![Locale::Fr]);
        // Any other key is a plain write: no rebuild.
        set_setting_and_menu(&core, "ui.theme", "dark", |_| panic!("rebuilt")).unwrap();
        assert_eq!(
            get_setting_impl(&core, "ui.theme").unwrap().as_deref(),
            Some("dark")
        );
    }
}
