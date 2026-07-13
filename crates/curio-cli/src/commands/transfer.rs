//! Moving things across the boundary: save-to-destination, destination
//! registry, OPML import/export.

use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, anyhow};
use curio_core::export::ExportDisposition;
use curio_types::DestinationName;
use serde::Serialize;

use crate::app::App;
use crate::cli::{DestCommand, ImportFormat, OpmlCommand};
use crate::output::emit_json;
use crate::resolve;

#[derive(Debug, Serialize)]
struct SaveView {
    curio_id: String,
    destination: String,
    /// Note path relative to the destination root.
    path: String,
    checksum: String,
    /// `created` (article.saved) / `updated` (article.updated) /
    /// `unchanged` (idempotency hit, no event).
    disposition: &'static str,
}

/// What `curio save` was asked to do (single note or a bulk filter).
#[derive(Debug)]
pub(crate) struct SaveRequest {
    pub(crate) id: Option<String>,
    pub(crate) dest: Option<String>,
    pub(crate) all: bool,
    pub(crate) read_later: bool,
    pub(crate) starred: bool,
    pub(crate) feed: Option<String>,
    pub(crate) tag: Option<String>,
}

#[derive(Debug, Serialize)]
struct BulkSaveView {
    destination: String,
    created: u64,
    updated: u64,
    unchanged: u64,
    total: u64,
}

pub(crate) fn save(app: &App, request: SaveRequest) -> anyhow::Result<ExitCode> {
    let name = request
        .dest
        .clone()
        .or_else(|| app.config.settings.default_destination.clone())
        .context(
            "no destination — pass --dest NAME or set settings.default_destination in curio.toml",
        )?;
    let name: DestinationName = name
        .parse()
        .map_err(|err| anyhow!("invalid destination name: {err}"))?;
    match request.id {
        Some(id) => save_one(app, &id, &name),
        None => save_bulk(app, request, &name),
    }
}

/// Bulk export: `--all` or at least one filter flag (a bare `curio save`
/// must not silently export the whole library).
fn save_bulk(app: &App, request: SaveRequest, name: &DestinationName) -> anyhow::Result<ExitCode> {
    let filtered =
        request.read_later || request.starred || request.feed.is_some() || request.tag.is_some();
    if !request.all && !filtered {
        anyhow::bail!(
            "nothing selected — pass an article id, --all, or a filter \
             (--read-later / --starred / --feed / --tag)"
        );
    }
    let feed = request
        .feed
        .as_deref()
        .map(|reference| resolve::feed_by_ref(&app.core, reference))
        .transpose()?;
    let filter = curio_core::storage::ListArticles {
        feed_id: feed.map(|f| f.id),
        read_later: request.read_later.then_some(true),
        starred: request.starred.then_some(true),
        tag: request.tag,
        ..curio_core::storage::ListArticles::default()
    };
    let outcome = app.core.save_all_to_destination(&filter, name)?;
    if app.json {
        emit_json(&BulkSaveView {
            destination: name.to_string(),
            created: outcome.created,
            updated: outcome.updated,
            unchanged: outcome.unchanged,
            total: outcome.total(),
        })?;
    } else {
        println!(
            "exported {} note(s) to {:?} — {} created, {} updated, {} unchanged",
            outcome.total(),
            name.as_str(),
            outcome.created,
            outcome.updated,
            outcome.unchanged
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn save_one(app: &App, id: &str, name: &DestinationName) -> anyhow::Result<ExitCode> {
    let article = resolve::article_by_prefix(&app.core, id)?;
    let outcome = app.core.save_to_destination(article.id, name)?;
    let disposition = match outcome.disposition {
        ExportDisposition::Created => "created",
        ExportDisposition::Updated => "updated",
        ExportDisposition::Unchanged => "unchanged",
    };
    if app.json {
        emit_json(&SaveView {
            curio_id: article.curio_id.to_string(),
            destination: outcome.destination.to_string(),
            path: outcome.path,
            checksum: outcome.checksum.to_string(),
            disposition,
        })?;
    } else {
        println!(
            "{disposition} {} in destination {:?}",
            outcome.path,
            outcome.destination.as_str()
        );
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Serialize)]
struct DestView {
    name: String,
    root: String,
}

pub(crate) fn dest(app: &mut App, command: DestCommand) -> anyhow::Result<ExitCode> {
    match command {
        DestCommand::Add { name, path } => {
            let name: DestinationName = name
                .parse()
                .map_err(|err| anyhow!("invalid destination name: {err}"))?;
            let root = std::path::absolute(&path)
                .with_context(|| format!("resolving {}", path.display()))?;
            app.core.add_destination(name.clone(), root.clone())?;
            app.config
                .destinations
                .insert(name.to_string(), root.clone());
            app.save_config()?;
            if app.json {
                emit_json(&DestView {
                    name: name.to_string(),
                    root: root.display().to_string(),
                })?;
            } else {
                println!("destination {:?} → {}", name.as_str(), root.display());
            }
        }
        DestCommand::List => {
            let views: Vec<DestView> = app
                .core
                .destinations()
                .iter()
                .map(|d| DestView {
                    name: d.name.to_string(),
                    root: d.root.display().to_string(),
                })
                .collect();
            if app.json {
                emit_json(&views)?;
            } else if views.is_empty() {
                println!("no destinations — register one with `curio dest add <name> <path>`");
            } else {
                for view in &views {
                    println!("{}  {}", view.name, view.root);
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Serialize)]
struct OpmlImportView {
    added: usize,
    skipped: usize,
}

#[derive(Debug, Serialize)]
struct ImportView {
    feeds_added: usize,
    feeds_skipped: usize,
    articles_added: usize,
    articles_skipped: usize,
}

pub(crate) fn import(app: &App, file: &Path, format: ImportFormat) -> anyhow::Result<ExitCode> {
    let content =
        std::fs::read_to_string(file).with_context(|| format!("reading {}", file.display()))?;
    let outcome = app.core.import_file(format.into(), &content)?;
    if app.json {
        emit_json(&ImportView {
            feeds_added: outcome.feeds_added,
            feeds_skipped: outcome.feeds_skipped,
            articles_added: outcome.articles_added,
            articles_skipped: outcome.articles_skipped,
        })?;
    } else {
        println!(
            "imported {} feed(s) and {} article(s); skipped {} already present",
            outcome.feeds_added,
            outcome.articles_added,
            outcome.feeds_skipped + outcome.articles_skipped
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn opml(app: &App, command: OpmlCommand) -> anyhow::Result<ExitCode> {
    match command {
        OpmlCommand::Import { file } => {
            let xml = std::fs::read_to_string(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            let outcome = app.core.import_opml(&xml)?;
            if app.json {
                emit_json(&OpmlImportView {
                    added: outcome.added,
                    skipped: outcome.skipped,
                })?;
            } else {
                println!(
                    "imported {} feed(s), skipped {} already subscribed",
                    outcome.added, outcome.skipped
                );
            }
        }
        OpmlCommand::Export { file } => {
            let xml = app.core.export_opml()?;
            if file.as_os_str() == "-" {
                print!("{xml}");
            } else {
                std::fs::write(&file, &xml)
                    .with_context(|| format!("writing {}", file.display()))?;
                if app.json {
                    emit_json(&serde_json::json!({ "file": file.display().to_string() }))?;
                } else {
                    println!("exported subscriptions to {}", file.display());
                }
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}
