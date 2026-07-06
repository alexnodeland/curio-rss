//! Platform path resolution (config/data/cache split — the DB lives in
//! the platform *data* dir, never `~/.config`; caches live in the
//! platform *cache* dir so the OS may reclaim them).
//!
//! The image-cache location must stay in lockstep with the `asset:`
//! protocol scope in `tauri.conf.json` (`$CACHE/curio/images/**`) — the
//! webview may only read what that scope names.

use std::path::PathBuf;

use crate::error::{CommandError, ErrorCode};

/// Reverse-DNS pieces for `directories` (D12: owned identifier,
/// `io.github.alexnodeland.curio`).
const QUALIFIER: &str = "io.github";
const ORGANIZATION: &str = "alexnodeland";
const APPLICATION: &str = "curio";

/// The profile directory holding `curio.db` + the events log.
/// `CURIO_PROFILE` overrides for development and tests.
///
/// # Errors
///
/// Internal error when the platform reports no home directory.
pub fn profile_dir() -> Result<PathBuf, CommandError> {
    if let Ok(overridden) = std::env::var("CURIO_PROFILE")
        && !overridden.trim().is_empty()
    {
        return Ok(PathBuf::from(overridden));
    }
    directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
        .map(|dirs| dirs.data_dir().to_path_buf())
        .ok_or_else(no_home)
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
    fn cache_paths_live_under_the_asset_scope_root() {
        let images = image_cache_dir().unwrap();
        assert!(images.ends_with("curio/images"));
        let diag = diagnostics_dir().unwrap();
        assert!(diag.ends_with("curio/diagnostics"));
    }
}
