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
    ///
    /// A profile on network or sync-managed storage is refused unless
    /// `allow_network_fs` (`--allow-network-fs`) or the
    /// `CURIO_ALLOW_NETWORK_FS` override is set.
    pub(crate) fn open(
        profile: Option<PathBuf>,
        json: bool,
        allow_network_fs: bool,
        mode: OpenMode,
    ) -> anyhow::Result<Self> {
        Self::open_in(
            profile,
            json,
            allow_network_fs,
            mode,
            &ProfileLayout::current,
        )
    }

    /// [`Self::open`] over an explicit default/legacy layout — the seam the
    /// tests drive. `layout` is consulted only when no `--profile` /
    /// `CURIO_PROFILE` was given.
    fn open_in(
        profile: Option<PathBuf>,
        json: bool,
        allow_network_fs: bool,
        mode: OpenMode,
        layout: &dyn Fn() -> anyhow::Result<ProfileLayout>,
    ) -> anyhow::Result<Self> {
        let (profile, is_default_profile) = if let Some(dir) = profile {
            (dir, false)
        } else {
            let layout = layout()?;
            adopt_legacy(&layout)?;
            (layout.default, true)
        };
        if mode == OpenMode::Existing {
            ensure_profile_exists(&profile)?;
        }
        let config = ProfileConfig::load(&profile)?;
        let core = CoreHandle::open_with(
            &profile,
            CoreOptions {
                fetch: config.fetch_config(),
                allow_network_fs: allow_network_fs
                    || curio_core::storage::network_fs_override_from_env(),
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

/// Where the default profile lives and where earlier releases kept one —
/// what a run without `--profile` / `CURIO_PROFILE` resolves against.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ProfileLayout {
    /// The default profile shared with the desktop app
    /// ([`curio_core::paths::default_profile_dir`]).
    default: PathBuf,
    /// [`curio_core::paths::legacy_profile_dirs`].
    legacy: Vec<PathBuf>,
}

impl ProfileLayout {
    /// This user's layout on this platform.
    fn current() -> anyhow::Result<Self> {
        Ok(Self {
            default: curio_core::paths::default_profile_dir()
                .context("pass --profile or set CURIO_PROFILE")?,
            legacy: curio_core::paths::legacy_profile_dirs(),
        })
    }
}

/// The one-time move of a pre-0.5 profile onto the shared default. What it
/// could not move is reported on stderr; when that leaves a library behind
/// while the default holds none, the run stops — creating (or pointing the
/// user at `init` for) a fresh default would strand that library for good.
fn adopt_legacy(layout: &ProfileLayout) -> anyhow::Result<()> {
    let outcomes = curio_core::paths::adopt_legacy_profiles(&layout.default, &layout.legacy);
    for notice in &outcomes {
        eprintln!("{}", legacy_notice(notice));
    }
    if let Some(stranded) = outcomes.iter().find_map(|outcome| match outcome {
        LegacyProfile::Left { path, .. } if outcome.would_be_stranded() => Some(path),
        _ => None,
    }) {
        anyhow::bail!(
            "not creating a new default profile at {} while the library from an earlier \
             release (see the warning above) is left behind — resolve that and run curio \
             again, or open the old library with --profile {}",
            layout.default.display(),
            stranded.display()
        );
    }
    Ok(())
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
        let layout = ProfileLayout::current().unwrap();
        // The desktop head resolves through the same core functions (its
        // own test pins that side), so both heads share one database.
        assert_eq!(
            layout.default,
            curio_core::paths::default_profile_dir().unwrap()
        );
        assert_eq!(layout.legacy, curio_core::paths::legacy_profile_dirs());
    }

    /// A temp default + one legacy location holding a real profile.
    struct Fixture {
        _tmp: tempfile::TempDir,
        layout: ProfileLayout,
    }

    impl Fixture {
        fn with_legacy_profile() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let legacy = tmp.path().join("old-cli");
            drop(App::open(Some(legacy.clone()), false, false, OpenMode::Create).unwrap());
            assert!(curio_core::paths::is_profile(&legacy));
            let layout = ProfileLayout {
                default: tmp.path().join("shared").join("default"),
                legacy: vec![legacy],
            };
            Self { _tmp: tmp, layout }
        }

        fn open(&self, flag: Option<PathBuf>, mode: OpenMode) -> anyhow::Result<App> {
            let layout = self.layout.clone();
            App::open_in(flag, false, false, mode, &move || Ok(layout.clone()))
        }

        fn legacy(&self) -> &Path {
            &self.layout.legacy[0]
        }
    }

    #[test]
    fn the_default_profile_adopts_a_lone_legacy_profile() {
        let fx = Fixture::with_legacy_profile();
        let app = fx.open(None, OpenMode::Existing).unwrap();
        assert!(app.is_default_profile);
        assert_eq!(app.profile, fx.layout.default);
        assert!(curio_core::paths::is_profile(&fx.layout.default));
        assert!(!fx.legacy().exists(), "moved, not copied");
    }

    #[test]
    fn an_explicit_profile_never_adopts() {
        let fx = Fixture::with_legacy_profile();
        let explicit = fx.layout.default.with_file_name("explicit");
        let app = fx.open(Some(explicit.clone()), OpenMode::Create).unwrap();
        assert!(!app.is_default_profile);
        assert_eq!(app.profile, explicit);
        assert!(curio_core::paths::is_profile(fx.legacy()), "left alone");
        assert!(!fx.layout.default.exists());
    }

    #[test]
    fn a_library_left_behind_blocks_a_fresh_default() {
        let fx = Fixture::with_legacy_profile();
        // Another process holds the old library open, so it cannot move.
        let holder = App::open(
            Some(fx.legacy().to_path_buf()),
            false,
            false,
            OpenMode::Existing,
        )
        .unwrap();
        for mode in [OpenMode::Existing, OpenMode::Create] {
            let err = fx.open(None, mode).unwrap_err().to_string();
            assert!(err.contains("not creating a new default profile"), "{err}");
            assert!(err.contains(&fx.legacy().display().to_string()), "{err}");
            assert!(!err.contains("run `curio init`"), "{err}");
            assert!(!fx.layout.default.exists(), "nothing created");
        }
        // Once it is released, the next run moves it.
        drop(holder);
        let app = fx.open(None, OpenMode::Existing).unwrap();
        assert_eq!(app.profile, fx.layout.default);
        assert!(!fx.legacy().exists());
    }

    #[test]
    fn with_no_legacy_profile_the_default_still_needs_init() {
        let tmp = tempfile::tempdir().unwrap();
        let layout = ProfileLayout {
            default: tmp.path().join("default"),
            legacy: vec![tmp.path().join("absent")],
        };
        let open = |mode| {
            let layout = layout.clone();
            App::open_in(None, false, false, mode, &move || Ok(layout.clone()))
        };
        let err = open(OpenMode::Existing).unwrap_err().to_string();
        assert!(err.contains("run `curio init`"), "{err}");
        let app = open(OpenMode::Create).unwrap();
        assert!(app.is_default_profile);
        assert!(curio_core::paths::is_profile(&layout.default));
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
