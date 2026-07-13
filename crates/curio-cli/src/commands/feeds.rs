//! `curio feed …` and `curio fetch` — subscriptions and refresh.

use std::process::ExitCode;

use anyhow::Context as _;
use curio_core::RefreshOutcome;
use curio_core::model::NewFeed;
use serde::Serialize;

use crate::app::App;
use crate::cli::FeedCommand;
use crate::output::{FeedView, emit_json};
use crate::resolve;

pub(crate) fn run(app: &mut App, command: FeedCommand) -> anyhow::Result<ExitCode> {
    match command {
        FeedCommand::Add {
            url,
            tags,
            allow_private_network,
        } => add(app, url, tags, allow_private_network),
        FeedCommand::List => list(app),
        FeedCommand::Rm { feed } => remove(app, &feed),
        FeedCommand::FullText { feed, mode } => full_text(app, &feed, mode.as_bool()),
    }
}

/// `curio feed full-text <feed> <on|off>` — flips per-feed full-text mode.
fn full_text(app: &App, reference: &str, enabled: bool) -> anyhow::Result<ExitCode> {
    let feed = resolve::feed_by_ref(&app.core, reference)?;
    app.core.set_feed_full_text(feed.id, enabled)?;
    if app.json {
        emit_json(&serde_json::json!({
            "feed": feed.url,
            "fetch_full_text": enabled,
        }))?;
    } else {
        println!(
            "full-text {} for {}",
            if enabled { "on" } else { "off" },
            feed.url
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn add(
    app: &mut App,
    url: String,
    tags: Vec<String>,
    allow_private_network: bool,
) -> anyhow::Result<ExitCode> {
    let feed = app.core.add_feed(NewFeed {
        url: url.clone(),
        title: None,
        tags,
    })?;
    if allow_private_network {
        // Contract W1: the flag is explicit configuration — set it on the
        // subscription AND record it in curio.toml so the config file
        // stays the human-auditable source of every exemption.
        app.core.set_feed_allow_private_network(feed.id, true)?;
        app.config
            .feeds
            .entry(url)
            .or_default()
            .allow_private_network = true;
        app.save_config()?;
    }
    let feed = app
        .core
        .get_feed(feed.id)?
        .context("freshly added feed vanished")?;
    if app.json {
        emit_json(&FeedView::new(&feed))?;
    } else {
        println!("subscribed {} (id {})", feed.url, feed.id);
        if feed.allow_private_network {
            println!("  allow_private_network = true (W1 exemption, recorded in curio.toml)");
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn list(app: &App) -> anyhow::Result<ExitCode> {
    let views: Vec<FeedView> = app.core.list_feeds()?.iter().map(FeedView::new).collect();
    if app.json {
        emit_json(&views)?;
    } else if views.is_empty() {
        println!("no feeds — subscribe with `curio feed add <url>`");
    } else {
        for view in &views {
            println!("{}", view.human_line());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn remove(app: &mut App, reference: &str) -> anyhow::Result<ExitCode> {
    let feed = resolve::feed_by_ref(&app.core, reference)?;
    app.core.remove_feed(feed.id)?;
    if app.config.feeds.remove(&feed.url).is_some() {
        app.save_config()?;
    }
    if app.json {
        emit_json(&FeedView::new(&feed))?;
    } else {
        println!("unsubscribed {} (articles kept)", feed.url);
    }
    Ok(ExitCode::SUCCESS)
}

/// One refreshed feed, as printed.
#[derive(Debug, Serialize)]
struct FetchView {
    feed_id: i64,
    url: String,
    status: &'static str,
    http_status: Option<u16>,
    new_articles: u64,
    updated_articles: u64,
    error: Option<String>,
}

impl FetchView {
    fn new(outcome: &RefreshOutcome, url: String) -> Self {
        Self {
            feed_id: outcome.feed_id.0,
            url,
            status: outcome.status.as_str(),
            http_status: outcome.http_status,
            new_articles: outcome.new_articles,
            updated_articles: outcome.updated_articles,
            error: outcome.error.clone(),
        }
    }
}

pub(crate) fn fetch(app: &App, reference: Option<&str>) -> anyhow::Result<ExitCode> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the fetch runtime")?;
    let outcomes = match reference {
        Some(reference) => {
            let feed = resolve::feed_by_ref(&app.core, reference)?;
            vec![runtime.block_on(app.core.refresh_feed(feed.id))?]
        }
        None => runtime.block_on(app.core.refresh_all())?,
    };
    let feeds = crate::output::feed_map(&app.core)?;
    let views: Vec<FetchView> = outcomes
        .iter()
        .map(|outcome| {
            let url = feeds
                .get(&outcome.feed_id.0)
                .map_or_else(|| format!("feed #{}", outcome.feed_id), |f| f.url.clone());
            FetchView::new(outcome, url)
        })
        .collect();
    if app.json {
        emit_json(&views)?;
    } else if views.is_empty() {
        println!("no active feeds to fetch");
    } else {
        let mut total_new = 0;
        for view in &views {
            match &view.error {
                Some(error) => println!("{}  error — {error}", view.url),
                None => println!(
                    "{}  {} ({} new, {} updated)",
                    view.url, view.status, view.new_articles, view.updated_articles
                ),
            }
            total_new += view.new_articles;
        }
        println!(
            "{} feed(s) refreshed, {total_new} new article(s)",
            views.len()
        );
    }
    Ok(ExitCode::SUCCESS)
}
