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
//! where it is.

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

/// The platform reports no home directory, so no default profile exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the platform reports no home directory — cannot resolve the default profile")]
pub struct NoHomeDir;

/// Whether `dir` holds a profile (its database exists).
#[must_use]
pub fn is_profile(dir: &Path) -> bool {
    dir.join(DB_FILE).is_file()
}

/// What [`adopt_legacy_profiles`] did with one legacy profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LegacyProfile {
    /// Moved into the shared default location; nothing left to do.
    Migrated {
        /// Where it was.
        from: PathBuf,
        /// Where it is now (the default profile).
        to: PathBuf,
    },
    /// Left where it is — the user reconciles it by hand.
    Left {
        /// The legacy profile's directory.
        path: PathBuf,
        /// Why it was not moved.
        reason: LeftReason,
    },
}

/// Why a legacy profile was not moved into the default location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeftReason {
    /// The default location already holds a profile; two libraries are
    /// never merged automatically.
    DefaultInUse,
    /// More than one legacy profile exists; which one wins is the user's
    /// call.
    Ambiguous,
    /// Its `SQLite` write-ahead log is present, so another (older) curio
    /// process may have it open — or it was not shut down cleanly.
    MaybeOpen,
    /// The default directory exists with other files in it.
    DefaultNotEmpty,
    /// The move itself failed (e.g. the two paths are on different
    /// volumes).
    MoveFailed(String),
}

impl std::fmt::Display for LegacyProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Migrated { from, to } => write!(
                f,
                "moved the profile at {} to the shared default {}",
                from.display(),
                to.display()
            ),
            Self::Left { path, reason } => {
                write!(
                    f,
                    "a profile from an earlier curio release is still at {} and is not in use: ",
                    path.display()
                )?;
                match reason {
                    LeftReason::DefaultInUse => f.write_str(
                        "the shared default already holds a library (open the old one with \
                         --profile to export what you need, then remove it)",
                    ),
                    LeftReason::Ambiguous => f.write_str(
                        "more than one old profile exists (move the one to keep into the \
                         shared default by hand)",
                    ),
                    LeftReason::MaybeOpen => f.write_str(
                        "it may be open in another curio process (quit it and run curio again \
                         to move it)",
                    ),
                    LeftReason::DefaultNotEmpty => f.write_str(
                        "the shared default directory exists and is not empty (move the old \
                         profile in by hand)",
                    ),
                    LeftReason::MoveFailed(error) => write!(
                        f,
                        "moving it failed ({error}); move it into the shared default by hand"
                    ),
                }
            }
        }
    }
}

/// The one-time migration onto the shared default profile.
///
/// Considers every directory in `legacy` that holds a profile. When
/// `default` holds none and exactly one legacy profile exists (and it does
/// not look open), that profile is renamed into `default` — a same-volume
/// rename, so the library moves whole or not at all. Every other case
/// leaves the legacy profile untouched and says why. An empty result
/// means there was nothing to do.
///
/// Called by a head only when it is about to open the *default* profile
/// (never for an explicit `--profile` / `CURIO_PROFILE`).
#[must_use]
pub fn adopt_legacy_profiles(default: &Path, legacy: &[PathBuf]) -> Vec<LegacyProfile> {
    let found: Vec<&PathBuf> = legacy
        .iter()
        .filter(|dir| dir.as_path() != default && is_profile(dir))
        .collect();
    let left = |reason: LeftReason| {
        found
            .iter()
            .map(|path| LegacyProfile::Left {
                path: (*path).clone(),
                reason: reason.clone(),
            })
            .collect()
    };
    match found.as_slice() {
        [] => Vec::new(),
        _ if is_profile(default) => left(LeftReason::DefaultInUse),
        [only] => adopt_one(only, default).into_iter().collect(),
        _ => left(LeftReason::Ambiguous),
    }
}

