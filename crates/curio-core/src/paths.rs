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
    /// Another process has its database open right now (`SQLite` kept its
    /// write-ahead log after a probe connection closed — it only deletes
    /// the log when the last connection goes). Transient: quitting that
    /// process lets the next start move it.
    InUse,
    /// The default directory exists with other files in it.
    DefaultNotEmpty,
    /// Its database could not be opened to check whether it is in use.
    Unreadable(String),
    /// The move itself failed (e.g. the two paths are on different
    /// volumes).
    MoveFailed(String),
}

impl LegacyProfile {
    /// Whether creating a fresh default profile now would strand this
    /// legacy library: it was left behind while the default holds none
    /// (every reason but [`LeftReason::DefaultInUse`]). Once a fresh
    /// default exists the library could never be moved in, so a head that
    /// sees this must not create one — the CLI refuses, the desktop opens
    /// the legacy library where it is ([`in_place_fallback`]).
    #[must_use]
    pub fn would_be_stranded(&self) -> bool {
        matches!(self, Self::Left { reason, .. } if *reason != LeftReason::DefaultInUse)
    }
}

/// The legacy library a head should open where it is, this session only,
/// instead of creating a fresh default that would strand it: `preferred`
/// (the head's own previous location) when it is among the stranded ones,
/// else the first stranded one. `None` when nothing would be stranded.
#[must_use]
pub fn in_place_fallback(outcomes: &[LegacyProfile], preferred: Option<&Path>) -> Option<PathBuf> {
    let stranded: Vec<&Path> = outcomes
        .iter()
        .filter(|outcome| outcome.would_be_stranded())
        .filter_map(|outcome| match outcome {
            LegacyProfile::Left { path, .. } => Some(path.as_path()),
            LegacyProfile::Migrated { .. } => None,
        })
        .collect();
    preferred
        .filter(|want| stranded.contains(want))
        .or_else(|| stranded.first().copied())
        .map(Path::to_path_buf)
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
                    "a profile from an earlier curio release was left at {}: ",
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
                    LeftReason::InUse => f.write_str(
                        "another curio process has it open (quit it, then start curio again \
                         to move it)",
                    ),
                    LeftReason::DefaultNotEmpty => f.write_str(
                        "the shared default directory exists and is not empty (move the old \
                         profile in by hand)",
                    ),
                    LeftReason::Unreadable(error) => write!(
                        f,
                        "its database could not be checked ({error}); move it into the shared \
                         default by hand"
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
/// `default` holds none and exactly one legacy profile exists (and no
/// process has it open), that profile is renamed into `default` — a
/// same-volume rename, so the library moves whole or not at all. Every
/// other case leaves the legacy profile untouched and says why. An empty
/// result means there was nothing to do.
///
/// A legacy profile whose `SQLite` write-ahead log is still on disk is
/// probed rather than skipped: a head that never closes its connections
/// (the desktop app exits without dropping its handle) leaves the log
/// behind, so its presence alone proves nothing. The probe opens the
/// database, checkpoints the log into it and closes again; `SQLite`
/// deletes the log only when the last connection closes, so a log that
/// survives the probe means another process still has the library open.
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
    match db_in_use(from) {
        Ok(false) => {}
        Ok(true) => return left(LeftReason::InUse),
        Err(error) => return left(LeftReason::Unreadable(error.to_string())),
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

/// Whether another process has the profile's database open (see
/// [`adopt_legacy_profiles`]). No write-ahead log means no open WAL-mode
/// connection — curio always runs its database in WAL mode.
fn db_in_use(dir: &Path) -> Result<bool, rusqlite::Error> {
    let wal = dir.join(format!("{DB_FILE}-wal"));
    if !wal.exists() {
        return Ok(false);
    }
    let conn = rusqlite::Connection::open_with_flags(
        dir.join(DB_FILE),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    // A read opens the log (replaying a stale one left by a process that
    // exited without closing); the checkpoint folds it into the database.
    // A busy checkpoint is no error here: the log check below answers.
    conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
    let _ = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    conn.close().map_err(|(_, error)| error)?;
    Ok(wal.exists())
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
        assert!(
            outcome
                .iter()
                .all(|o| o.to_string().contains("was left at"))
        );
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

    /// A real WAL-mode database at `dir` with one row, the connection
    /// still open (so the row sits in `curio.db-wal`, not yet
    /// checkpointed).
    fn open_wal_profile(dir: &Path) -> rusqlite::Connection {
        std::fs::create_dir_all(dir).unwrap();
        let conn = rusqlite::Connection::open(dir.join(DB_FILE)).unwrap();
        conn.pragma_update(None, "journal_mode", "WAL").unwrap();
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        conn.execute_batch("CREATE TABLE t (v TEXT); INSERT INTO t VALUES ('kept');")
            .unwrap();
        assert!(dir.join("curio.db-wal").is_file());
        conn
    }

    fn read_row(dir: &Path) -> String {
        rusqlite::Connection::open(dir.join(DB_FILE))
            .unwrap()
            .query_row("SELECT v FROM t", [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn a_legacy_profile_open_in_another_connection_stays_until_it_closes() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        let holder = open_wal_profile(&legacy);
        let default = tmp.path().join("new");

        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert_left(&outcome, &legacy, &LeftReason::InUse);
        assert!(outcome[0].would_be_stranded());
        assert!(!default.exists());
        assert!(outcome[0].to_string().contains("quit it"));

        // Once the other process lets go, the next start moves it.
        drop(holder);
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert!(
            matches!(outcome.as_slice(), [LegacyProfile::Migrated { .. }]),
            "{outcome:?}"
        );
        assert_eq!(read_row(&default), "kept");
    }

    #[test]
    fn a_stale_wal_with_no_live_process_does_not_block_the_move() {
        // What a head that exits without closing its connections leaves
        // behind: the database plus a write-ahead log holding the latest
        // writes, and no process attached to either.
        let tmp = tempfile::tempdir().unwrap();
        let live = tmp.path().join("live");
        let conn = open_wal_profile(&live);
        let legacy = tmp.path().join("old");
        std::fs::create_dir_all(&legacy).unwrap();
        for file in [DB_FILE, "curio.db-wal"] {
            std::fs::copy(live.join(file), legacy.join(file)).unwrap();
        }
        drop(conn);
        assert!(legacy.join("curio.db-wal").is_file());
        let default = tmp.path().join("new");

        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert_eq!(
            outcome,
            vec![LegacyProfile::Migrated {
                from: legacy.clone(),
                to: default.clone()
            }]
        );
        assert!(!default.join("curio.db-wal").exists(), "checkpointed away");
        assert_eq!(read_row(&default), "kept", "the log's writes survive");
    }

    #[test]
    fn an_unreadable_legacy_database_with_a_wal_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let legacy = tmp.path().join("old");
        // Not an SQLite file, so the probe cannot tell whether it is open.
        make_profile(&legacy);
        std::fs::write(legacy.join("curio.db-wal"), b"").unwrap();
        let default = tmp.path().join("new");
        let outcome = adopt_legacy_profiles(&default, std::slice::from_ref(&legacy));
        assert!(
            matches!(
                outcome.as_slice(),
                [LegacyProfile::Left {
                    reason: LeftReason::Unreadable(_),
                    ..
                }]
            ),
            "{outcome:?}"
        );
        assert!(outcome[0].to_string().contains("could not be checked"));
        assert!(is_profile(&legacy));
        assert!(!default.exists());
    }

    #[test]
    fn only_a_library_left_beside_an_empty_default_would_be_stranded() {
        let left = |reason| LegacyProfile::Left {
            path: PathBuf::from("/old"),
            reason,
        };
        assert!(!left(LeftReason::DefaultInUse).would_be_stranded());
        for reason in [
            LeftReason::Ambiguous,
            LeftReason::InUse,
            LeftReason::DefaultNotEmpty,
            LeftReason::Unreadable("e".into()),
            LeftReason::MoveFailed("e".into()),
        ] {
            assert!(left(reason).would_be_stranded());
        }
        let moved = LegacyProfile::Migrated {
            from: PathBuf::from("/old"),
            to: PathBuf::from("/new"),
        };
        assert!(!moved.would_be_stranded());
    }

    #[test]
    fn the_in_place_fallback_prefers_the_heads_own_old_location() {
        let left = |path: &str, reason| LegacyProfile::Left {
            path: PathBuf::from(path),
            reason,
        };
        let ambiguous = [
            left("/cli", LeftReason::Ambiguous),
            left("/desktop", LeftReason::Ambiguous),
        ];
        assert_eq!(
            in_place_fallback(&ambiguous, Some(Path::new("/desktop"))),
            Some(PathBuf::from("/desktop"))
        );
        assert_eq!(
            in_place_fallback(&ambiguous, None),
            Some(PathBuf::from("/cli"))
        );
        assert_eq!(
            in_place_fallback(&ambiguous, Some(Path::new("/elsewhere"))),
            Some(PathBuf::from("/cli"))
        );
        // Nothing stranded: the default (in use, or freshly moved) opens.
        assert_eq!(
            in_place_fallback(
                &[left("/desktop", LeftReason::DefaultInUse)],
                Some(Path::new("/desktop"))
            ),
            None
        );
        let moved = LegacyProfile::Migrated {
            from: PathBuf::from("/cli"),
            to: PathBuf::from("/new"),
        };
        assert_eq!(in_place_fallback(&[moved], None), None);
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
            LeftReason::InUse,
            LeftReason::DefaultNotEmpty,
            LeftReason::Unreadable("boom".into()),
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
