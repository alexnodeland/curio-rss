//! The optional BYO Reddit OAuth credentials (D15): stored in the OS
//! keychain, installed into the running core in the same breath. The
//! secret is strictly write-only across IPC — the status command
//! returns the client id and nothing else, and no command ever echoes
//! the secret back to the webview.

use std::sync::Arc;

use curio_core::CoreHandle;
use curio_core::enrich::reddit_auth::RedditApiConfig;
use curio_core::secrets::{self, RedditAppSecret, SecretsError};
use tauri::State;

use super::{SharedCore, run_blocking};
use crate::dto::RedditApiStatusDto;
use crate::error::{CommandError, ErrorCode};

/// Stores the user's Reddit app credentials (keychain) and switches the
/// engine to the authenticated 100/min tier immediately.
#[tauri::command]
#[specta::specta]
pub async fn set_reddit_api(
    core: State<'_, SharedCore>,
    client_id: String,
    client_secret: String,
) -> Result<RedditApiStatusDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || set_reddit_api_impl(&core, &client_id, &client_secret)).await
}

/// Removes the stored credentials and returns to the unauthenticated
/// tier. Idempotent.
#[tauri::command]
#[specta::specta]
pub async fn clear_reddit_api(
    core: State<'_, SharedCore>,
) -> Result<RedditApiStatusDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || clear_reddit_api_impl(&core)).await
}

/// Whether credentials are installed, and under which client id — never
/// the secret.
#[tauri::command]
#[specta::specta]
pub async fn get_reddit_api_status(
    core: State<'_, SharedCore>,
) -> Result<RedditApiStatusDto, CommandError> {
    let core = Arc::clone(core.inner());
    run_blocking(move || Ok(status_of(&core))).await
}

/// Startup: pull the keychain item (if any) into the running core.
/// Best-effort — a locked/absent keychain backend degrades to the
/// unauthenticated tier with a log line, never a failed launch.
pub fn load_credentials_at_startup(core: &CoreHandle) {
    match secrets::load_reddit_api() {
        Ok(Some(secret)) => core.set_reddit_api(Some(RedditApiConfig::new(
            secret.client_id,
            secret.client_secret,
        ))),
        Ok(None) => {}
        Err(err) => {
            tracing::warn!(%err, "keychain unavailable; reddit stays unauthenticated");
        }
    }
}

// ------------------------------------------------------------------ impls

fn set_reddit_api_impl(
    core: &CoreHandle,
    client_id: &str,
    client_secret: &str,
) -> Result<RedditApiStatusDto, CommandError> {
    let client_id = client_id.trim().to_owned();
    let client_secret = client_secret.trim().to_owned();
    if client_id.is_empty() || client_secret.is_empty() {
        return Err(CommandError::invalid_input(
            "both the client id and the client secret are required",
        ));
    }
    secrets::store_reddit_api(&RedditAppSecret {
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
    })
    .map_err(|err| map_secrets(&err))?;
    core.set_reddit_api(Some(RedditApiConfig::new(client_id, client_secret)));
    Ok(status_of(core))
}

fn clear_reddit_api_impl(core: &CoreHandle) -> Result<RedditApiStatusDto, CommandError> {
    secrets::clear_reddit_api().map_err(|err| map_secrets(&err))?;
    core.set_reddit_api(None);
    Ok(status_of(core))
}

fn status_of(core: &CoreHandle) -> RedditApiStatusDto {
    let client_id = core.reddit_api_client_id();
    RedditApiStatusDto {
        configured: client_id.is_some(),
        client_id,
    }
}

/// Keychain refusals are user-actionable prose (locked keychain, access
/// denied) — surface them verbatim.
fn map_secrets(err: &SecretsError) -> CommandError {
    CommandError::user(ErrorCode::Io, err.to_string(), true)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::commands::testutil::temp_core;
    use crate::error::ErrorKind;

    #[test]
    fn the_credential_round_trip_stays_secret_free_on_the_way_out() {
        curio_core::secrets::use_mock_store_for_tests();
        let (_dir, core) = temp_core();

        assert!(!status_of(&core).configured);

        let status = set_reddit_api_impl(&core, " cid123 ", " shh ").unwrap();
        assert!(status.configured);
        assert_eq!(status.client_id.as_deref(), Some("cid123"));

        // A restart-equivalent: the keychain item hydrates a fresh core.
        let (_dir2, fresh) = temp_core();
        load_credentials_at_startup(&fresh);
        assert_eq!(fresh.reddit_api_client_id().as_deref(), Some("cid123"));

        let cleared = clear_reddit_api_impl(&core).unwrap();
        assert!(!cleared.configured);
        assert!(cleared.client_id.is_none());

        let error = set_reddit_api_impl(&core, "cid", "   ").unwrap_err();
        assert_eq!(error.kind, ErrorKind::User);
    }
}
