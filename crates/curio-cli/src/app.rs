//! The opened application: profile dir + config + engine handle.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use curio_core::paths::LegacyProfile;
use curio_core::{CoreHandle, CoreOptions};
use curio_types::DestinationName;

use crate::config::ProfileConfig;

/// Everything a command needs: the resolved profile, its configuration,
/// the engine, and the output mode.
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) profile: PathBuf,
    /// `true` when `profile` is the default one shared with the desktop
    /// app (no `--profile` / `CURIO_PROFILE`).
    pub(crate) is_default_profile: bool,
    pub(crate) config: ProfileConfig,
    pub(crate) core: CoreHandle,
    pub(crate) json: bool,
}

impl App {
    /// Opens the profile: loads `curio.toml`, opens the engine with the
    /// configured fetch policy, and reconciles the config-owned surfaces
    /// (destinations, W1 flags) into the engine.
    ///
    /// Only [`OpenMode::Create`] (`curio init`) may create a profile; every
    /// other command refuses a directory that holds none, so a mistyped
    /// `--profile` fails loudly instead of printing an empty library.
    pub(crate) fn open(
        profile: Option<PathBuf>,
        json: bool,
        mode: OpenMode,
    ) -> anyhow::Result<Self> {
        let choice = resolve_profile_dir(profile)?;
        if choice.is_default {
            // The one-time move of a pre-0.5 profile onto the shared
            // default; whatever it could not move is reported, never fatal.
            for notice in curio_core::paths::adopt_legacy_profiles(
                &choice.dir,
                &curio_core::paths::legacy_profile_dirs(),
            ) {
                eprintln!("{}", legacy_notice(&notice));
            }
        }
        let ProfileChoice {
            dir: profile,
            is_default: is_default_profile,
        } = choice;
        if mode == OpenMode::Existing {
            ensure_profile_exists(&profile)?;
        }
        let config = ProfileConfig::load(&profile)?;
        let core = CoreHandle::open_with(
            &profile,
            CoreOptions {
                fetch: config.fetch_config(),
            },
        )
        .with_context(|| format!("opening profile {}", profile.display()))?;
        reconcile(&core, &config)?;
        load_reddit_credentials(&core);
        Ok(Self {
            profile,
            is_default_profile,
            config,
            core,
            json,
        })
    }

    /// Persists the in-memory config back to `curio.toml`.
    pub(crate) fn save_config(&self) -> anyhow::Result<()> {
        self.config.save(&self.profile)
    }
}

/// Whether opening may create the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenMode {
    /// `curio init`: scaffold the profile when it is absent.
    Create,
    /// Every other command: the profile must already exist.
    Existing,
}

/// The profile directory a command runs against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileChoice {
    pub(crate) dir: PathBuf,
    /// `true` when neither `--profile` nor `CURIO_PROFILE` was given, so
    /// this is the default profile the desktop app opens too.
    pub(crate) is_default: bool,
}

/// `--profile` / `CURIO_PROFILE`, else the default profile shared with the
/// desktop app ([`curio_core::paths::default_profile_dir`]).
pub(crate) fn resolve_profile_dir(flag: Option<PathBuf>) -> anyhow::Result<ProfileChoice> {
    if let Some(dir) = flag {
        return Ok(ProfileChoice {
            dir,
            is_default: false,
        });
    }
    let dir =
        curio_core::paths::default_profile_dir().context("pass --profile or set CURIO_PROFILE")?;
    Ok(ProfileChoice {
        dir,
        is_default: true,
    })
}

/// Refuses a directory that holds no profile, pointing at `curio init`.
fn ensure_profile_exists(profile: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        curio_core::paths::is_profile(profile),
        "no curio profile at {} — run `curio init` to create it (or check --profile / CURIO_PROFILE)",
        profile.display()
    );
    Ok(())
}

/// The stderr line for one legacy-profile outcome.
fn legacy_notice(notice: &LegacyProfile) -> String {
    match notice {
        LegacyProfile::Migrated { .. } => format!("note: {notice}"),
        LegacyProfile::Left { .. } => format!("warning: {notice}"),
    }
}

/// Loads the profile's optional Reddit API credentials from the OS
/// keychain into the runtime core (D15). Best-effort on purpose: a
/// locked or missing keychain backend (headless Linux without
/// secret-service) degrades to the unauthenticated tier with a warning,
/// never a failed command.
fn load_reddit_credentials(core: &CoreHandle) {
    match curio_core::secrets::load_reddit_api(core.profile_dir()) {
        Ok(Some(secret)) => {
            core.set_reddit_api(Some(curio_core::enrich::reddit_auth::RedditApiConfig::new(
                secret.client_id,
                secret.client_secret,
            )));
        }
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(%err, "keychain unavailable; reddit stays unauthenticated");
        }
    }
}

/// Applies curio.toml onto the engine: destinations are reconciled (never
/// recreated once exported to — a vanished vault must stay visible),
/// and per-feed W1 flags are pushed onto matching subscriptions.
fn reconcile(core: &CoreHandle, config: &ProfileConfig) -> anyhow::Result<()> {
    for (name, root) in &config.destinations {
        let name: DestinationName = name
            .parse()
            .with_context(|| format!("invalid destination name {name:?} in curio.toml"))?;
        core.reconcile_destination(name, root.clone())?;
    }
    for (url, overrides) in &config.feeds {
        if let Some(feed) = core.get_feed_by_url(url)?
            && feed.allow_private_network != overrides.allow_private_network
        {
            core.set_feed_allow_private_network(feed.id, overrides.allow_private_network)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use curio_core::paths::LeftReason;

    use super::*;

    #[test]
    fn the_default_profile_is_the_one_the_desktop_opens() {
        let choice = resolve_profile_dir(None).unwrap();
        assert!(choice.is_default);
        // The desktop head resolves through the same core function (its
        // own test pins that side), so both heads share one database.
        assert_eq!(
            choice.dir,
            curio_core::paths::default_profile_dir().unwrap()
        );
    }

    #[test]
    fn an_explicit_profile_is_not_the_default() {
        let choice = resolve_profile_dir(Some(PathBuf::from("/tmp/p"))).unwrap();
        assert_eq!(
            choice,
            ProfileChoice {
                dir: PathBuf::from("/tmp/p"),
                is_default: false
            }
        );
    }

    #[test]
    fn a_directory_without_a_database_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let err = ensure_profile_exists(dir.path()).unwrap_err().to_string();
        assert!(err.contains("run `curio init`"), "{err}");
        std::fs::write(dir.path().join(curio_core::paths::DB_FILE), b"").unwrap();
        ensure_profile_exists(dir.path()).unwrap();
    }

    #[test]
    fn legacy_notices_are_labelled_by_severity() {
        let moved = LegacyProfile::Migrated {
            from: PathBuf::from("/old"),
            to: PathBuf::from("/new"),
        };
        assert!(legacy_notice(&moved).starts_with("note: moved the profile"));
        let left = LegacyProfile::Left {
            path: PathBuf::from("/old"),
            reason: LeftReason::DefaultInUse,
        };
        assert!(legacy_notice(&left).starts_with("warning: a profile from an earlier"));
    }
}
