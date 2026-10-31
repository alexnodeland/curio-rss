//! The network-filesystem guard.
//!
//! `SQLite` in WAL mode relies on POSIX advisory locks and a shared-memory
//! index (`curio.db-shm`) that only work when every process sees the same
//! local file. On a network mount (NFS, SMB, AFP, `WebDAV`) locks are
//! emulated or silently ignored; in a sync-managed folder (iCloud Drive,
//! Dropbox, Syncthing, the macOS File Provider folders under
//! `~/Library/CloudStorage`) a sync client copies `curio.db` without its
//! `-wal` and `-shm` sidecars, or replaces it under a live connection.
//! Either way the database can be corrupted. So the profile open refuses
//! such locations ([`super::OpenError::NetworkFs`]) unless the user opts in
//! explicitly ([`ALLOW_NETWORK_FS_ENV`], or the CLI's `--allow-network-fs`).
//!
//! Detection is best effort and errs towards "local":
//!
//! - the filesystem type — `statfs(2)` `f_type` on Linux, `f_fstypename`
//!   on macOS — for network mounts;
//! - a Windows UNC path (`\\server\share\…`) for network shares (a mapped
//!   drive letter is not recognised);
//! - the folder markers sync clients leave in their root — `.stfolder`
//!   (Syncthing), a `.dropbox` *file* (Dropbox; `~/.dropbox` is the
//!   client's config directory, not a synced root);
//! - the well-known sync locations `~/Library/Mobile Documents` (iCloud
//!   Drive), `~/Library/CloudStorage` (File Provider sync clients) and
//!   `iCloudDrive` (iCloud for Windows).

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf, Prefix};

/// The environment variable both heads read as the explicit override:
/// `1`, `true`, `yes` or `on` (any case) opens a profile on network or
/// sync-managed storage anyway.
pub const ALLOW_NETWORK_FS_ENV: &str = "CURIO_ALLOW_NETWORK_FS";

/// What kind of shared storage a profile path sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum NetworkFsKind {
    /// An NFS mount.
    Nfs,
    /// An SMB / CIFS share.
    Smb,
    /// An AFP (Apple Filing Protocol) share.
    Afp,
    /// A `WebDAV` mount.
    WebDav,
    /// A distributed network filesystem (AFS, Coda, Ceph, Lustre).
    Distributed,
    /// A Windows network share addressed by UNC path.
    WindowsShare,
    /// iCloud Drive (`~/Library/Mobile Documents`, or `iCloudDrive` on
    /// Windows).
    ICloudDrive,
    /// A macOS File Provider sync folder (`~/Library/CloudStorage`:
    /// Dropbox, `OneDrive`, Google Drive, Box…).
    CloudStorage,
    /// A Dropbox folder (its root holds a `.dropbox` file).
    Dropbox,
    /// A Syncthing folder (its root holds `.stfolder`).
    Syncthing,
}

impl fmt::Display for NetworkFsKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Nfs => "an NFS mount",
            Self::Smb => "an SMB/CIFS network share",
            Self::Afp => "an AFP network share",
            Self::WebDav => "a WebDAV mount",
            Self::Distributed => "a distributed network filesystem",
            Self::WindowsShare => "a Windows network share",
            Self::ICloudDrive => "iCloud Drive",
            Self::CloudStorage => "a cloud-sync folder (~/Library/CloudStorage)",
            Self::Dropbox => "a Dropbox folder",
            Self::Syncthing => "a Syncthing folder",
        })
    }
}

/// Whether the [`ALLOW_NETWORK_FS_ENV`] override is set to a true value
/// in this process's environment. Heads fold it into
/// [`crate::CoreOptions::allow_network_fs`].
#[must_use]
pub fn network_fs_override_from_env() -> bool {
    std::env::var_os(ALLOW_NETWORK_FS_ENV).is_some_and(|value| is_truthy(&value))
}

fn is_truthy(value: &OsStr) -> bool {
    value.to_str().is_some_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

/// Classifies the storage `path` lives on — `None` for (what looks like)
/// a local disk. `path` need not exist yet: its nearest existing ancestor
/// is probed, so a profile is checked *before* anything is created.
#[must_use]
pub fn detect_network_fs(path: &Path) -> Option<NetworkFsKind> {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let existing = path
        .ancestors()
        .find(|ancestor| ancestor.try_exists().unwrap_or(false))?;
    let rest = path
        .strip_prefix(existing)
        .unwrap_or_else(|_| Path::new(""));
    // Resolve symlinks, so a local-looking link into a synced folder is
    // still caught; an unreadable ancestor is probed as written.
    let existing = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.to_path_buf());
    let resolved: PathBuf = existing.join(rest);
    by_location(&resolved)
        .or_else(|| by_marker(&existing))
        .or_else(|| by_fs_type(&existing))
}

