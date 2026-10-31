//! Imports (OPML and refugee files) and OPML export.

use crate::feeds::OpmlFeed;
use crate::import::{self, ImportKind, ImportSource, ImportedItem};
use crate::model::{ArticleContent, NewArticle, NewFeed};

use super::{CoreError, CoreHandle, ImportOutcome, OpmlImportOutcome};

impl CoreHandle {
    /// Imports an OPML document, subscribing every feed not already
    /// known (each emits `feed.added` with its folder/category tags).
    ///
    /// Thin wrapper over [`CoreHandle::import_file`]; OPML carries only
    /// feeds, so the article counters are always zero here.
    ///
    /// # Errors
    ///
    /// OPML parse, storage or emission errors.
    pub fn import_opml(&self, xml: &str) -> Result<OpmlImportOutcome, CoreError> {
        let outcome = self.import_file(ImportSource::Opml, xml)?;
        Ok(OpmlImportOutcome {
            added: outcome.feeds_added,
            skipped: outcome.feeds_skipped,
        })
    }

    /// Parses `content` as `source` and applies it — the one entry point
    /// for every refugee import (OPML, Pocket, Instapaper, Readwise).
    ///
    /// # Errors
    ///
    /// [`CoreError::Import`] if the file cannot be parsed, plus the storage
    /// and emission errors of [`CoreHandle::import_items`].
    pub fn import_file(
        &self,
        source: ImportSource,
        content: &str,
    ) -> Result<ImportOutcome, CoreError> {
        self.import_items(import::parse(source, content)?)
    }

    /// Applies a neutral [`ImportedItem`] stream: feed items become
    /// subscriptions (each emits `feed.added`), article items become
    /// feedless read-later saves — upserted, flagged read-later, and
    /// tagged, each flagging/tagging staging its own event. URLs already
    /// present (a subscribed feed, a previously imported article) are
    /// skipped, so re-importing the same export is idempotent. Every
    /// staged event flushes in a single emission at the end.
    ///
    /// # Errors
    ///
    /// Storage or emission errors.
    pub fn import_items(&self, items: Vec<ImportedItem>) -> Result<ImportOutcome, CoreError> {
        let mut outcome = ImportOutcome::default();
        for item in items {
            match item.kind {
                ImportKind::Feed => {
                    if self.storage.get_feed_by_url(&item.url)?.is_some() {
                        outcome.feeds_skipped += 1;
                        continue;
                    }
                    self.storage.add_feed(NewFeed {
                        url: item.url,
                        title: item.title,
                        tags: item.tags,
                    })?;
                    outcome.feeds_added += 1;
                }
                ImportKind::Article => {
                    let title = item.title.unwrap_or_else(|| item.url.clone());
                    let key =
                        crate::feeds::dedupe_key(None, Some(&item.url), &title, item.saved_at);
                    if self.storage.article_id_by_dedupe_key(None, &key)?.is_some() {
                        outcome.articles_skipped += 1;
                        continue;
                    }
                    self.storage.upsert_articles(vec![NewArticle {
                        feed_id: None,
                        dedupe_key: key.clone(),
                        title,
                        source_url: item.url,
                        author: None,
                        published_at: item.saved_at,
                        content: ArticleContent::default(),
                        lang: None,
                        word_count: None,
                        source_updated_at: item.saved_at,
                        lead_image: None,
                    }])?;
                    // The row exists now; look its id back up to flag it.
                    // upsert_articles does not return ids (feed refresh
                    // never needs them), so the importer reads it back.
                    let id = self.storage.article_id_by_dedupe_key(None, &key)?.ok_or(
                        CoreError::NotFound {
                            entity: "imported article",
                        },
                    )?;
                    self.storage.add_read_later(id)?;
                    for tag in item.tags {
                        self.storage.tag_article(id, &tag)?;
                    }
                    outcome.articles_added += 1;
                }
            }
        }
        self.emit()?;
        Ok(outcome)
    }

    /// Exports every subscription as OPML 2.0.
    ///
    /// # Errors
    ///
    /// Storage or OPML write errors.
    pub fn export_opml(&self) -> Result<String, CoreError> {
        let feeds = self
            .storage
            .list_feeds()?
            .into_iter()
            .map(|feed| OpmlFeed {
                xml_url: feed.url,
                title: feed.title,
                html_url: feed.site_url,
                tags: feed.tags,
            })
            .collect::<Vec<_>>();
        Ok(crate::feeds::export_opml("Curio subscriptions", &feeds)?)
    }
}
