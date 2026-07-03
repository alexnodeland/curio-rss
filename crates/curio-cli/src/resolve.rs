//! Argument resolution: turning what a human typed into engine rows.

use anyhow::{Context as _, bail};
use curio_core::CoreHandle;
use curio_core::model::{Article, Feed, FeedId};
use curio_core::storage::ListArticles;

use crate::output::short_id;

const PAGE: u32 = 500;

/// Resolves a `curio_id` fragment (case-insensitive) to exactly one
/// article. A prefix, the short tail shown by listings, or any other
/// unique substring all work — `UUIDv7` prefixes are timestamp bits and
/// collide within an ingest batch, so pure-prefix matching would leave
/// same-batch articles unaddressable. Zero or multiple matches are
/// errors that tell the user what to do next.
pub(crate) fn article_by_prefix(core: &CoreHandle, fragment: &str) -> anyhow::Result<Article> {
    let needle = fragment.to_ascii_lowercase();
    if needle.is_empty() {
        bail!("empty article id — run `curio list` to find one");
    }
    let mut matches: Vec<Article> = Vec::new();
    let mut before = None;
    loop {
        let page = core.list_articles(ListArticles {
            feed_id: None,
            before,
            limit: PAGE,
        })?;
        let page_len = page.len();
        for article in page {
            before = Some(article.id);
            if article.curio_id.to_string().contains(&needle) {
                matches.push(article);
            }
        }
        if page_len < PAGE as usize {
            break;
        }
    }
    match matches.len() {
        0 => bail!("no article matches id {fragment:?} — run `curio list` to find one"),
        1 => matches.pop().context("unreachable: len checked"),
        n => {
            let ids: Vec<String> = matches
                .iter()
                .take(5)
                .map(|a| short_id(&a.curio_id.to_string()))
                .collect();
            bail!(
                "{n} articles match id {fragment:?} ({}, …) — give more characters",
                ids.join(", ")
            )
        }
    }
}

/// Resolves a feed reference — numeric row id, exact URL, or a unique
/// URL/title substring — to exactly one subscription.
pub(crate) fn feed_by_ref(core: &CoreHandle, reference: &str) -> anyhow::Result<Feed> {
    if let Ok(id) = reference.parse::<i64>() {
        if let Some(feed) = core.get_feed(FeedId(id))? {
            return Ok(feed);
        }
    }
    let feeds = core.list_feeds()?;
    if let Some(feed) = feeds.iter().find(|f| f.url == reference) {
        return Ok(feed.clone());
    }
    let matches: Vec<&Feed> = feeds
        .iter()
        .filter(|f| {
            f.url.contains(reference)
                || f.title
                    .as_deref()
                    .is_some_and(|title| title.contains(reference))
        })
        .collect();
    match matches.len() {
        0 => bail!("no feed matches {reference:?} — run `curio feed list`"),
        1 => Ok(matches[0].clone()),
        n => {
            let urls: Vec<&str> = matches.iter().take(5).map(|f| f.url.as_str()).collect();
            bail!(
                "{n} feeds match {reference:?} ({}, …) — use the id or full URL",
                urls.join(", ")
            )
        }
    }
}
