//! Platform path resolution (config/data/cache split — the DB lives in
//! the platform *data* dir, never `~/.config`; caches live in the
//! platform *cache* dir so the OS may reclaim them).
//!
//! The image-cache location must stay in lockstep with the `asset:`
//! protocol scope in `tauri.conf.json` (`$CACHE/curio/images/**`) — the
//! webview may only read what that scope names.

use std::path::PathBuf;

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

    #[test]
    fn cache_paths_live_under_the_asset_scope_root() {
        let images = image_cache_dir().unwrap();
        assert!(images.ends_with("curio/images"));
        let diag = diagnostics_dir().unwrap();
        assert!(diag.ends_with("curio/diagnostics"));
    }
}
