//! Output shaping: serializable views + printers.
//!
//! Every command prints either human lines or — with the global `--json`
//! flag — exactly one JSON document on stdout, so scripts and agents can
//! parse without scraping.

use std::collections::BTreeMap;

use curio_core::CoreHandle;
use curio_core::model::{Article, Feed};
use serde::Serialize;

/// Prints one pretty JSON document on stdout.
pub(crate) fn emit_json<T: Serialize>(value: &T) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// How many `curio_id` characters the human views print — enough to be
/// unique in any personal-scale profile, short enough to type.
pub(crate) const SHORT_ID_LEN: usize = 8;

/// The head of a `curio_id` string, as shown in listings.
pub(crate) fn short_id(curio_id: &str) -> String {
    curio_id.chars().take(SHORT_ID_LEN).collect()
}

/// A feed, as printed.
#[derive(Debug, Serialize)]
pub(crate) struct FeedView {
    pub(crate) id: i64,
    pub(crate) url: String,
    pub(crate) title: Option<String>,
    pub(crate) status: &'static str,
    pub(crate) allow_private_network: bool,
    pub(crate) added_at: String,
    pub(crate) last_fetched_at: Option<String>,
}

impl FeedView {
    pub(crate) fn new(feed: &Feed) -> Self {
        Self {
            id: feed.id.0,
            url: feed.url.clone(),
            title: feed.title.clone(),
            status: feed.status.as_str(),
            allow_private_network: feed.allow_private_network,
            added_at: feed.added_at.to_string(),
            last_fetched_at: feed.last_fetched_at.map(|t| t.to_string()),
        }
    }

    pub(crate) fn human_line(&self) -> String {
        let title = self.title.as_deref().unwrap_or("(untitled)");
        format!(
            "{:>4}  {:<10}  {}  {}",
            self.id, self.status, title, self.url
        )
    }
}

/// An article with its state projection and tags, as printed.
#[derive(Debug, Serialize)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "the state projection IS four booleans"
)]
pub(crate) struct ArticleView {
    /// The short id listings show — a typable `curio_id` prefix.
    pub(crate) id: String,
    pub(crate) curio_id: String,
    pub(crate) title: String,
    pub(crate) source: String,
    pub(crate) feed: Option<String>,
    pub(crate) feed_title: Option<String>,
    pub(crate) author: Option<String>,
    pub(crate) published: Option<String>,
    pub(crate) saved: String,
    pub(crate) read: bool,
    pub(crate) starred: bool,
    pub(crate) read_later: bool,
    pub(crate) archived: bool,
    pub(crate) tags: Vec<String>,
    pub(crate) word_count: Option<u32>,
}

impl ArticleView {
    /// Assembles the view: article row + state projection + tags + feed
    /// lookup (pass the map from [`feed_map`]).
    pub(crate) fn build(
        core: &CoreHandle,
        article: &Article,
        feeds: &BTreeMap<i64, Feed>,
    ) -> anyhow::Result<Self> {
        let state = core.article_state(article.id)?;
        let tags = core.article_tags(article.id)?;
        let feed = article.feed_id.and_then(|id| feeds.get(&id.0));
        let curio_id = article.curio_id.to_string();
        Ok(Self {
            id: short_id(&curio_id),
            curio_id,
            title: article.title.clone(),
            source: article.source_url.clone(),
            feed: feed.map(|f| f.url.clone()),
            feed_title: feed.and_then(|f| f.title.clone()),
            author: article.author.clone(),
            published: article.published_at.map(|t| t.to_string()),
            saved: article.saved_at.to_string(),
            read: state.read,
            starred: state.starred,
            read_later: state.read_later,
            archived: state.archived,
            tags,
            word_count: article.word_count,
        })
    }

    pub(crate) fn human_line(&self) -> String {
        let mut flags = String::new();
        flags.push(if self.read { ' ' } else { '\u{2022}' });
        flags.push(if self.starred { '*' } else { ' ' });
        flags.push(if self.read_later { 'L' } else { ' ' });
        flags.push(if self.archived { 'A' } else { ' ' });
        let feed = self
            .feed_title
            .as_deref()
            .or(self.feed.as_deref())
            .unwrap_or("(manual)");
        let tags = if self.tags.is_empty() {
            String::new()
        } else {
            format!("  [{}]", self.tags.join(","))
        };
        format!("{}  {}  {}  ({feed}){tags}", self.id, flags, self.title)
    }
}

/// Feed rows keyed by row id — the lookup [`ArticleView::build`] wants.
pub(crate) fn feed_map(core: &CoreHandle) -> anyhow::Result<BTreeMap<i64, Feed>> {
    Ok(core
        .list_feeds()?
        .into_iter()
        .map(|feed| (feed.id.0, feed))
        .collect())
}

/// Prints a slice of article views in the current output mode.
pub(crate) fn print_articles(views: &[ArticleView], json: bool) -> anyhow::Result<()> {
    if json {
        emit_json(&views)?;
    } else if views.is_empty() {
        println!("no articles");
    } else {
        for view in views {
            println!("{}", view.human_line());
        }
    }
    Ok(())
}
