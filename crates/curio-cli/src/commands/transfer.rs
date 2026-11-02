//! Moving things across the boundary: save-to-destination, destination
//! registry, OPML import/export.

use std::io::IsTerminal as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, anyhow};
use curio_core::export::{ExportDisposition, write_file_atomic};
use curio_core::import::read_import_file;
use curio_core::{
    BulkSaveCancel, BulkSaveFailure, BulkSaveProgress, ImportKind, RejectReason, Rejected,
};
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
    /// Articles exported or already up to date (failed ones excluded).
    total: u64,
    /// Articles that could not be exported; the run carried on past them.
    failed: u64,
    /// The first failures in detail (`failed` stays exact past the cap).
    failures: Vec<BulkSaveFailureView>,
}

#[derive(Debug, Serialize)]
struct BulkSaveFailureView {
    curio_id: String,
    title: String,
    error: String,
}

impl From<&BulkSaveFailure> for BulkSaveFailureView {
    fn from(failure: &BulkSaveFailure) -> Self {
        Self {
            curio_id: failure.curio_id.to_string(),
            title: failure.title.clone(),
            error: failure.error.clone(),
        }
    }
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
    // The progress line is for a person watching: only on a terminal.
    let mut progress = std::io::stderr()
        .is_terminal()
        .then(|| ProgressLine::new(std::io::stderr()));
    let outcome =
        app.core
            .save_all_to_destination_with(&filter, name, &BulkSaveCancel::new(), |step| {
                if let Some(line) = progress.as_mut() {
                    line.update(step);
                }
            })?;
    if let Some(line) = progress.as_mut() {
        line.finish();
    }
    if app.json {
        emit_json(&BulkSaveView {
            destination: name.to_string(),
            created: outcome.created,
            updated: outcome.updated,
            unchanged: outcome.unchanged,
            total: outcome.total(),
            failed: outcome.failed,
            failures: outcome.failures.iter().map(Into::into).collect(),
        })?;
    } else {
        warn_failed(&outcome.failures, outcome.failed);
        let failed = if outcome.failed > 0 {
            format!(", {} failed", outcome.failed)
        } else {
            String::new()
        };
        println!(
            "exported {} note(s) to {:?} — {} created, {} updated, {} unchanged{failed}",
            outcome.total(),
            name.as_str(),
            outcome.created,
            outcome.updated,
            outcome.unchanged
        );
    }
    // A note that could not be exported is a failed run for a script,
    // though everything else was exported.
    Ok(if outcome.failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

/// Prints each article a bulk save skipped to stderr, so stdout stays the
/// summary.
fn warn_failed(failures: &[BulkSaveFailure], failed: u64) {
    for failure in failures {
        eprintln!("could not export {:?}: {}", failure.title, failure.error);
    }
    let unlisted = failed.saturating_sub(failures.len() as u64);
    if unlisted > 0 {
        eprintln!("…and {unlisted} more (see the log with RUST_LOG=warn)");
    }
}

/// A bulk save's one-line progress, redrawn in place on a terminal.
struct ProgressLine<W: std::io::Write> {
    out: W,
    drawn: bool,
}

impl<W: std::io::Write> ProgressLine<W> {
    fn new(out: W) -> Self {
        Self { out, drawn: false }
    }

    fn update(&mut self, step: BulkSaveProgress) {
        let failed = if step.failed > 0 {
            format!(", {} failed", step.failed)
        } else {
            String::new()
        };
        // A progress line that cannot be drawn is not worth failing the
        // export over.
        let _ = write!(
            self.out,
            "\r\x1b[2Kexporting… {}/{}{failed}",
            step.done, step.total
        );
        let _ = self.out.flush();
        self.drawn = true;
    }

    /// Clears the line, so the summary prints on a clean one.
    fn finish(&mut self) {
        if self.drawn {
            let _ = write!(self.out, "\r\x1b[2K");
            let _ = self.out.flush();
        }
    }
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
            // curio.toml now declares it: record it as seeded, so the
            // next run does not re-add it should another head remove it.
            crate::app::reconcile(&app.core, &app.config)?;
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
                .destinations()?
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
    /// Outlines dropped as unreadable or with an unusable URL.
    invalid: usize,
    /// Why outlines were dropped (capped; `invalid` stays exact).
    rejected: Vec<RejectedView>,
}

#[derive(Debug, Serialize)]
struct ImportView {
    feeds_added: usize,
    feeds_skipped: usize,
    feeds_invalid: usize,
    articles_added: usize,
    articles_skipped: usize,
    articles_invalid: usize,
    /// Why rows were dropped (capped; the counters stay exact).
    rejected: Vec<RejectedView>,
}

/// One dropped import row, for `--json`.
#[derive(Debug, Serialize)]
struct RejectedView {
    /// `feed` / `article`.
    kind: &'static str,
    /// `malformed` / `invalid_url` / `unsupported_scheme`.
    reason: &'static str,
    detail: String,
}

impl From<&Rejected> for RejectedView {
    fn from(rejected: &Rejected) -> Self {
        Self {
            kind: match rejected.kind {
                ImportKind::Feed => "feed",
                ImportKind::Article => "article",
            },
            reason: reason_label(rejected.reason),
            detail: rejected.detail.clone(),
        }
    }
}

fn reason_label(reason: RejectReason) -> &'static str {
    match reason {
        RejectReason::Malformed => "malformed",
        RejectReason::InvalidUrl => "invalid_url",
        RejectReason::UnsupportedScheme => "unsupported_scheme",
    }
}

/// Reads an import file the way every head does: size-capped, with its
/// encoding detected (Latin-1 OPML, UTF-16 CSV, ...).
fn read_import(file: &Path) -> anyhow::Result<String> {
    read_import_file(file).with_context(|| format!("reading {}", file.display()))
}

/// Prints each dropped row to stderr, so stdout stays the summary.
fn warn_rejected(rejected: &[Rejected]) {
    for row in rejected {
        eprintln!(
            "skipped invalid entry ({}): {}",
            reason_label(row.reason).replace('_', " "),
            row.detail
        );
    }
}

pub(crate) fn import(app: &App, file: &Path, format: ImportFormat) -> anyhow::Result<ExitCode> {
    let content = read_import(file)?;
    let outcome = app.core.import_file(format.into(), &content)?;
    if app.json {
        emit_json(&ImportView {
            feeds_added: outcome.feeds_added,
            feeds_skipped: outcome.feeds_skipped,
            feeds_invalid: outcome.feeds_invalid,
            articles_added: outcome.articles_added,
            articles_skipped: outcome.articles_skipped,
            articles_invalid: outcome.articles_invalid,
            rejected: outcome.rejected.iter().map(RejectedView::from).collect(),
        })?;
    } else {
        warn_rejected(&outcome.rejected);
        println!(
            "imported {} feed(s) and {} article(s); skipped {} already present, {} invalid",
            outcome.feeds_added,
            outcome.articles_added,
            outcome.feeds_skipped + outcome.articles_skipped,
            outcome.feeds_invalid + outcome.articles_invalid
        );
    }
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn opml(app: &App, command: OpmlCommand) -> anyhow::Result<ExitCode> {
    match command {
        OpmlCommand::Import { file } => {
            let xml = read_import(&file)?;
            let outcome = app.core.import_opml(&xml)?;
            if app.json {
                emit_json(&OpmlImportView {
                    added: outcome.added,
                    skipped: outcome.skipped,
                    invalid: outcome.invalid,
                    rejected: outcome.rejected.iter().map(RejectedView::from).collect(),
                })?;
            } else {
                warn_rejected(&outcome.rejected);
                println!(
                    "imported {} feed(s), skipped {} already subscribed, {} invalid",
                    outcome.added, outcome.skipped, outcome.invalid
                );
            }
        }
        OpmlCommand::Export { file } => {
            let xml = app.core.export_opml()?;
            if file.as_os_str() == "-" {
                print!("{xml}");
            } else {
                write_file_atomic(&file, xml.as_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_progress_line_redraws_in_place_and_clears_itself() {
        let mut line = ProgressLine::new(Vec::new());
        line.finish();
        assert!(line.out.is_empty(), "nothing drawn, nothing to clear");
        line.update(BulkSaveProgress {
            done: 200,
            total: 450,
            failed: 0,
        });
        line.update(BulkSaveProgress {
            done: 400,
            total: 450,
            failed: 2,
        });
        line.finish();
        assert_eq!(
            String::from_utf8(line.out).unwrap_or_default(),
            "\r\x1b[2Kexporting… 200/450\r\x1b[2Kexporting… 400/450, 2 failed\r\x1b[2K"
        );
    }
}
