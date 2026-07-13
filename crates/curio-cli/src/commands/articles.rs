//! Article commands: list / show / open / search and the state flips.

use std::process::ExitCode;

use anyhow::Context as _;
use curio_core::storage::ListArticles;
use serde::Serialize;

use crate::app::App;
use crate::cli::ListArgs;
use crate::output::{ArticleView, emit_json, feed_map, print_articles, short_id};
use crate::resolve;

const PAGE: u32 = 200;

pub(crate) fn list(app: &App, args: &ListArgs) -> anyhow::Result<ExitCode> {
    let feed = args
        .feed
        .as_deref()
        .map(|reference| resolve::feed_by_ref(&app.core, reference))
        .transpose()?;
    let feeds = feed_map(&app.core)?;
    let mut views = Vec::new();
    let mut before = None;
    'pages: loop {
        let page = app.core.list_articles(ListArticles {
            feed_id: feed.as_ref().map(|f| f.id),
            before,
            limit: PAGE,
            ..ListArticles::default()
        })?;
        let page_len = page.len();
        for article in page {
            before = Some(article.id);
            let view = ArticleView::build(&app.core, &article, &feeds)?;
            if matches_filters(&view, args) {
                views.push(view);
                if views.len() >= args.limit as usize {
                    break 'pages;
                }
            }
        }
        if page_len < PAGE as usize {
            break;
        }
    }
    print_articles(&views, app.json)?;
    Ok(ExitCode::SUCCESS)
}

fn matches_filters(view: &ArticleView, args: &ListArgs) -> bool {
    (!args.unread || !view.read)
        && (!args.starred || view.starred)
        && (!args.read_later || view.read_later)
        && args.tag.as_ref().is_none_or(|tag| view.tags.contains(tag))
}

pub(crate) fn search(app: &App, query: &str, limit: u32) -> anyhow::Result<ExitCode> {
    let feeds = feed_map(&app.core)?;
    let views = app
        .core
        .search_articles(query, limit)?
        .iter()
        .map(|article| ArticleView::build(&app.core, article, &feeds))
        .collect::<anyhow::Result<Vec<_>>>()?;
    print_articles(&views, app.json)?;
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Serialize)]
struct ShowView {
    #[serde(flatten)]
    article: ArticleView,
    markdown: String,
}

pub(crate) fn show(app: &App, id: &str) -> anyhow::Result<ExitCode> {
    let article = resolve::article_by_prefix(&app.core, id)?;
    let markdown = curio_core::content::to_markdown(&article.content.html)?;
    app.core.mark_read(article.id, true)?;
    let feeds = feed_map(&app.core)?;
    let view = ArticleView::build(&app.core, &article, &feeds)?;
    if app.json {
        emit_json(&ShowView {
            article: view,
            markdown,
        })?;
    } else {
        println!("# {}", view.title);
        println!();
        let mut meta = vec![view.source.clone()];
        if let Some(author) = &view.author {
            meta.push(author.clone());
        }
        if let Some(published) = &view.published {
            meta.push(published.clone());
        }
        println!("{}", meta.join(" · "));
        if !view.tags.is_empty() {
            println!("tags: {}", view.tags.join(", "));
        }
        println!("{}", "\u{2500}".repeat(72));
        println!("{markdown}");
    }
    Ok(ExitCode::SUCCESS)
}

#[derive(Debug, Serialize)]
struct OpenView {
    curio_id: String,
    url: String,
    browser: String,
}

