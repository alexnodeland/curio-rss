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
//! item, so two profiles can hold different Reddit apps. The item is
//! keyed by an id the profile keeps in `.curio/profile-id` (minted on
//! first use), not by where the directory sits: moving or renaming a
//! profile keeps its credentials, and a logout can always find the item
//! it has to remove. A *copy* of a profile directory carries the same id
//! and therefore shares the item.
//!
//! Setting `CURIO_MOCK_KEYRING` (any value) swaps in `keyring`'s
//! in-memory mock store for the whole process — the test-only escape
//! hatch the CLI/desktop suites use to stay hermetic.

use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Once, OnceLock, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

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
/// The directory, under the profile, that holds the profile id file.
const PROFILE_ID_DIR: &str = ".curio";
/// The profile id file: the profile's stable keychain key.
const PROFILE_ID_FILE: &str = "profile-id";
/// Hex characters in a profile key (16 bytes).
const PROFILE_KEY_LEN: usize = 32;

/// Keychain failures, surfaced verbatim (they are already user-facing
/// prose: locked keychain, denied access, missing backend).
#[derive(Debug, thiserror::Error)]
pub enum SecretsError {
    /// The platform store refused the operation.
    #[error("keychain: {0}")]
    Keyring(#[from] keyring::Error),
    /// The stored item (or the profile id file) exists but is not
    /// Curio-shaped.
    #[error("keychain item is corrupt: {0}")]
    Corrupt(String),
    /// The profile's keychain id (`.curio/profile-id`) could not be read
    /// or created.
    #[error("profile keychain id {}: {source}", path.display())]
    ProfileId {
        /// The id file (or the directory that should hold it).
        path: PathBuf,
        /// The underlying filesystem error.
        source: io::Error,
    },
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
/// A legacy global item (see [`load_reddit_api`]) is left alone: other
/// profiles may not have copied it yet.
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses or the profile's
/// keychain id cannot be read or created.
pub fn store_reddit_api(profile_dir: &Path, secret: &RedditAppSecret) -> Result<(), SecretsError> {
    let entry = reddit_entry(&profile_account(profile_dir)?)?;
    write_secret(&entry, secret)
}

/// Loads the Reddit credentials of the profile at `profile_dir`; `None`
/// when nothing is stored.
///
/// **Migration:** earlier releases kept one global item (account
/// `reddit-api`) shared by every profile. When this profile has no item
/// of its own but the global one exists, the credentials are *copied*
/// into this profile's item and the global item is kept — the CLI's and
/// the desktop app's default profiles are different directories, and
/// each profile that loads after upgrading gets its own copy, just as
/// every profile used the global item before. Only an explicit
/// [`clear_reddit_api`] retires the global item. A failed copy is
/// logged and the credentials are still returned (the next load
/// retries).
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses, an item is not
/// Curio-shaped, or the profile's keychain id cannot be read or created.
pub fn load_reddit_api(profile_dir: &Path) -> Result<Option<RedditAppSecret>, SecretsError> {
    let scoped = reddit_entry(&profile_account(profile_dir)?)?;
    if let Some(secret) = read_secret(&scoped)? {
        return Ok(Some(secret));
    }
    let legacy = reddit_entry(REDDIT_ACCOUNT)?;
    let Some(secret) = read_secret(&legacy)? else {
        return Ok(None);
    };
    Ok(Some(adopt_legacy(&scoped, secret)))
}

/// Copies the legacy credentials into the profile's own item,
/// best-effort: a refused write is logged and the credentials are still
/// used for this session (the legacy item is untouched, so the next load
/// retries).
fn adopt_legacy(scoped: &Entry, secret: RedditAppSecret) -> RedditAppSecret {
    if let Err(err) = write_secret(scoped, &secret) {
        tracing::warn!(
            error = %err,
            "could not copy the legacy Reddit keychain item into this profile; \
             using it for this session"
        );
    }
    secret
}

/// Deletes the stored Reddit credentials of the profile at
/// `profile_dir`. Idempotent — deleting an absent item is a success.
///
/// The legacy global item (see [`load_reddit_api`]) is deleted too: it
/// would otherwise be copied straight back into this profile on the next
/// load, undoing the logout. Profiles that already copied it keep their
/// copy.
///
/// # Errors
///
/// [`SecretsError`] when the platform store refuses or the profile's
/// keychain id cannot be read or created.
pub fn clear_reddit_api(profile_dir: &Path) -> Result<(), SecretsError> {
    let scoped = reddit_entry(&profile_account(profile_dir)?)?;
    delete_secret(&scoped)?;
    let legacy = reddit_entry(REDDIT_ACCOUNT)?;
    delete_secret(&legacy)
}

/// The profile's stable keychain key: the 32-hex-character id stored in
/// `<profile>/.curio/profile-id`, minted (and the file created) on first
/// use. Stored rather than derived from the path, so it survives the
/// profile directory being moved or renamed; opaque rather than the path
/// itself, so the keychain's metadata never carries the user's directory
/// layout and the account name fits every platform store's length limit.
fn profile_key(profile_dir: &Path) -> Result<String, SecretsError> {
    let dir = profile_dir.join(PROFILE_ID_DIR);
    let path = dir.join(PROFILE_ID_FILE);
    match std::fs::read_to_string(&path) {
        Ok(contents) => return parse_key(&path, &contents),
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(source) => return Err(SecretsError::ProfileId { path, source }),
    }
    std::fs::create_dir_all(&dir).map_err(|source| SecretsError::ProfileId {
        path: dir.clone(),
        source,
    })?;
    claim_key(&path, &mint_key(profile_dir))
}

/// Writes `candidate` as the profile id unless one already exists
/// (`create_new`, so two heads opening a fresh profile at once agree on
/// the first writer's id); returns the id that won.
fn claim_key(path: &Path, candidate: &str) -> Result<String, SecretsError> {
    let io_error = |source| SecretsError::ProfileId {
        path: path.to_path_buf(),
        source,
    };
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(mut file) => {
            if let Err(source) = file.write_all(format!("{candidate}\n").as_bytes()) {
                // Never leave a truncated id behind for the next open.
                drop(file);
                let _ = std::fs::remove_file(path);
                return Err(io_error(source));
            }
            Ok(candidate.to_owned())
        }
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
            let contents = std::fs::read_to_string(path).map_err(io_error)?;
            parse_key(path, &contents)
        }
        Err(source) => Err(io_error(source)),
    }
}

