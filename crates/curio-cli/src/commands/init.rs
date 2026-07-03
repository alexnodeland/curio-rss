//! `curio init` — scaffold the profile directory.

use std::process::ExitCode;

use serde::Serialize;

use crate::app::App;
use crate::config::CONFIG_FILE;
use crate::output::emit_json;

#[derive(Debug, Serialize)]
struct InitView {
    profile: String,
    config: String,
    db: String,
    events: String,
    /// `true` when this run wrote a fresh `curio.toml`.
    created: bool,
}

/// Opening the [`App`] already created the directory, database, and
/// events log; init's own job is the config file and telling the user
/// where everything lives.
pub(crate) fn run(app: &App) -> anyhow::Result<ExitCode> {
    let config_path = app.profile.join(CONFIG_FILE);
    let created = !config_path.exists();
    if created {
        app.save_config()?;
    }
    let view = InitView {
        profile: app.profile.display().to_string(),
        config: config_path.display().to_string(),
        db: app.profile.join("curio.db").display().to_string(),
        events: app
            .profile
            .join(".curio")
            .join("events")
            .display()
            .to_string(),
        created,
    };
    if app.json {
        emit_json(&view)?;
    } else {
        let verb = if created {
            "initialized"
        } else {
            "already initialized"
        };
        println!("{verb} profile at {}", view.profile);
        println!("  config  {}", view.config);
        println!("  db      {}", view.db);
        println!("  events  {}", view.events);
    }
    Ok(ExitCode::SUCCESS)
}