/// Emits `article.opened` first (the record is the point), then hands the
/// source URL to `$BROWSER` (fallback: the platform opener).
pub(crate) fn open(app: &App, id: &str) -> anyhow::Result<ExitCode> {
    let article = resolve::article_by_prefix(&app.core, id)?;
    app.core.record_opened(article.id, None)?;
    app.core.mark_read(article.id, true)?;
    let browser = std::env::var("BROWSER").unwrap_or_else(|_| default_opener().to_owned());
    std::process::Command::new(&browser)
        .arg(&article.source_url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .with_context(|| format!("launching browser {browser:?}"))?;
    if app.json {
        emit_json(&OpenView {
            curio_id: article.curio_id.to_string(),
            url: article.source_url,
            browser,
        })?;
    } else {
        println!("opened {} in {browser}", article.source_url);
    }
    Ok(ExitCode::SUCCESS)
}

fn default_opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

#[derive(Debug, Serialize)]
struct ClipView {
    #[serde(flatten)]
    article: ArticleView,
    /// `false`: the URL was already in the library and was re-flagged.
    created: bool,
    /// Whether full-text content was fetched and stored by this clip.
    hydrated: bool,
}

/// `curio clip <url>` — the single-URL read-later save.
pub(crate) fn clip(app: &App, url: &str, tags: Vec<String>) -> anyhow::Result<ExitCode> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building the clip runtime")?;
    let saved = runtime.block_on(app.core.save_url(url, tags))?;
    let feeds = feed_map(&app.core)?;
    let view = ArticleView::build(&app.core, &saved.article, &feeds)?;
    if app.json {
        emit_json(&ClipView {
            article: view,
            created: saved.created,
            hydrated: saved.hydrated,
        })?;
    } else {
        let verb = if saved.created {
            "clipped"
        } else {
            "re-flagged"
        };
        println!(
            "{verb} {} — {} ({})",
            short_id(&saved.article.curio_id.to_string()),
            saved.article.title,
            if saved.hydrated {
                format!("{} words", saved.article.word_count.unwrap_or(0))
            } else if saved.created {
                "page unreachable — saved the bare link".to_owned()
            } else {
                "already in the library".to_owned()
            }
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// The six flag flips, one command each.
#[derive(Debug, Clone, Copy)]
pub(crate) enum StateAction {
    Star,
    Unstar,
    Later,
    Unlater,
    Archive,
    Unarchive,
}

impl StateAction {
    fn verb(self) -> &'static str {
        match self {
            Self::Star => "starred",
            Self::Unstar => "unstarred",
            Self::Later => "added to read-later",
            Self::Unlater => "removed from read-later",
            Self::Archive => "archived",
            Self::Unarchive => "unarchived",
        }
    }
}

#[derive(Debug, Serialize)]
struct StateView {
    curio_id: String,
    action: &'static str,
    /// `false` means the flip was a no-op (idempotent, no event emitted).
    changed: bool,
}

pub(crate) fn set_state(app: &App, id: &str, action: StateAction) -> anyhow::Result<ExitCode> {
    let article = resolve::article_by_prefix(&app.core, id)?;
    let changed = match action {
        StateAction::Star => app.core.set_starred(article.id, true)?,
        StateAction::Unstar => app.core.set_starred(article.id, false)?,
        StateAction::Later => app.core.set_read_later(article.id, true)?,
        StateAction::Unlater => app.core.set_read_later(article.id, false)?,
        StateAction::Archive => app.core.set_archived(article.id, true)?,
        StateAction::Unarchive => app.core.set_archived(article.id, false)?,
    };
    report_state(app, &article.curio_id.to_string(), action.verb(), changed)?;
    Ok(ExitCode::SUCCESS)
}

pub(crate) fn tag(app: &App, id: &str, tag: &str, add: bool) -> anyhow::Result<ExitCode> {
    let article = resolve::article_by_prefix(&app.core, id)?;
    let changed = if add {
        app.core.tag_article(article.id, tag)?
    } else {
        app.core.untag_article(article.id, tag)?
    };
    let verb = if add { "tagged" } else { "untagged" };
    report_state(app, &article.curio_id.to_string(), verb, changed)?;
    Ok(ExitCode::SUCCESS)
}

fn report_state(
    app: &App,
    curio_id: &str,
    action: &'static str,
    changed: bool,
) -> anyhow::Result<()> {
    if app.json {
        emit_json(&StateView {
            curio_id: curio_id.to_owned(),
            action,
            changed,
        })?;
    } else if changed {
        println!("{action} {}", short_id(curio_id));
    } else {
        println!("no change — {} already in that state", short_id(curio_id));
    }
    Ok(())
}
