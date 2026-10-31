//! Imports (OPML and refugee files) and OPML export.

use std::collections::HashSet;

use crate::feeds::OpmlFeed;
use crate::import::{self, ImportKind, ImportSource, ImportedItem, Rejected};
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
            invalid: outcome.feeds_invalid,
        })
    }

    /// Parses `content` as `source` and applies it — the one entry point
    /// for every refugee import (OPML, Pocket, Instapaper, Readwise).
    ///
    /// Lenient: an outline or row that cannot be read, or whose URL is
    /// unusable, is counted in the outcome's `*_invalid` counters (with its
    /// reason in `rejected`) instead of failing the file.
    ///
    /// # Errors
    ///
    /// [`CoreError::Import`] if the file cannot be parsed at all, plus the
    /// storage and emission errors of [`CoreHandle::import_items`].
    pub fn import_file(
        &self,
        source: ImportSource,
        content: &str,
    ) -> Result<ImportOutcome, CoreError> {
        let parsed = import::parse(source, content)?;
        // Rows the parser could not read lead the report; the URL check
        // below appends the rest.
        let mut outcome = ImportOutcome::default();
        for rejected in parsed.rejected {
            outcome.reject(rejected);
        }
        self.apply_items(&mut outcome, parsed.items)?;
        Ok(outcome)
    }

    /// Applies a neutral [`ImportedItem`] stream: feed items become
    /// subscriptions (each emits `feed.added`), article items become
    /// feedless read-later saves — upserted, flagged read-later, and
    /// tagged, each flagging/tagging staging its own event. Every URL is
    /// first normalized ([`import::normalize_url`]); one that is not a
    /// usable http(s) URL is counted as invalid, not imported. URLs already
    /// present (a subscribed feed, a previously imported article — compared
    /// in normalized form, so `HTTPS://Example.com` matches
    /// `https://example.com/`) are skipped, so re-importing the same export
    /// is idempotent. Every staged event flushes in a single emission at
    /// the end.
    ///
    /// # Errors
    ///
    /// Storage or emission errors.
    pub fn import_items(&self, items: Vec<ImportedItem>) -> Result<ImportOutcome, CoreError> {
        let mut outcome = ImportOutcome::default();
        self.apply_items(&mut outcome, items)?;
        Ok(outcome)
    }

    /// [`CoreHandle::import_items`] accumulating into an existing outcome.
    fn apply_items(
        &self,
        outcome: &mut ImportOutcome,
        items: Vec<ImportedItem>,
    ) -> Result<(), CoreError> {
        // Every subscription in normalized form: an existing feed stored as
        // `HTTPS://Example.com` still dedupes against `https://example.com/`.
        let mut known_feeds: HashSet<String> = self
            .storage
            .list_feeds()?
            .into_iter()
            .map(|feed| import::normalize_url(&feed.url).unwrap_or(feed.url))
            .collect();
        for mut item in items {
            item.url = match import::normalize_url(&item.url) {
                Ok(url) => url,
                Err(reason) => {
                    outcome.reject(Rejected {
                        kind: item.kind,
                        reason,
                        detail: item.url,
                    });
                    continue;
                }
            };
            match item.kind {
                ImportKind::Feed => {
                    if !known_feeds.insert(item.url.clone()) {
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
        self.emit();
        Ok(())
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
