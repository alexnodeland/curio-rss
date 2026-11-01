//! Where a profile lives by default — the one answer every head shares.
//!
//! The desktop app and the CLI open the **same** profile unless told
//! otherwise (`--profile` / `CURIO_PROFILE`), so `curio clip <url>` lands
//! in the reader's library and the agent/cron surface sees what the reader
//! sees. Both heads resolve the default here; neither builds its own path.
//!
//! The default is the platform's *local* (non-roaming) data directory
//! under the app's reverse-DNS identifier (`io.github.alexnodeland.curio`,
//! decision D12):
//!
//! | Platform | Default profile |
//! |----------|-----------------|
//! | macOS    | `~/Library/Application Support/io.github.alexnodeland.curio` |
//! | Linux    | `$XDG_DATA_HOME/curio` (default `~/.local/share/curio`) |
//! | Windows  | `%LOCALAPPDATA%\alexnodeland\curio\data` |
//!
//! Local (not Roaming) `AppData` on Windows: a large `SQLite`/WAL database
//! has no business in a roaming profile.
//!
//! Before 0.5 the heads disagreed (the CLI used a bare `curio` directory,
//! the Windows desktop used Roaming `AppData`). [`legacy_profile_dirs`]
//! names those old locations and [`adopt_legacy_profiles`] moves a lone
//! legacy profile into the shared default once, or reports why it left it
//! where it is. A head never creates a fresh default over a legacy library
//! it could not move ([`LegacyProfile::would_be_stranded`]): that would
//! strand the library for good, since a default that holds one is never
//! merged into.

use std::path::{Path, PathBuf};

/// The database file whose presence makes a directory a profile.
pub const DB_FILE: &str = "curio.db";

/// Reverse-DNS pieces of the app identifier (D12).
const QUALIFIER: &str = "io.github";
const ORGANIZATION: &str = "alexnodeland";
const APPLICATION: &str = "curio";

/// The platform families with distinct directory conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// macOS: everything under `~/Library/Application Support`.
    MacOs,
    /// Windows: Local vs Roaming `AppData`.
    Windows,
    /// Linux and the other XDG unixes.
    Xdg,
}

impl Platform {
    /// The platform this binary was built for.
    #[must_use]
    pub const fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Xdg
        }
    }
}

/// The OS base directories a profile path is derived from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformRoots {
    /// The user's local (machine-bound) data root: `~/Library/Application
    /// Support`, `$XDG_DATA_HOME`, or `%LOCALAPPDATA%`.
    pub local_data: PathBuf,
    /// The user's roaming data root: the same as `local_data` except on
    /// Windows (`%APPDATA%`).
    pub roaming_data: PathBuf,
}

impl PlatformRoots {
    /// The current user's roots, as the OS reports them; `None` when the
    /// platform reports no home directory.
    #[must_use]
    pub fn current() -> Option<Self> {
        directories::BaseDirs::new().map(|dirs| Self {
            local_data: dirs.data_local_dir().to_path_buf(),
            roaming_data: dirs.data_dir().to_path_buf(),
        })
    }
}

/// The default profile directory both heads open, on any platform.
#[must_use]
pub fn default_profile_dir_in(platform: Platform, roots: &PlatformRoots) -> PathBuf {
    match platform {
        Platform::MacOs => roots
            .local_data
            .join(format!("{QUALIFIER}.{ORGANIZATION}.{APPLICATION}")),
        Platform::Windows => roots
            .local_data
            .join(ORGANIZATION)
            .join(APPLICATION)
            .join("data"),
        Platform::Xdg => roots.local_data.join(APPLICATION),
    }
}

/// Where earlier releases kept a profile on this platform, excluding the
/// current default (on Linux both heads always agreed, so there are none).
#[must_use]
pub fn legacy_profile_dirs_in(platform: Platform, roots: &PlatformRoots) -> Vec<PathBuf> {
    let candidates = match platform {
        // The pre-0.5 CLI: `ProjectDirs::from("", "", "curio")`.
        Platform::MacOs => vec![roots.local_data.join(APPLICATION)],
        Platform::Windows => vec![
            // The pre-0.5 CLI.
            roots.roaming_data.join(APPLICATION).join("data"),
            // The pre-0.5 desktop, in Roaming `AppData`.
            roots
                .roaming_data
                .join(ORGANIZATION)
                .join(APPLICATION)
                .join("data"),
        ],
        Platform::Xdg => Vec::new(),
    };
    let default = default_profile_dir_in(platform, roots);
    candidates
        .into_iter()
        .filter(|dir| *dir != default)
        .collect()
}

/// Where the desktop app of an earlier release kept its profile, when that
/// differs from the shared default (only on Windows: Roaming `AppData`).
/// Always one of [`legacy_profile_dirs_in`].
#[must_use]
pub fn legacy_desktop_profile_dir_in(platform: Platform, roots: &PlatformRoots) -> Option<PathBuf> {
    match platform {
        Platform::Windows => Some(
            roots
                .roaming_data
                .join(ORGANIZATION)
                .join(APPLICATION)
                .join("data"),
        ),
        Platform::MacOs | Platform::Xdg => None,
    }
    .filter(|dir| *dir != default_profile_dir_in(platform, roots))
}

/// The default profile directory for the current user and platform.
///
/// # Errors
///
/// [`NoHomeDir`] when the platform reports no home directory.
pub fn default_profile_dir() -> Result<PathBuf, NoHomeDir> {
    PlatformRoots::current()
        .map(|roots| default_profile_dir_in(Platform::current(), &roots))
        .ok_or(NoHomeDir)
}

