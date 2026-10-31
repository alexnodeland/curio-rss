//! Platform path resolution (config/data/cache split — the DB lives in
//! the platform *data* dir, never `~/.config`; caches live in the
//! platform *cache* dir so the OS may reclaim them).
//!
//! The image-cache location must stay in lockstep with the `asset:`
//! protocol scope in `tauri.conf.json` (`$CACHE/curio/images/**`) — the
//! webview may only read what that scope names.

use std::path::{Path, PathBuf};

use curio_core::paths::LegacyProfile;

use crate::dto::LegacyProfileDto;
use crate::error::{CommandError, ErrorCode};

/// The profile directory holding `curio.db` + the events log: the
/// default shared with the CLI ([`curio_core::paths::default_profile_dir`]).
/// `CURIO_PROFILE` overrides for development and tests.
///
/// # Errors
///
/// Internal error when the platform reports no home directory.
pub fn profile_dir() -> Result<ProfileChoice, CommandError> {
    profile_dir_from(std::env::var("CURIO_PROFILE").ok())
}

/// The profile the app opens, and whether it is the shared default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileChoice {
    /// The profile directory.
    pub dir: PathBuf,
    /// `true` unless `CURIO_PROFILE` chose it — only the default profile
    /// adopts a legacy one ([`curio_core::paths::adopt_legacy_profiles`]).
    pub is_default: bool,
}

/// [`profile_dir`] over an explicit `CURIO_PROFILE` value (blank = unset).
fn profile_dir_from(overridden: Option<String>) -> Result<ProfileChoice, CommandError> {
    if let Some(dir) = overridden.filter(|dir| !dir.trim().is_empty()) {
        return Ok(ProfileChoice {
            dir: PathBuf::from(dir),
            is_default: false,
        });
    }
    curio_core::paths::default_profile_dir()
        .map(|dir| ProfileChoice {
            dir,
            is_default: true,
        })
        .map_err(|_| no_home())
}

/// The profile this session opens, after the one-time legacy migration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupProfile {
    /// The directory to open: the chosen profile, or — when a fresh
    /// default would strand a legacy library — that legacy library, in
    /// place, for this session only.
    pub dir: PathBuf,
    /// What became of each legacy profile (empty for `CURIO_PROFILE`).
    pub outcomes: Vec<LegacyProfile>,
    /// What the startup notice and the doctor panel show (managed state,
    /// served by `legacy_profiles`).
    pub legacy: LegacyProfiles,
}

/// The legacy-profile outcomes of this session's startup.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LegacyProfiles(pub Vec<LegacyProfileDto>);

/// Resolves what to open at startup. Only the default profile adopts a
/// legacy one (`legacy`: [`curio_core::paths::legacy_profile_dirs`]); if
/// one is left behind while the default holds no library, the app opens it
/// where it is rather than creating an empty default over it — preferring
/// `own_legacy`, the desktop's own earlier location
/// ([`curio_core::paths::legacy_desktop_profile_dir`]), which is what the
/// reader last saw. The next start tries the move again.
#[must_use]
pub fn startup_profile(
    choice: ProfileChoice,
    legacy: &[PathBuf],
    own_legacy: Option<&Path>,
) -> StartupProfile {
    if !choice.is_default {
        return StartupProfile {
            dir: choice.dir,
            outcomes: Vec::new(),
            legacy: LegacyProfiles::default(),
        };
    }
    let outcomes = curio_core::paths::adopt_legacy_profiles(&choice.dir, legacy);
    let in_place = curio_core::paths::in_place_fallback(&outcomes, own_legacy);
    let dtos = outcomes
        .iter()
        .map(|outcome| LegacyProfileDto::new(outcome, &choice.dir, in_place.as_deref()))
        .collect();
    StartupProfile {
        dir: in_place.unwrap_or(choice.dir),
        outcomes,
        legacy: LegacyProfiles(dtos),
    }
}

/// Where [`crate::image_cache::ImageCache`] stores fetched images:
/// `<os-cache>/curio/images` — exactly the `asset:` scope.
///
/// # Errors
///
/// Internal error when the platform reports no home directory.
pub fn image_cache_dir() -> Result<PathBuf, CommandError> {
    os_cache_dir().map(|cache| cache.join("curio").join("images"))
}

/// Where diagnostics bundles are staged: `<os-cache>/curio/diagnostics`.
///
/// # Errors
///
/// Internal error when the platform reports no home directory.
pub fn diagnostics_dir() -> Result<PathBuf, CommandError> {
    os_cache_dir().map(|cache| cache.join("curio").join("diagnostics"))
}

