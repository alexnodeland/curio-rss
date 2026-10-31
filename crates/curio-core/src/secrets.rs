//! OS-keychain storage for the few real secrets Curio holds — today,
//! the user's own Reddit API credentials (D15).
//!
//! Deliberately **pull-only**: the core never reads the keychain on its
//! own (opening a profile must never pop an OS keychain prompt, and
//! hermetic core tests must never touch the real store). The heads call
//! these functions explicitly and hand the result to
//! [`crate::CoreHandle::set_reddit_api`].
//!
//! Items are **scoped per profile**: every function takes the profile
//! directory, and each profile's credentials live in their own keychain
//! item, so two profiles can hold different Reddit apps.
//!
//! Setting `CURIO_MOCK_KEYRING` (any value) swaps in `keyring`'s
//! in-memory mock store for the whole process — the test-only escape
//! hatch the CLI/desktop suites use to stay hermetic.

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex, Once, OnceLock, PoisonError};

use keyring::Entry;
use sha2::{Digest as _, Sha256};

/// The keychain service every Curio item lives under.
const SERVICE: &str = "Curio";
/// The account name of the Reddit API credentials. Alone it names the
/// legacy global item (shared by every profile, see [`load_reddit_api`]);
/// profile items append the profile key: `reddit-api/<key>`.
const REDDIT_ACCOUNT: &str = "reddit-api";
/// What the redacting `Debug` impls print in place of a secret.
const REDACTED: &str = "<redacted>";

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
///
/// `Debug` is hand-written and redacts the secret, so the value is safe
/// in a log line, `dbg!` or panic message.
#[derive(Clone, PartialEq, Eq)]
pub struct RedditAppSecret {
    /// The app's client id.
    pub client_id: String,
    /// The app's client secret.
    pub client_secret: String,
}

impl fmt::Debug for RedditAppSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedditAppSecret")
            .field("client_id", &self.client_id)
            .field("client_secret", &REDACTED)
            .finish()
    }
}

/// Persists the Reddit credentials of the profile at `profile_dir` (one
/// keychain item per profile, JSON payload).
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses.
pub fn store_reddit_api(profile_dir: &Path, secret: &RedditAppSecret) -> Result<(), SecretsError> {
    let entry = reddit_entry(&profile_account(profile_dir))?;
    write_secret(&entry, secret)
}

/// Loads the Reddit credentials of the profile at `profile_dir`; `None`
/// when nothing is stored.
///
/// **Migration:** earlier releases kept one global item (account
/// `reddit-api`) shared by every profile. When this profile has no item
/// of its own but the global one exists, the first profile to load
/// claims it: the credentials move into this profile's item and the
/// global item is deleted, so it can never resurface after a logout.
/// Any other profile then needs its own login.
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses or the item is
/// not Curio-shaped.
pub fn load_reddit_api(profile_dir: &Path) -> Result<Option<RedditAppSecret>, SecretsError> {
    let scoped = reddit_entry(&profile_account(profile_dir))?;
    if let Some(secret) = read_secret(&scoped)? {
        return Ok(Some(secret));
    }
    let legacy = reddit_entry(REDDIT_ACCOUNT)?;
    let Some(secret) = read_secret(&legacy)? else {
        return Ok(None);
    };
    write_secret(&scoped, &secret)?;
    delete_secret(&legacy)?;
    Ok(Some(secret))
}

/// Deletes the stored Reddit credentials of the profile at
/// `profile_dir`. Idempotent — deleting an absent item is a success.
///
/// A still-unclaimed legacy global item (see [`load_reddit_api`]) is
/// deleted too: it would otherwise migrate straight back into this
/// profile on the next load, undoing the logout.
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses.
pub fn clear_reddit_api(profile_dir: &Path) -> Result<(), SecretsError> {
    let scoped = reddit_entry(&profile_account(profile_dir))?;
    delete_secret(&scoped)?;
    let legacy = reddit_entry(REDDIT_ACCOUNT)?;
    delete_secret(&legacy)
}

