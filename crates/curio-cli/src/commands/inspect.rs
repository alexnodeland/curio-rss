//! Introspection: `curio events tail` and `curio doctor`.

use std::process::ExitCode;

use curio_core::events::read_all;
use curio_core::storage::Storage;
use serde::Serialize;

use crate::app::App;
use crate::cli::EventsCommand;
use crate::output::emit_json;

pub(crate) fn events(app: &App, command: &EventsCommand) -> anyhow::Result<ExitCode> {
    let EventsCommand::Tail { n } = command;
    let dir = app.profile.join(".curio").join("events");
    let all = read_all(&dir)?;
    let start = all.len().saturating_sub(*n);
    let tail = &all[start..];
    if app.json {
        emit_json(&tail)?;
    } else if tail.is_empty() {
        println!("no events");
    } else {
        for envelope in tail {
            let payload = serde_json::to_value(envelope)?
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            println!(
                "{}  {:<26}  {}",
                envelope.ts,
                envelope.event.event_type(),
                serde_json::to_string(&payload)?
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Serialize)]
struct Check {
    name: &'static str,
    ok: bool,
    detail: String,
}

impl Check {
    fn ok(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
        }
    }

    fn from_result(name: &'static str, result: Result<String, String>) -> Self {
        match result {
            Ok(detail) => Self::ok(name, detail),
            Err(detail) => Self {
                name,
                ok: false,
                detail,
            },
        }
    }
}

#[derive(Debug, Serialize)]
struct DoctorView {
    ok: bool,
    checks: Vec<Check>,
}

/// Health probes over the three durable surfaces: the database, the FTS
/// index, and the events log.
pub(crate) fn doctor(app: &App) -> anyhow::Result<ExitCode> {
    let storage = app.core.storage();
    let mut checks = vec![
        Check::ok("profile", app.profile.display().to_string()),
        schema_check(storage),
        Check::from_result(
            "db-integrity",
            storage
                .integrity_check()
                .map(|()| "PRAGMA integrity_check ok".to_owned())
                .map_err(|err| err.to_string()),
        ),
        Check::from_result(
            "fts-sync",
            storage
                .fts_integrity_check()
                .map(|()| "FTS5 index matches the articles table".to_owned())
                .map_err(|err| err.to_string()),
        ),
        events_check(app),
        gitignore_check(app),
        intents_check(storage),
    ];
    match storage.count_articles() {
        Ok(count) => checks.push(Check::ok("articles", format!("{count} stored"))),
        Err(err) => checks.push(Check::from_result("articles", Err(err.to_string()))),
    }

    let ok = checks.iter().all(|check| check.ok);
    if app.json {
        emit_json(&DoctorView { ok, checks })?;
    } else {
        for check in &checks {
            let mark = if check.ok { "ok  " } else { "FAIL" };
            println!("{mark}  {:<14}  {}", check.name, check.detail);
        }
    }
    Ok(if ok {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

fn schema_check(storage: &Storage) -> Check {
    Check::from_result(
        "schema",
        match storage.db_schema_version() {
            Ok(found) if found == Storage::schema_version() => Ok(format!("v{found}")),
            Ok(found) => Err(format!(
                "database at v{found}, this build expects v{}",
                Storage::schema_version()
            )),
            Err(err) => Err(err.to_string()),
        },
    )
}

fn events_check(app: &App) -> Check {
    let dir = app.profile.join(".curio").join("events");
    Check::from_result(
        "events-log",
        match read_all(&dir) {
            Ok(events) => Ok(format!("{} event(s) parse cleanly", events.len())),
            Err(err) => Err(err.to_string()),
        },
    )
}

/// The contract says the events log is never committed to git; the
/// profile's `.curio/.gitignore` is the mechanism.
fn gitignore_check(app: &App) -> Check {
    let path = app.profile.join(".curio").join(".gitignore");
    Check::from_result(
        "events-gitignore",
        match std::fs::read_to_string(&path) {
            Ok(content) if content.lines().any(|line| line.trim() == "events/") => {
                Ok(format!("{} ignores events/", path.display()))
            }
            Ok(_) => Err(format!("{} does not ignore events/", path.display())),
            Err(err) => Err(format!("{}: {err}", path.display())),
        },
    )
}

/// Staged-but-unflushed event intents mean a crash interrupted emission;
/// the next open replays them, so a healthy handle shows zero.
fn intents_check(storage: &Storage) -> Check {
    Check::from_result(
        "event-intents",
        match storage.pending_intents() {
            Ok(pending) if pending.is_empty() => Ok("no staged intents".to_owned()),
            Ok(pending) => Err(format!(
                "{} staged intent(s) not yet flushed",
                pending.len()
            )),
            Err(err) => Err(err.to_string()),
        },
    )
}