/// The UNC prefix and the well-known sync locations, from the path alone.
fn by_location(path: &Path) -> Option<NetworkFsKind> {
    let mut previous: Option<&OsStr> = None;
    for component in path.components() {
        match component {
            Component::Prefix(prefix)
                if matches!(prefix.kind(), Prefix::UNC(..) | Prefix::VerbatimUNC(..)) =>
            {
                return Some(NetworkFsKind::WindowsShare);
            }
            Component::Normal(name) => {
                if previous == Some(OsStr::new("Library")) {
                    if name == "Mobile Documents" {
                        return Some(NetworkFsKind::ICloudDrive);
                    }
                    if name == "CloudStorage" {
                        return Some(NetworkFsKind::CloudStorage);
                    }
                }
                if name == "iCloudDrive" {
                    return Some(NetworkFsKind::ICloudDrive);
                }
                previous = Some(name);
            }
            _ => previous = None,
        }
    }
    None
}

/// Sync-client markers in `existing` or any of its ancestors.
fn by_marker(existing: &Path) -> Option<NetworkFsKind> {
    existing.ancestors().find_map(|dir| {
        if dir.join(".stfolder").exists() {
            Some(NetworkFsKind::Syncthing)
        } else if dir.join(".dropbox").is_file() {
            Some(NetworkFsKind::Dropbox)
        } else {
            None
        }
    })
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn by_fs_type(existing: &Path) -> Option<NetworkFsKind> {
    let stat = rustix::fs::statfs(existing).ok()?;
    // Every magic number below fits in 32 bits; `f_type`'s width and
    // signedness differ per architecture, so compare the low 32 bits.
    let magic = u32::try_from(i128::from(stat.f_type) & 0xFFFF_FFFF).ok()?;
    kind_for_linux_magic(magic)
}

#[cfg(target_os = "macos")]
fn by_fs_type(existing: &Path) -> Option<NetworkFsKind> {
    let stat = rustix::fs::statfs(existing).ok()?;
    let name: Vec<u8> = stat
        .f_fstypename
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c.to_ne_bytes()[0])
        .collect();
    kind_for_fstypename(std::str::from_utf8(&name).ok()?)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos")))]
fn by_fs_type(_existing: &Path) -> Option<NetworkFsKind> {
    None
}

/// `statfs(2)` `f_type` magic numbers (`linux/magic.h`, `fs/smb`) of
/// network filesystems.
// Pure and compiled everywhere so its table is unit-tested on every CI
// OS; only the Linux/Android probe calls it.
#[cfg_attr(not(any(target_os = "linux", target_os = "android")), allow(dead_code))]
fn kind_for_linux_magic(magic: u32) -> Option<NetworkFsKind> {
    match magic {
        0x6969 => Some(NetworkFsKind::Nfs),
        // SMB (legacy), CIFS, SMB2/3.
        0x517B | 0xFF53_4D42 | 0xFE53_4D42 => Some(NetworkFsKind::Smb),
        // AFS, kAFS, Coda, Ceph, Lustre.
        0x5346_414F | 0x6B41_4653 | 0x7375_7245 | 0x00C3_6400 | 0x0BD0_0BD0 => {
            Some(NetworkFsKind::Distributed)
        }
        _ => None,
    }
}

/// macOS `statfs` `f_fstypename` values of network filesystems.
// Pure and compiled everywhere so its table is unit-tested on every CI
// OS; only the macOS probe calls it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn kind_for_fstypename(name: &str) -> Option<NetworkFsKind> {
    match name {
        "nfs" => Some(NetworkFsKind::Nfs),
        "smbfs" | "cifs" => Some(NetworkFsKind::Smb),
        "afpfs" => Some(NetworkFsKind::Afp),
        "webdav" => Some(NetworkFsKind::WebDav),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    // Test code: a failed unwrap is the assertion.
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_plain_temp_dir_is_local() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(detect_network_fs(dir.path()), None);
        assert_eq!(detect_network_fs(&dir.path().join("not/yet/created")), None);
    }

    #[test]
    fn a_syncthing_root_marks_everything_below_it() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".stfolder")).unwrap();
        let nested = dir.path().join("notes/curio");
        assert_eq!(
            detect_network_fs(dir.path()),
            Some(NetworkFsKind::Syncthing)
        );
        // Not created yet: the nearest existing ancestor is probed.
        assert_eq!(detect_network_fs(&nested), Some(NetworkFsKind::Syncthing));
        std::fs::create_dir_all(&nested).unwrap();
        assert_eq!(detect_network_fs(&nested), Some(NetworkFsKind::Syncthing));
        assert!(!nested.join("curio.db").exists(), "probing writes nothing");
    }

    #[test]
    fn a_dropbox_root_is_its_marker_file_not_the_config_dir() {
        let synced = tempfile::tempdir().unwrap();
        std::fs::write(synced.path().join(".dropbox"), b"{}").unwrap();
        assert_eq!(
            detect_network_fs(&synced.path().join("curio")),
            Some(NetworkFsKind::Dropbox)
        );
        // `~/.dropbox` is the client's own config directory: a home that
        // has one is not itself synced.
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".dropbox")).unwrap();
        assert_eq!(detect_network_fs(&home.path().join("curio")), None);
    }

    #[test]
    fn well_known_sync_locations_are_recognised() {
        let home = tempfile::tempdir().unwrap();
        let icloud = home
            .path()
            .join("Library/Mobile Documents/com~apple~CloudDocs/curio");
        assert_eq!(detect_network_fs(&icloud), Some(NetworkFsKind::ICloudDrive));
        let provider = home.path().join("Library/CloudStorage/Dropbox/curio");
        std::fs::create_dir_all(&provider).unwrap();
        assert_eq!(
            detect_network_fs(&provider),
            Some(NetworkFsKind::CloudStorage)
        );
        assert_eq!(
            detect_network_fs(&home.path().join("iCloudDrive/curio")),
            Some(NetworkFsKind::ICloudDrive)
        );
        // The macOS default profile location is local.
        assert_eq!(
            detect_network_fs(
                &home
                    .path()
                    .join("Library/Application Support/io.github.alexnodeland.curio")
            ),
            None
        );
        // Only `Library/<sync dir>` counts, not the bare name.
        assert_eq!(
            detect_network_fs(&home.path().join("CloudStorage/curio")),
            None
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_into_a_synced_folder_is_followed() {
        let synced = tempfile::tempdir().unwrap();
        std::fs::create_dir(synced.path().join(".stfolder")).unwrap();
        let local = tempfile::tempdir().unwrap();
        let link = local.path().join("profile");
        std::os::unix::fs::symlink(synced.path(), &link).unwrap();
        assert_eq!(detect_network_fs(&link), Some(NetworkFsKind::Syncthing));
    }

    #[cfg(windows)]
    #[test]
    fn a_unc_path_is_a_windows_share() {
        assert_eq!(
            by_location(Path::new(r"\\server\share\curio")),
            Some(NetworkFsKind::WindowsShare)
        );
        assert_eq!(
            by_location(Path::new(r"\\?\UNC\server\share\curio")),
            Some(NetworkFsKind::WindowsShare)
        );
        assert_eq!(by_location(Path::new(r"C:\Users\me\curio")), None);
    }

    #[test]
    fn linux_magic_numbers_map_to_their_class() {
        assert_eq!(kind_for_linux_magic(0x6969), Some(NetworkFsKind::Nfs));
        for smb in [0x517B, 0xFF53_4D42, 0xFE53_4D42] {
            assert_eq!(kind_for_linux_magic(smb), Some(NetworkFsKind::Smb));
        }
        assert_eq!(
            kind_for_linux_magic(0x00C3_6400),
            Some(NetworkFsKind::Distributed)
        );
        // ext4, tmpfs, btrfs: local.
        for local in [0xEF53, 0x0102_1994, 0x9123_683E] {
            assert_eq!(kind_for_linux_magic(local), None);
        }
    }

    #[test]
    fn macos_type_names_map_to_their_class() {
        assert_eq!(kind_for_fstypename("nfs"), Some(NetworkFsKind::Nfs));
        assert_eq!(kind_for_fstypename("smbfs"), Some(NetworkFsKind::Smb));
        assert_eq!(kind_for_fstypename("afpfs"), Some(NetworkFsKind::Afp));
        assert_eq!(kind_for_fstypename("webdav"), Some(NetworkFsKind::WebDav));
        for local in ["apfs", "hfs", "msdos", "exfat"] {
            assert_eq!(kind_for_fstypename(local), None);
        }
    }

    #[test]
    fn the_override_accepts_the_usual_true_spellings() {
        for yes in ["1", "true", "TRUE", "yes", "On", " 1 "] {
            assert!(is_truthy(OsStr::new(yes)), "{yes:?}");
        }
        for no in ["", "0", "false", "no", "off", "maybe"] {
            assert!(!is_truthy(OsStr::new(no)), "{no:?}");
        }
    }

    #[test]
    fn every_kind_reads_as_a_location() {
        for kind in [
            NetworkFsKind::Nfs,
            NetworkFsKind::Smb,
            NetworkFsKind::Afp,
            NetworkFsKind::WebDav,
            NetworkFsKind::Distributed,
            NetworkFsKind::WindowsShare,
            NetworkFsKind::ICloudDrive,
            NetworkFsKind::CloudStorage,
            NetworkFsKind::Dropbox,
            NetworkFsKind::Syncthing,
        ] {
            assert_ne!(kind.to_string(), "");
        }
    }
}