/// The user-level OS cache dir (`$CACHE` in Tauri scope terms) — not the
/// app-scoped one, to match the asset-protocol scope variable.
fn os_cache_dir() -> Result<PathBuf, CommandError> {
    directories::BaseDirs::new()
        .map(|dirs| dirs.cache_dir().to_path_buf())
        .ok_or_else(no_home)
}

fn no_home() -> CommandError {
    CommandError::internal(
        ErrorCode::Internal,
        "the platform reports no home directory — cannot resolve profile/cache paths",
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn the_default_profile_is_the_one_the_cli_opens() {
        let choice = profile_dir_from(None).unwrap();
        assert!(choice.is_default);
        assert_eq!(
            choice.dir,
            curio_core::paths::default_profile_dir().unwrap()
        );
        // A blank override is no override.
        assert_eq!(profile_dir_from(Some("  ".into())).unwrap(), choice);
    }

    #[test]
    fn curio_profile_overrides_the_default() {
        let choice = profile_dir_from(Some("/tmp/p".into())).unwrap();
        assert_eq!(
            choice,
            ProfileChoice {
                dir: PathBuf::from("/tmp/p"),
                is_default: false
            }
        );
    }

    fn default_choice(dir: &Path) -> ProfileChoice {
        ProfileChoice {
            dir: dir.to_path_buf(),
            is_default: true,
        }
    }

    /// A real profile (opened once through the core) at `dir`.
    fn make_profile(dir: &Path) {
        drop(curio_core::CoreHandle::open(dir).unwrap());
        assert!(curio_core::paths::is_profile(dir));
    }

    #[test]
    fn the_default_adopts_a_lone_legacy_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        let default = tmp.path().join("shared");
        let startup = startup_profile(
            default_choice(&default),
            std::slice::from_ref(&legacy),
            None,
        );
        assert_eq!(startup.dir, default);
        assert!(curio_core::paths::is_profile(&default));
        assert!(!legacy.exists());
        let [notice] = startup.legacy.0.as_slice() else {
            panic!("one notice expected: {:?}", startup.legacy);
        };
        assert_eq!(notice.status, crate::dto::LegacyProfileStatusDto::Moved);
        assert!(!notice.opened_in_place);
    }

    #[test]
    fn curio_profile_never_adopts() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        let explicit = ProfileChoice {
            dir: tmp.path().join("dev"),
            is_default: false,
        };
        let startup = startup_profile(explicit.clone(), std::slice::from_ref(&legacy), None);
        assert_eq!(startup.dir, explicit.dir);
        assert_eq!(startup.outcomes, Vec::new());
        assert_eq!(startup.legacy, LegacyProfiles::default());
        assert!(curio_core::paths::is_profile(&legacy), "left alone");
    }

    #[test]
    fn a_library_that_cannot_move_opens_in_place_instead_of_a_fresh_default() {
        let tmp = tempfile::tempdir().unwrap();
        let cli = tmp.path().join("old-cli");
        let desktop = tmp.path().join("old-desktop");
        make_profile(&cli);
        make_profile(&desktop);
        let default = tmp.path().join("shared");
        let legacy = [cli.clone(), desktop.clone()];

        // Two old libraries: the desktop reopens its own, where it is.
        let startup = startup_profile(default_choice(&default), &legacy, Some(&desktop));
        assert_eq!(startup.dir, desktop);
        assert!(!default.exists(), "no empty default created over them");
        assert_eq!(startup.legacy.0.len(), 2);
        let in_place: Vec<_> = startup
            .legacy
            .0
            .iter()
            .filter(|dto| dto.opened_in_place)
            .map(|dto| dto.path.clone())
            .collect();
        assert_eq!(in_place, vec![desktop.display().to_string()]);
        assert!(
            startup
                .legacy
                .0
                .iter()
                .all(|dto| dto.status == crate::dto::LegacyProfileStatusDto::Ambiguous)
        );

        // With an existing default nothing is stranded: the default opens.
        make_profile(&default);
        let startup = startup_profile(default_choice(&default), &legacy, Some(&desktop));
        assert_eq!(startup.dir, default);
        assert!(startup.legacy.0.iter().all(|dto| !dto.opened_in_place));
    }

    #[test]
    fn cache_paths_live_under_the_asset_scope_root() {
        let images = image_cache_dir().unwrap();
        assert!(images.ends_with("curio/images"));
        let diag = diagnostics_dir().unwrap();
        assert!(diag.ends_with("curio/diagnostics"));
    }
}
