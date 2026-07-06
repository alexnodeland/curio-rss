//! The path gate: raw filesystem paths never cross IPC as free strings.
//!
//! Grafted from the desktop-monolith design (decisions.md D13). Dialog
//! picks happen Rust-side (`pick_import_file` / `pick_export_path` /
//! `pick_destination_root`); every picked path is canonicalized, stored
//! in this session registry, and returned to the frontend as an opaque
//! single-use token (ULID). Path-consuming commands (`add_destination`,
//! `import_opml`, `export_opml`, `copy_diagnostics_bundle`'s reveal)
//! accept only a token — redeeming checks the declared intent and burns
//! the token. This complements core's own named-destination containment
//! assertion: defense-in-depth, not the primary defense.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::dto::PathTokenDto;
use crate::error::CommandError;

/// What a registered path may be used for. Redeeming with a different
/// intent fails — an import pick can never become a write target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathIntent {
    /// A file to read (OPML import).
    ImportFile,
    /// A file to (over)write (OPML export).
    ExportFile,
    /// A directory to register as a destination root.
    DestinationRoot,
    /// Display/reveal only — never read or written via a command.
    Reveal,
}

struct RegisteredPath {
    path: PathBuf,
    intent: PathIntent,
}

/// The session path registry, managed as Tauri state. Tokens live until
/// redeemed or the app exits; nothing is persisted.
#[derive(Debug, Default)]
pub struct PathRegistry {
    inner: Mutex<HashMap<String, RegisteredPath>>,
}

impl std::fmt::Debug for RegisteredPath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Paths are user-private; keep them out of debug output.
        f.debug_struct("RegisteredPath")
            .field("intent", &self.intent)
            .finish_non_exhaustive()
    }
}

impl PathRegistry {
    /// A fresh, empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Canonicalizes and registers a dialog-picked path, minting a
    /// single-use token. For [`PathIntent::ExportFile`] the file may not
    /// exist yet, so the parent is canonicalized and the file name
    /// re-joined; every other intent requires the path to exist.
    ///
    /// # Errors
    ///
    /// [`CommandError`] (user tier) when the path cannot be canonicalized.
    pub fn mint(&self, path: &Path, intent: PathIntent) -> Result<PathTokenDto, CommandError> {
        let canonical = canonicalize_for(path, intent)?;
        let token = ulid::Ulid::new().to_string();
        let display = canonical.display().to_string();
        self.lock().insert(
            token.clone(),
            RegisteredPath {
                path: canonical,
                intent,
            },
        );
        Ok(PathTokenDto {
            token,
            path: display,
        })
    }

    /// Redeems (and burns) a token, returning the registered path.
    ///
    /// # Errors
    ///
    /// [`CommandError`] (user tier) for an unknown/already-used token or
    /// an intent mismatch.
    pub fn redeem(&self, token: &str, intent: PathIntent) -> Result<PathBuf, CommandError> {
        let entry = self.lock().remove(token).ok_or_else(|| {
            CommandError::invalid_input("unknown or already-used path token — pick the path again")
        })?;
        if entry.intent != intent {
            return Err(CommandError::invalid_input(
                "path token was minted for a different purpose — pick the path again",
            ));
        }
        Ok(entry.path)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, RegisteredPath>> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn canonicalize_for(path: &Path, intent: PathIntent) -> Result<PathBuf, CommandError> {
    match intent {
        PathIntent::ExportFile => {
            let parent = path.parent().ok_or_else(|| {
                CommandError::invalid_input("export path has no parent directory")
            })?;
            let file_name = path
                .file_name()
                .ok_or_else(|| CommandError::invalid_input("export path has no file name"))?;
            let canonical_parent = parent.canonicalize().map_err(|error| {
                CommandError::invalid_input(format!("export directory is not usable: {error}"))
            })?;
            Ok(canonical_parent.join(file_name))
        }
        PathIntent::ImportFile | PathIntent::DestinationRoot | PathIntent::Reveal => {
            path.canonicalize().map_err(|error| {
                CommandError::invalid_input(format!("picked path is not usable: {error}"))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn mint_then_redeem_returns_the_canonical_path() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("subs.opml");
        std::fs::write(&file, "<opml/>").unwrap();

        let registry = PathRegistry::new();
        let minted = registry.mint(&file, PathIntent::ImportFile).unwrap();
        let redeemed = registry
            .redeem(&minted.token, PathIntent::ImportFile)
            .unwrap();
        assert_eq!(redeemed, file.canonicalize().unwrap());
        assert_eq!(minted.path, redeemed.display().to_string());
    }

    #[test]
    fn tokens_are_single_use() {
        let dir = tempfile::tempdir().unwrap();
        let registry = PathRegistry::new();
        let minted = registry
            .mint(dir.path(), PathIntent::DestinationRoot)
            .unwrap();
        registry
            .redeem(&minted.token, PathIntent::DestinationRoot)
            .unwrap();
        let second = registry.redeem(&minted.token, PathIntent::DestinationRoot);
        assert!(second.is_err(), "a burned token must not redeem twice");
    }

    #[test]
    fn intent_mismatch_is_refused_and_burns_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let registry = PathRegistry::new();
        let minted = registry
            .mint(dir.path(), PathIntent::DestinationRoot)
            .unwrap();
        assert!(
            registry
                .redeem(&minted.token, PathIntent::ImportFile)
                .is_err()
        );
        // Burned on the failed attempt — defense-in-depth.
        assert!(
            registry
                .redeem(&minted.token, PathIntent::DestinationRoot)
                .is_err()
        );
    }

    #[test]
    fn export_paths_may_not_exist_yet_but_their_parent_must() {
        let dir = tempfile::tempdir().unwrap();
        let registry = PathRegistry::new();
        let target = dir.path().join("out.opml");
        let minted = registry.mint(&target, PathIntent::ExportFile).unwrap();
        let redeemed = registry
            .redeem(&minted.token, PathIntent::ExportFile)
            .unwrap();
        assert_eq!(redeemed.file_name().unwrap(), "out.opml");

        let missing_parent = dir.path().join("nope").join("out.opml");
        assert!(
            registry
                .mint(&missing_parent, PathIntent::ExportFile)
                .is_err()
        );
    }

    #[test]
    fn nonexistent_import_paths_are_refused_at_mint() {
        let registry = PathRegistry::new();
        let err = registry.mint(
            Path::new("/definitely/not/here.opml"),
            PathIntent::ImportFile,
        );
        assert!(err.is_err());
    }

    #[test]
    fn bogus_tokens_are_a_user_error() {
        let registry = PathRegistry::new();
        let error = registry
            .redeem("01JUNKJUNKJUNKJUNKJUNKJUNK", PathIntent::ImportFile)
            .unwrap_err();
        assert_eq!(error.kind, crate::error::ErrorKind::User);
    }
}