/// Moves the lone legacy profile `from` into the empty-or-absent `to`.
fn adopt_one(from: &Path, to: &Path) -> Option<LegacyProfile> {
    let left = |reason| {
        Some(LegacyProfile::Left {
            path: from.to_path_buf(),
            reason,
        })
    };
    if from.join(format!("{DB_FILE}-wal")).exists() {
        return left(LeftReason::MaybeOpen);
    }
    match std::fs::read_dir(to) {
        Ok(mut entries) => {
            if entries.next().is_some() {
                return left(LeftReason::DefaultNotEmpty);
            }
            // An empty directory: clear it so the rename can take its place
            // (Windows refuses to rename onto an existing directory).
            if let Err(error) = std::fs::remove_dir(to) {
                return left(LeftReason::MoveFailed(error.to_string()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return left(LeftReason::MoveFailed(error.to_string())),
    }
    if let Some(parent) = to.parent()
        && let Err(error) = std::fs::create_dir_all(parent)
    {
        return left(LeftReason::MoveFailed(error.to_string()));
    }
    match std::fs::rename(from, to) {
        Ok(()) => Some(LegacyProfile::Migrated {
            from: from.to_path_buf(),
            to: to.to_path_buf(),
        }),
        // Another head starting at the same moment won the race: the
        // library already sits at the default, nothing to report.
        Err(_) if !from.exists() && is_profile(to) => None,
        Err(error) => left(LeftReason::MoveFailed(error.to_string())),
    }
}

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

    fn make_profile(dir: &Path) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(DB_FILE), b"db").unwrap();
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
    fn nothing_to_adopt_without_legacy_profiles() {
        let tmp = tempfile::tempdir().unwrap();
        let default = tmp.path().join("new");
        let empty_legacy = tmp.path().join("old");
        std::fs::create_dir_all(&empty_legacy).unwrap();
        assert_eq!(
            adopt_legacy_profiles(&default, &[empty_legacy, tmp.path().join("gone")]),
            Vec::<LegacyProfile>::new()
        );
        assert!(!default.exists());
    }

    #[test]
    fn a_lone_legacy_profile_moves_into_an_absent_default() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        std::fs::write(legacy.join("curio.toml"), b"[settings]\n").unwrap();
        let default = tmp.path().join("deep").join("new");

        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert_eq!(
            outcome,
            vec![LegacyProfile::Migrated {
                from: legacy.clone(),
                to: default.clone()
            }]
        );
        assert!(is_profile(&default));
        assert!(default.join("curio.toml").is_file());
        assert!(!legacy.exists());
        assert!(outcome[0].to_string().contains("moved the profile"));
        // Idempotent: the second start has nothing left to do.
        assert_eq!(
            adopt_legacy_profiles(&default, &[legacy]),
            Vec::<LegacyProfile>::new()
        );
    }

    #[test]
    fn a_lone_legacy_profile_replaces_an_empty_default_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        let default = tmp.path().join("new");
        std::fs::create_dir_all(&default).unwrap();
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert!(matches!(
            outcome.as_slice(),
            [LegacyProfile::Migrated { .. }]
        ));
        assert!(is_profile(&default));
    }

    fn assert_left(outcome: &[LegacyProfile], path: &Path, want: &LeftReason) {
        assert!(
            outcome.iter().any(|o| matches!(
                o,
                LegacyProfile::Left { path: p, reason } if p == path && reason == want
            )),
            "expected {path:?} left for {want:?}, got {outcome:?}"
        );
        assert!(outcome.iter().all(|o| o.to_string().contains("not in use")));
    }

    #[test]
    fn never_merges_into_an_existing_default_profile() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        let default = tmp.path().join("new");
        make_profile(&legacy);
        make_profile(&default);
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert_left(&outcome, &legacy, &LeftReason::DefaultInUse);
        assert!(is_profile(&legacy), "the legacy library is untouched");
        assert!(outcome[0].to_string().contains("--profile"));
    }

    #[test]
    fn two_legacy_profiles_are_left_for_the_user() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a");
        let b = tmp.path().join("b");
        make_profile(&a);
        make_profile(&b);
        let default = tmp.path().join("new");
        let outcome = adopt_legacy_profiles(&default, &[a.clone(), b.clone()]);
        assert_eq!(outcome.len(), 2);
        assert_left(&outcome, &a, &LeftReason::Ambiguous);
        assert_left(&outcome, &b, &LeftReason::Ambiguous);
        assert!(!default.exists());
    }

    #[test]
    fn a_legacy_profile_with_a_wal_may_be_open_and_stays() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        std::fs::write(legacy.join("curio.db-wal"), b"").unwrap();
        let default = tmp.path().join("new");
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert_left(&outcome, &legacy, &LeftReason::MaybeOpen);
        assert!(!default.exists());
    }

    #[test]
    fn a_non_empty_default_dir_blocks_the_move() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        let default = tmp.path().join("new");
        std::fs::create_dir_all(default.join("logs")).unwrap();
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert_left(&outcome, &legacy, &LeftReason::DefaultNotEmpty);
        assert!(is_profile(&legacy));
    }

    #[test]
    fn a_failed_move_is_reported_not_raised() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        make_profile(&legacy);
        // The default's parent is a regular file, so it cannot be created.
        let blocker = tmp.path().join("file");
        std::fs::write(&blocker, b"").unwrap();
        let default = blocker.join("new");
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert!(
            matches!(
                outcome.as_slice(),
                [LegacyProfile::Left {
                    reason: LeftReason::MoveFailed(_),
                    ..
                }]
            ),
            "{outcome:?}"
        );
        assert!(outcome[0].to_string().contains("moving it failed"));
        assert!(is_profile(&legacy));
    }

    #[test]
    fn every_left_reason_reads_as_a_sentence() {
        for reason in [
            LeftReason::DefaultInUse,
            LeftReason::Ambiguous,
            LeftReason::MaybeOpen,
            LeftReason::DefaultNotEmpty,
            LeftReason::MoveFailed("boom".into()),
        ] {
            let text = LegacyProfile::Left {
                path: PathBuf::from("/old"),
                reason,
            }
            .to_string();
            assert!(text.starts_with("a profile from an earlier curio release"));
            assert!(text.contains("/old"));
        }
    }

    #[test]
    fn no_home_dir_has_an_actionable_message() {
        assert!(NoHomeDir.to_string().contains("home directory"));
    }
}
