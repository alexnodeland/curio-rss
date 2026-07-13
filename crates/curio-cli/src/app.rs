//! The opened application: profile dir + config + engine handle.

use std::path::PathBuf;

use anyhow::Context as _;
use curio_core::{CoreHandle, CoreOptions};
use curio_types::DestinationName;

use crate::config::ProfileConfig;

/// Everything a command needs: the resolved profile, its configuration,
/// the engine, and the output mode.
#[derive(Debug)]
pub(crate) struct App {
    pub(crate) profile: PathBuf,
    pub(crate) config: ProfileConfig,
    pub(crate) core: CoreHandle,
    pub(crate) json: bool,
}

impl App {
    /// Opens (creating if absent) the profile: loads `curio.toml`, opens
    /// the engine with the configured fetch policy, and reconciles the
    /// config-owned surfaces (destinations, W1 flags) into the engine.
    pub(crate) fn open(profile: Option<PathBuf>, json: bool) -> anyhow::Result<Self> {
        let profile = resolve_profile_dir(profile)?;
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

/// `--profile` / `CURIO_PROFILE`, else the platform data directory
/// (XDG-respecting on Linux, `~/Library/Application Support` on macOS).
pub(crate) fn resolve_profile_dir(flag: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    if let Some(dir) = flag {
        return Ok(dir);
    }
    directories::ProjectDirs::from("", "", "curio")
        .map(|dirs| dirs.data_dir().to_path_buf())
        .context(
            "cannot determine the platform data directory — pass --profile or set CURIO_PROFILE",
        )
}

/// Loads the optional Reddit API credentials from the OS keychain into
/// the runtime core (D15). Best-effort on purpose: a locked or missing
/// keychain backend (headless Linux without secret-service) degrades to
/// the unauthenticated tier with a warning, never a failed command.
fn load_reddit_credentials(core: &CoreHandle) {
    match curio_core::secrets::load_reddit_api() {
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

/// Applies curio.toml onto the engine: destinations are (re-)registered,
/// and per-feed W1 flags are pushed onto matching subscriptions.
fn reconcile(core: &CoreHandle, config: &ProfileConfig) -> anyhow::Result<()> {
    for (name, root) in &config.destinations {
        let name: DestinationName = name
            .parse()
            .with_context(|| format!("invalid destination name {name:?} in curio.toml"))?;
        core.add_destination(name, root.clone())?;
    }
    for (url, overrides) in &config.feeds {
        if let Some(feed) = core.storage().get_feed_by_url(url)? {
            if feed.allow_private_network != overrides.allow_private_network {
                core.set_feed_allow_private_network(feed.id, overrides.allow_private_network)?;
            }
        }
    }
    Ok(())
}