/// The legacy profile locations for the current user and platform (empty
/// when the platform reports no home directory).
#[must_use]
pub fn legacy_profile_dirs() -> Vec<PathBuf> {
    PlatformRoots::current()
        .map(|roots| legacy_profile_dirs_in(Platform::current(), &roots))
        .unwrap_or_default()
}

/// [`legacy_desktop_profile_dir_in`] for the current user and platform.
#[must_use]
pub fn legacy_desktop_profile_dir() -> Option<PathBuf> {
    PlatformRoots::current()
        .and_then(|roots| legacy_desktop_profile_dir_in(Platform::current(), &roots))
}

/// The platform reports no home directory, so no default profile exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the platform reports no home directory — cannot resolve the default profile")]
pub struct NoHomeDir;

/// Whether `dir` holds a profile (its database exists).
#[must_use]
pub fn is_profile(dir: &Path) -> bool {
    dir.join(DB_FILE).is_file()
}

mod legacy;

pub use legacy::{LeftReason, LegacyProfile, adopt_legacy_profiles, in_place_fallback};

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn roots(local: &str, roaming: &str) -> PlatformRoots {
        PlatformRoots {
            local_data: PathBuf::from(local),
            roaming_data: PathBuf::from(roaming),
        }
    }

    #[test]
    fn default_profile_dir_per_platform() {
        let mac = roots(
            "/Users/u/Library/Application Support",
            "/Users/u/Library/Application Support",
        );
        assert_eq!(
            default_profile_dir_in(Platform::MacOs, &mac),
            Path::new("/Users/u/Library/Application Support/io.github.alexnodeland.curio")
        );
        let linux = roots("/home/u/.local/share", "/home/u/.local/share");
        assert_eq!(
            default_profile_dir_in(Platform::Xdg, &linux),
            Path::new("/home/u/.local/share/curio")
        );
        let win = roots("C:/Users/u/AppData/Local", "C:/Users/u/AppData/Roaming");
        assert_eq!(
            default_profile_dir_in(Platform::Windows, &win),
            Path::new("C:/Users/u/AppData/Local")
                .join("alexnodeland")
                .join("curio")
                .join("data"),
            "Local AppData, never Roaming"
        );
    }

    #[test]
    fn default_matches_the_desktop_identifier_on_this_os() {
        // What the desktop head opened before the paths were unified (its
        // `ProjectDirs` identifier), with Local `AppData` on Windows: the
        // shared default must be exactly that on every OS CI runs on.
        let desktop = directories::ProjectDirs::from(QUALIFIER, ORGANIZATION, APPLICATION)
            .unwrap()
            .data_local_dir()
            .to_path_buf();
        assert_eq!(default_profile_dir().unwrap(), desktop);
    }

    #[test]
    fn legacy_dirs_name_the_old_cli_and_roaming_desktop_locations() {
        let mac = roots("/L", "/L");
        assert_eq!(
            legacy_profile_dirs_in(Platform::MacOs, &mac),
            vec![PathBuf::from("/L/curio")]
        );
        let win = roots("/Local", "/Roaming");
        assert_eq!(
            legacy_profile_dirs_in(Platform::Windows, &win),
            vec![
                Path::new("/Roaming").join("curio").join("data"),
                Path::new("/Roaming")
                    .join("alexnodeland")
                    .join("curio")
                    .join("data"),
            ]
        );
        assert_eq!(
            legacy_profile_dirs_in(Platform::Xdg, &roots("/s", "/s")),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn legacy_dirs_never_include_the_default() {
        for platform in [Platform::MacOs, Platform::Windows, Platform::Xdg] {
            let r = roots("/same", "/same");
            let default = default_profile_dir_in(platform, &r);
            assert!(!legacy_profile_dirs_in(platform, &r).contains(&default));
        }
        assert!(
            legacy_profile_dirs()
                .iter()
                .all(|dir| { default_profile_dir().map_or(true, |default| *dir != default) })
        );
    }

    #[test]
    fn current_platform_matches_cfg() {
        let expected = if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Xdg
        };
        assert_eq!(Platform::current(), expected);
    }

    #[test]
    fn only_windows_had_a_separate_desktop_location() {
        let win = roots("/Local", "/Roaming");
        let desktop = legacy_desktop_profile_dir_in(Platform::Windows, &win).unwrap();
        assert_eq!(
            desktop,
            Path::new("/Roaming")
                .join("alexnodeland")
                .join("curio")
                .join("data")
        );
        assert!(legacy_profile_dirs_in(Platform::Windows, &win).contains(&desktop));
        // Local == Roaming would make it the default itself: not legacy.
        assert_eq!(
            legacy_desktop_profile_dir_in(Platform::Windows, &roots("/same", "/same")),
            None
        );
        assert_eq!(
            legacy_desktop_profile_dir_in(Platform::MacOs, &roots("/L", "/L")),
            None
        );
        assert_eq!(
            legacy_desktop_profile_dir_in(Platform::Xdg, &roots("/s", "/s")),
            None
        );
        if Platform::current() != Platform::Windows {
            assert_eq!(legacy_desktop_profile_dir(), None);
        }
    }

    #[test]
    fn no_home_dir_has_an_actionable_message() {
        assert!(NoHomeDir.to_string().contains("home directory"));
    }
}
