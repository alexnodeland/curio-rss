//! `curio.toml` — the human-editable profile configuration.
//!
//! Ownership split: feeds, articles, and state live in `curio.db` (the
//! engine's domain); this file owns what a human should be able to edit
//! with a text editor — named destinations, settings, and per-feed
//! overrides. Contract W1 lives here on purpose: `allow_private_network`
//! requires an explicit config edit (this file, or the `feed add
//! --allow-private-network` flag that writes it) and is never settable
//! from feed content.
//!
//! On every run the CLI reconciles this file *into* the engine:
//! destinations are (re-)registered and W1 flags are applied to matching
//! subscriptions. The full format is documented in `docs/cli.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context as _;
use curio_core::fetch::FetchConfig;
use serde::{Deserialize, Serialize};

/// File name inside the profile directory.
pub(crate) const CONFIG_FILE: &str = "curio.toml";

const HEADER: &str = "\
# curio.toml — profile configuration for the `curio` CLI (docs/cli.md).
#
# Feeds, articles, and reading state live in curio.db next to this file;
# curio.toml owns destinations, settings, and per-feed overrides.
# Contract W1: [feeds.\"<url>\"] allow_private_network = true exempts one
# feed from the SSRF private-network guard — an explicit config edit,
# never settable from feed content.

";

/// The whole `curio.toml` document.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct ProfileConfig {
    /// `[settings]`.
    #[serde(default)]
    pub(crate) settings: Settings,
    /// `[destinations]`: name → directory.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) destinations: BTreeMap<String, PathBuf>,
    /// `[feeds."<url>"]` per-feed overrides.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub(crate) feeds: BTreeMap<String, FeedOverrides>,
}

/// `[settings]` — global knobs.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct Settings {
    /// Destination `curio save` uses when `--dest` is omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) default_destination: Option<String>,
    /// Minimum spacing between requests to the same host (default 500).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) politeness_delay_ms: Option<u64>,
    /// Override the outbound User-Agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) user_agent: Option<String>,
}

/// `[feeds."<url>"]` — per-feed overrides keyed by feed URL.
#[derive(Debug, Default, Serialize, Deserialize)]
pub(crate) struct FeedOverrides {
    /// Contract W1: exempt this feed from the SSRF private-network guard.
    #[serde(default)]
    pub(crate) allow_private_network: bool,
}

impl ProfileConfig {
    /// Loads `curio.toml` from a profile directory; a missing file is the
    /// default configuration.
    pub(crate) fn load(profile: &Path) -> anyhow::Result<Self> {
        let path = profile.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&raw).with_context(|| format!("parsing {}", path.display()))
    }

    /// Writes `curio.toml` back into a profile directory.
    pub(crate) fn save(&self, profile: &Path) -> anyhow::Result<()> {
        let body = toml::to_string_pretty(self).context("serializing curio.toml")?;
        let path = profile.join(CONFIG_FILE);
        std::fs::write(&path, format!("{HEADER}{body}"))
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The outbound-client policy this configuration asks for.
    pub(crate) fn fetch_config(&self) -> FetchConfig {
        let mut fetch = FetchConfig::default();
        if let Some(ms) = self.settings.politeness_delay_ms {
            fetch.politeness_delay = Duration::from_millis(ms);
        }
        if let Some(user_agent) = &self.settings.user_agent {
            fetch.user_agent.clone_from(user_agent);
        }
        fetch
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn a_missing_file_is_the_default_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = ProfileConfig::load(dir.path()).unwrap();
        assert!(config.destinations.is_empty());
        assert!(config.feeds.is_empty());
        assert_eq!(
            config.fetch_config().politeness_delay,
            FetchConfig::default().politeness_delay
        );
    }

    #[test]
    fn config_round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = ProfileConfig::default();
        config.settings.default_destination = Some("vault".to_owned());
        config.settings.politeness_delay_ms = Some(0);
        config
            .destinations
            .insert("vault".to_owned(), PathBuf::from("/tmp/vault"));
        config.feeds.insert(
            "http://127.0.0.1:8080/feed.xml".to_owned(),
            FeedOverrides {
                allow_private_network: true,
            },
        );
        config.save(dir.path()).unwrap();

        let loaded = ProfileConfig::load(dir.path()).unwrap();
        assert_eq!(
            loaded.settings.default_destination.as_deref(),
            Some("vault")
        );
        assert_eq!(loaded.fetch_config().politeness_delay, Duration::ZERO);
        assert!(loaded.feeds["http://127.0.0.1:8080/feed.xml"].allow_private_network);
        assert_eq!(loaded.destinations["vault"], PathBuf::from("/tmp/vault"));
    }
}