/// A stable, length-bounded key for a profile directory: the first 16
/// bytes of the sha256 of its canonical path, hex-encoded. Canonical so
/// a relative path, a symlink and the absolute path agree (the CLI and
/// the desktop app resolve their profile independently); hashed so the
/// keychain's metadata never carries the user's directory layout and the
/// account name fits every platform store's length limit. A directory
/// that cannot be canonicalized (not created yet) keys by its path as
/// given.
#[must_use]
pub fn profile_key(profile_dir: &Path) -> String {
    let canonical =
        std::fs::canonicalize(profile_dir).unwrap_or_else(|_| profile_dir.to_path_buf());
    let digest = Sha256::digest(canonical.as_os_str().as_encoded_bytes());
    digest
        .iter()
        .take(16)
        .fold(String::with_capacity(32), |mut hex, byte| {
            // Writing into a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The keychain account of a profile's Reddit item.
fn profile_account(profile_dir: &Path) -> String {
    format!("{REDDIT_ACCOUNT}/{}", profile_key(profile_dir))
}

fn write_secret(entry: &Entry, secret: &RedditAppSecret) -> Result<(), SecretsError> {
    let payload = serde_json::json!({
        "client_id": secret.client_id,
        "client_secret": secret.client_secret,
    })
    .to_string();
    entry.set_password(&payload)?;
    Ok(())
}

fn read_secret(entry: &Entry) -> Result<Option<RedditAppSecret>, SecretsError> {
    let payload = match entry.get_password() {
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

fn delete_secret(entry: &Entry) -> Result<(), SecretsError> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// One process-wide handle per keychain account. Shared on purpose:
/// `keyring`'s mock builder mints an *independent* credential per
/// `Entry`, so per-call entries would make the test store amnesiac; for
/// the real stores a cached handle is simply cheaper. Bounded by the
/// profiles one process touches (one, in practice) plus the legacy
/// account.
fn reddit_entry(account: &str) -> Result<Arc<Entry>, SecretsError> {
    static ENTRIES: OnceLock<Mutex<HashMap<String, Arc<Entry>>>> = OnceLock::new();
    ensure_store();
    let mut entries = ENTRIES
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    if let Some(entry) = entries.get(account) {
        return Ok(Arc::clone(entry));
    }
    let entry = Arc::new(Entry::new(SERVICE, account)?);
    entries.insert(account.to_owned(), Arc::clone(&entry));
    Ok(entry)
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

    /// One test drives the whole mock-store story: the credential
    /// builder is process-global, so splitting these into separate tests
    /// would race on install order.
    #[test]
    fn the_round_trip_is_scoped_per_profile_and_migrates_the_global_item() {
        use_mock_store_for_tests();
        let root = tempfile::tempdir().unwrap();
        let alpha = root.path().join("alpha");
        let beta = root.path().join("beta");
        std::fs::create_dir_all(&alpha).unwrap();
        std::fs::create_dir_all(&beta).unwrap();

        assert_eq!(load_reddit_api(&alpha).unwrap(), None, "empty store");
        clear_reddit_api(&alpha).unwrap(); // idempotent on absence

        let secret = RedditAppSecret {
            client_id: "cid123".to_owned(),
            client_secret: "shh".to_owned(),
        };
        store_reddit_api(&alpha, &secret).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(secret.clone()));
        // Another profile does not see alpha's credentials…
        assert_eq!(load_reddit_api(&beta).unwrap(), None);

        // …and holds different ones side by side.
        let other = RedditAppSecret {
            client_id: "cid456".to_owned(),
            client_secret: "hush".to_owned(),
        };
        store_reddit_api(&beta, &other).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(secret.clone()));
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(other.clone()));

        // Overwrite wins, per profile.
        let rotated = RedditAppSecret {
            client_secret: "shh2".to_owned(),
            ..secret
        };
        store_reddit_api(&alpha, &rotated).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(rotated));
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(other.clone()));

        // Logging one profile out leaves the other intact.
        clear_reddit_api(&alpha).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), None);
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(other));
        clear_reddit_api(&beta).unwrap();

        // The legacy global item is claimed by the first profile that
        // loads, then gone — a second profile does not inherit it.
        let legacy = RedditAppSecret {
            client_id: "old".to_owned(),
            client_secret: "older".to_owned(),
        };
        let legacy_entry = reddit_entry(REDDIT_ACCOUNT).unwrap();
        write_secret(&legacy_entry, &legacy).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(legacy.clone()));
        assert_eq!(read_secret(&legacy_entry).unwrap(), None);
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(legacy.clone()));
        assert_eq!(load_reddit_api(&beta).unwrap(), None);
        clear_reddit_api(&alpha).unwrap();

        // A logout also drops a still-unclaimed global item, so it can't
        // migrate straight back in.
        write_secret(&legacy_entry, &legacy).unwrap();
        clear_reddit_api(&alpha).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), None);

        // A corrupt item is reported, not silently treated as absent.
        let alpha_entry = reddit_entry(&profile_account(&alpha)).unwrap();
        alpha_entry.set_password("not json").unwrap();
        assert!(matches!(
            load_reddit_api(&alpha),
            Err(SecretsError::Corrupt(_))
        ));
        alpha_entry.set_password(r#"{"client_id":"x"}"#).unwrap();
        assert!(matches!(
            load_reddit_api(&alpha),
            Err(SecretsError::Corrupt(_))
        ));
        clear_reddit_api(&alpha).unwrap();
    }

    #[test]
    fn the_profile_key_is_canonical_bounded_and_distinct() {
        let root = tempfile::tempdir().unwrap();
        let alpha = root.path().join("alpha");
        std::fs::create_dir_all(&alpha).unwrap();
        let key = profile_key(&alpha);
        assert_eq!(key.len(), 32);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        // The same directory reached through a non-canonical path agrees.
        assert_eq!(profile_key(&alpha.join("..").join("alpha")), key);
        assert_ne!(profile_key(&root.path().join("beta")), key);
        // The directory layout never reaches the keychain account.
        assert!(!profile_account(&alpha).contains("alpha"));
    }

    #[test]
    fn debug_never_prints_the_client_secret() {
        let secret = RedditAppSecret {
            client_id: "cid123".to_owned(),
            client_secret: "s3cr3t-value".to_owned(),
        };
        let debug = format!("{secret:?} {secret:#?}");
        assert!(debug.contains("cid123"), "{debug}");
        assert!(debug.contains(REDACTED), "{debug}");
        assert!(!debug.contains("s3cr3t-value"), "secret leaked: {debug}");
    }
}
