//! OS-keychain storage for the few real secrets Curio holds — today,
//! the user's own Reddit API credentials (D15).
//!
//! Deliberately **pull-only**: the core never reads the keychain on its
//! own (opening a profile must never pop an OS keychain prompt, and
//! hermetic core tests must never touch the real store). The heads call
//! these functions explicitly and hand the result to
//! [`crate::CoreHandle::set_reddit_api`].
//!
//! Setting `CURIO_MOCK_KEYRING` (any value) swaps in `keyring`'s
//! in-memory mock store for the whole process — the test-only escape
//! hatch the CLI/desktop suites use to stay hermetic.

use std::sync::{Once, OnceLock};

use keyring::Entry;

/// The keychain service every Curio item lives under.
const SERVICE: &str = "Curio";
/// The account name of the Reddit API credentials item.
const REDDIT_ACCOUNT: &str = "reddit-api";

/// Keychain failures, surfaced verbatim (they are already user-facing
/// prose: locked keychain, denied access, missing backend).
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// The platform store refused the operation.
    #[error("keychain: {0}")]
    Keyring(#[from] keyring::Error),
    /// The stored item exists but is not Curio-shaped.
    #[error("keychain item is corrupt: {0}")]
    Corrupt(String),
}

/// The stored Reddit app credentials (see reddit.com/prefs/apps).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedditAppSecret {
    /// The app's client id.
    pub client_id: String,
    /// The app's client secret.
    pub client_secret: String,
}

/// Persists the Reddit credentials (one keychain item, JSON payload).
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses.
pub fn store_reddit_api(secret: &RedditAppSecret) -> Result<(), SecretsError> {
    let payload = serde_json::json!({
        "client_id": secret.client_id,
        "client_secret": secret.client_secret,
    })
    .to_string();
    reddit_entry()?.set_password(&payload)?;
    Ok(())
}

/// Loads the Reddit credentials; `None` when nothing is stored.
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses or the item is
/// not Curio-shaped.
pub fn load_reddit_api() -> Result<Option<RedditAppSecret>, SecretsError> {
    let payload = match reddit_entry()?.get_password() {
        Ok(payload) => payload,
        Err(keyring::Error::NoEntry) => return Ok(None),
        Err(err) => return Err(err.into()),
    };
    let value: serde_json::Value =
        serde_json::from_str(&payload).map_err(|err| SecretsError::Corrupt(err.to_string()))?;
    let field = |name: &str| {
        value
            .get(name)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
            .ok_or_else(|| SecretsError::Corrupt(format!("missing {name}")))
    };
    Ok(Some(RedditAppSecret {
        client_id: field("client_id")?,
        client_secret: field("client_secret")?,
    }))
}

/// Deletes the stored Reddit credentials. Idempotent — deleting an
/// absent item is a success.
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses.
pub fn clear_reddit_api() -> Result<(), SecretsError> {
    match reddit_entry()?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// One process-wide handle to the Reddit keychain item. Shared on
/// purpose: `keyring`'s mock builder mints an *independent* credential
/// per `Entry`, so per-call entries would make the test store amnesiac;
/// for the real stores a cached handle is simply cheaper.
fn reddit_entry() -> Result<&'static Entry, SecretsError> {
    static ENTRY: OnceLock<Entry> = OnceLock::new();
    ensure_store();
    if let Some(entry) = ENTRY.get() {
        return Ok(entry);
    }
    let entry = Entry::new(SERVICE, REDDIT_ACCOUNT)?;
    Ok(ENTRY.get_or_init(|| entry))
}

/// Swaps in the in-memory mock store once per process when
/// `CURIO_MOCK_KEYRING` is set (test-only; documented above).
fn ensure_store() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        if std::env::var_os("CURIO_MOCK_KEYRING").is_some() {
            keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
        }
    });
}

/// Forces the in-memory mock store for the rest of the process —
/// **test-only** (the in-process twin of `CURIO_MOCK_KEYRING`, for
/// suites that cannot set an env var safely). Call it before any other
/// function in this module.
pub fn use_mock_store_for_tests() {
    keyring::set_default_credential_builder(keyring::mock::default_credential_builder());
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    /// One test drives the whole round trip: the credential builder is
    /// process-global, so splitting these into separate tests would
    /// race on install order.
    #[test]
    fn the_round_trip_works_against_the_mock_store() {
        use_mock_store_for_tests();

        assert_eq!(load_reddit_api().unwrap(), None, "empty store");
        clear_reddit_api().unwrap(); // idempotent on absence

        let secret = RedditAppSecret {
            client_id: "cid123".to_owned(),
            client_secret: "shh".to_owned(),
        };
        store_reddit_api(&secret).unwrap();
        assert_eq!(load_reddit_api().unwrap(), Some(secret.clone()));

        // Overwrite wins.
        let rotated = RedditAppSecret {
            client_secret: "shh2".to_owned(),
            ..secret
        };
        store_reddit_api(&rotated).unwrap();
        assert_eq!(load_reddit_api().unwrap(), Some(rotated));

        clear_reddit_api().unwrap();
        assert_eq!(load_reddit_api().unwrap(), None);
    }
}