/// Validates a stored profile id (32 hex characters, surrounding
/// whitespace ignored).
fn parse_key(path: &Path, contents: &str) -> Result<String, SecretsError> {
    let key = contents.trim();
    if key.len() == PROFILE_KEY_LEN && key.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(key.to_ascii_lowercase())
    } else {
        Err(SecretsError::Corrupt(format!(
            "{} does not hold a profile id",
            path.display()
        )))
    }
}

/// A fresh profile id: the first 16 bytes of a sha256 over the profile
/// path, the clock, the process id and a process-local counter — unique
/// per profile without pulling in a random-number dependency.
fn mint_key(profile_dir: &Path) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos());
    let mut hasher = Sha256::new();
    hasher.update(profile_dir.as_os_str().as_encoded_bytes());
    hasher.update(nanos.to_le_bytes());
    hasher.update(std::process::id().to_le_bytes());
    hasher.update(COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    hasher.finalize().iter().take(PROFILE_KEY_LEN / 2).fold(
        String::with_capacity(PROFILE_KEY_LEN),
        |mut hex, byte| {
            // Writing into a String cannot fail.
            let _ = write!(hex, "{byte:02x}");
            hex
        },
    )
}

/// The keychain account of a profile's Reddit item.
fn profile_account(profile_dir: &Path) -> Result<String, SecretsError> {
    Ok(format!("{REDDIT_ACCOUNT}/{}", profile_key(profile_dir)?))
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

    fn secret(client_id: &str, client_secret: &str) -> RedditAppSecret {
        RedditAppSecret {
            client_id: client_id.to_owned(),
            client_secret: client_secret.to_owned(),
        }
    }

    /// One test drives the whole mock-store story: the credential
    /// builder is process-global and every profile shares the one legacy
    /// account, so splitting these into separate tests would race.
    #[test]
    fn the_round_trip_is_scoped_per_profile_and_copies_the_global_item() {
        use_mock_store_for_tests();
        let root = tempfile::tempdir().unwrap();
        let profile = |name: &str| {
            let dir = root.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        };
        let (alpha, beta, gamma) = (profile("alpha"), profile("beta"), profile("gamma"));

        assert_eq!(load_reddit_api(&alpha).unwrap(), None, "empty store");
        clear_reddit_api(&alpha).unwrap(); // idempotent on absence

        let first = secret("cid123", "shh");
        store_reddit_api(&alpha, &first).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(first.clone()));
        // Another profile does not see alpha's credentials…
        assert_eq!(load_reddit_api(&beta).unwrap(), None);

        // …and holds different ones side by side.
        let other = secret("cid456", "hush");
        store_reddit_api(&beta, &other).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(first.clone()));
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(other.clone()));

        // Overwrite wins, per profile.
        let rotated = secret("cid123", "shh2");
        store_reddit_api(&alpha, &rotated).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(rotated.clone()));
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(other.clone()));

        // Moving a profile keeps its credentials: the key lives inside it.
        let moved = root.path().join("alpha-moved");
        std::fs::rename(&alpha, &moved).unwrap();
        assert_eq!(load_reddit_api(&moved).unwrap(), Some(rotated));
        std::fs::rename(&moved, &alpha).unwrap();

        // Logging one profile out leaves the other intact.
        clear_reddit_api(&alpha).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), None);
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(other));
        clear_reddit_api(&beta).unwrap();

        // The legacy global item is *copied* into each profile that
        // loads (the CLI's and the desktop's are different profiles) and
        // survives, so a profile that loads later still gets it.
        let legacy = secret("old", "older");
        let legacy_entry = reddit_entry(REDDIT_ACCOUNT).unwrap();
        write_secret(&legacy_entry, &legacy).unwrap();
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(legacy.clone()));
        assert_eq!(read_secret(&legacy_entry).unwrap(), Some(legacy.clone()));
        // A login on another profile neither destroys it…
        let fresh = secret("cid789", "new");
        store_reddit_api(&beta, &fresh).unwrap();
        assert_eq!(load_reddit_api(&beta).unwrap(), Some(fresh));
        assert_eq!(read_secret(&legacy_entry).unwrap(), Some(legacy.clone()));
        // …nor keeps a later profile from getting its copy.
        assert_eq!(load_reddit_api(&gamma).unwrap(), Some(legacy.clone()));

        // A logout retires the global item, so it can't be copied straight
        // back in — but profiles that already copied it keep theirs.
        clear_reddit_api(&alpha).unwrap();
        assert_eq!(read_secret(&legacy_entry).unwrap(), None);
        assert_eq!(load_reddit_api(&alpha).unwrap(), None);
        assert_eq!(load_reddit_api(&gamma).unwrap(), Some(legacy.clone()));
        clear_reddit_api(&beta).unwrap();
        clear_reddit_api(&gamma).unwrap();

        // A failed copy still hands the credentials over (the head runs
        // authenticated), and the next load retries the copy.
        write_secret(&legacy_entry, &legacy).unwrap();
        let alpha_entry = reddit_entry(&profile_account(&alpha).unwrap()).unwrap();
        let mock: &keyring::mock::MockCredential =
            alpha_entry.get_credential().downcast_ref().unwrap();
        // The mock fails the *next* call on the entry, so arm it right
        // before the copy rather than before the whole load.
        mock.set_error(keyring::Error::NoStorageAccess(
            io::Error::other("denied").into(),
        ));
        assert_eq!(adopt_legacy(&alpha_entry, legacy.clone()), legacy);
        assert_eq!(read_secret(&alpha_entry).unwrap(), None, "copy refused");
        assert_eq!(load_reddit_api(&alpha).unwrap(), Some(legacy.clone()));
        assert_eq!(read_secret(&alpha_entry).unwrap(), Some(legacy), "retried");
        // A refused scoped read is an error, not a fallback to the global item.
        mock.set_error(keyring::Error::NoStorageAccess(
            io::Error::other("denied").into(),
        ));
        assert!(matches!(
            load_reddit_api(&alpha),
            Err(SecretsError::Keyring(_))
        ));
        clear_reddit_api(&alpha).unwrap();

        // A corrupt item is reported, not silently treated as absent.
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
    fn the_profile_key_is_stable_bounded_and_distinct() {
        let root = tempfile::tempdir().unwrap();
        let alpha = root.path().join("alpha");
        std::fs::create_dir_all(&alpha).unwrap();
        let key = profile_key(&alpha).unwrap();
        assert_eq!(key.len(), PROFILE_KEY_LEN);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        // Minted once, then read back — through any path to the profile.
        assert_eq!(profile_key(&alpha).unwrap(), key);
        assert_eq!(profile_key(&alpha.join("..").join("alpha")).unwrap(), key);
        let stored = std::fs::read_to_string(alpha.join(".curio").join("profile-id")).unwrap();
        assert_eq!(stored.trim(), key);
        // Another profile (created on demand) gets its own.
        assert_ne!(profile_key(&root.path().join("beta")).unwrap(), key);
        // The directory layout never reaches the keychain account.
        assert!(!profile_account(&alpha).unwrap().contains("alpha"));
    }

    #[test]
    fn the_first_profile_id_written_wins() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("profile-id");
        let (a, b) = ("a".repeat(PROFILE_KEY_LEN), "b".repeat(PROFILE_KEY_LEN));
        assert_eq!(claim_key(&path, &a).unwrap(), a);
        assert_eq!(claim_key(&path, &b).unwrap(), a, "a racing head adopts it");
        // An unwritable location is an error, not a silently new key.
        let missing = root.path().join("absent").join("profile-id");
        assert!(matches!(
            claim_key(&missing, &a),
            Err(SecretsError::ProfileId { .. })
        ));
    }

    #[test]
    fn an_unusable_profile_id_is_an_error_never_a_new_key() {
        let root = tempfile::tempdir().unwrap();

        // Malformed contents.
        let garbled = root.path().join("garbled");
        std::fs::create_dir_all(garbled.join(".curio")).unwrap();
        std::fs::write(garbled.join(".curio").join("profile-id"), "nope\n").unwrap();
        assert!(matches!(
            profile_key(&garbled),
            Err(SecretsError::Corrupt(_))
        ));

        // Unreadable: the id path is a directory.
        let unreadable = root.path().join("unreadable");
        std::fs::create_dir_all(unreadable.join(".curio").join("profile-id")).unwrap();
        let err = profile_key(&unreadable).unwrap_err();
        assert!(matches!(err, SecretsError::ProfileId { .. }), "{err}");
        assert!(err.to_string().contains("profile-id"), "{err}");

        // Uncreatable: `.curio` is a file.
        let blocked = root.path().join("blocked");
        std::fs::create_dir_all(&blocked).unwrap();
        std::fs::write(blocked.join(".curio"), "").unwrap();
        assert!(matches!(
            profile_key(&blocked),
            Err(SecretsError::ProfileId { .. })
        ));
    }

    #[test]
    fn debug_never_prints_the_client_secret() {
        let secret = secret("cid123", "s3cr3t-value");
        let debug = format!("{secret:?} {secret:#?}");
        assert!(debug.contains("cid123"), "{debug}");
        assert!(debug.contains(REDACTED), "{debug}");
        assert!(!debug.contains("s3cr3t-value"), "secret leaked: {debug}");
    }
}
