//! `curio reddit …` — the optional BYO Reddit OAuth credentials (D15).
//!
//! Credentials live in the OS keychain; the runtime core is updated in
//! the same breath so the change takes effect without a restart. The
//! secret is write-only: `status` reports the client id and nothing
//! else, and `login` prefers reading the secret from stdin over a shell
//! argument that would land in history.

use std::io::Read as _;
use std::process::ExitCode;

use anyhow::Context as _;
use curio_core::enrich::reddit_auth::RedditApiConfig;
use curio_core::secrets::{self, RedditAppSecret};
use serde::Serialize;

use crate::app::App;
use crate::cli::RedditCommand;
use crate::output::emit_json;

pub(crate) fn run(app: &App, command: RedditCommand) -> anyhow::Result<ExitCode> {
    match command {
        RedditCommand::Login {
            client_id,
            client_secret,
        } => login(app, &client_id, client_secret),
        RedditCommand::Status => status(app),
        RedditCommand::Logout => logout(app),
    }
}

#[derive(Debug, Serialize)]
struct RedditStatusView {
    configured: bool,
    client_id: Option<String>,
}

fn login(app: &App, client_id: &str, client_secret: Option<String>) -> anyhow::Result<ExitCode> {
    let client_id = client_id.trim().to_owned();
    anyhow::ensure!(!client_id.is_empty(), "client id must not be empty");
    let client_secret = if let Some(secret) = client_secret {
        secret
    } else {
        eprintln!("reading the client secret from stdin…");
        let mut buffer = String::new();
        std::io::stdin()
            .read_to_string(&mut buffer)
            .context("reading the client secret from stdin")?;
        buffer
    }
    .trim()
    .to_owned();
    anyhow::ensure!(!client_secret.is_empty(), "client secret must not be empty");

    secrets::store_reddit_api(&RedditAppSecret {
        client_id: client_id.clone(),
        client_secret: client_secret.clone(),
    })?;
    app.core
        .set_reddit_api(Some(RedditApiConfig::new(client_id.clone(), client_secret)));
    if app.json {
        emit_json(&RedditStatusView {
            configured: true,
            client_id: Some(client_id),
        })?;
    } else {
        println!(
            "reddit API credentials stored in the OS keychain (client id {client_id}) — \
             enrichment now uses the authenticated 100/min tier"
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn status(app: &App) -> anyhow::Result<ExitCode> {
    let client_id = app.core.reddit_api_client_id();
    if app.json {
        emit_json(&RedditStatusView {
            configured: client_id.is_some(),
            client_id,
        })?;
    } else {
        match client_id {
            Some(client_id) => println!(
                "configured (client id {client_id}) — authenticated tier, ~100 requests/min"
            ),
            None => println!(
                "not configured — unauthenticated tier, ~10 requests/min \
                 (run `curio reddit login` to speed this up)"
            ),
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn logout(app: &App) -> anyhow::Result<ExitCode> {
    secrets::clear_reddit_api()?;
    app.core.set_reddit_api(None);
    if app.json {
        emit_json(&RedditStatusView {
            configured: false,
            client_id: None,
        })?;
    } else {
        println!("reddit API credentials removed — back to the unauthenticated tier");
    }
    Ok(ExitCode::SUCCESS)
}
