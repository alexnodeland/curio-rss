//! Destinations: the named-destination registry and note export.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::PoisonError;

use curio_types::{Destination, DestinationName};

use crate::content;
use crate::export::{self, ExportDisposition, ExportInput};
use crate::model::ArticleId;
use crate::storage::{ListArticles, Storage};

use super::{BulkSaveOutcome, CoreError, CoreHandle, SaveOutcome};

impl CoreHandle {
    /// Registers (or re-roots) a named destination and persists the
    /// registry. The only place a destination path ever enters.
    ///
    /// The registry is persisted *inside* the write-lock critical
    /// section, and the in-memory map only adopts the entry after the
    /// persist succeeds — so concurrent registrations cannot clobber
    /// each other's settings value, and a failed persist never leaves a
    /// destination that works now but vanishes on the next open.
    ///
    /// # Errors
    ///
    /// Filesystem or settings errors.
    pub fn add_destination(&self, name: DestinationName, root: PathBuf) -> Result<(), CoreError> {
        std::fs::create_dir_all(&root).map_err(|source| CoreError::Io {
            path: root.clone(),
            source,
        })?;
        let mut map = self.lock_destinations_mut();
        let mut next = map.clone();
        next.insert(name, root);
        self.persist_destinations(&next)?;
        *map = next;
        Ok(())
    }

    /// Unregisters a named destination and persists the registry.
    /// Exported notes under the old root are untouched — removal only
    /// forgets the name (the event stream already told their story).
    /// Same persist-then-adopt discipline as [`CoreHandle::add_destination`].
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`] if no destination is registered
    /// under `name`; settings errors.
    pub fn remove_destination(&self, name: &DestinationName) -> Result<(), CoreError> {
        let mut map = self.lock_destinations_mut();
        if !map.contains_key(name) {
            return Err(CoreError::UnknownDestination {
                name: name.to_string(),
            });
        }
        let mut next = map.clone();
        next.remove(name);
        self.persist_destinations(&next)?;
        *map = next;
        Ok(())
    }

    /// The registered destinations.
    #[must_use]
    pub fn destinations(&self) -> Vec<Destination> {
        self.lock_destinations()
            .iter()
            .map(|(name, root)| Destination {
                name: name.clone(),
                root: root.clone(),
            })
            .collect()
    }

    /// Exports an article to a named destination and emits the matching
    /// contract event: `article.saved` on first export,
    /// `article.updated` on a changed re-export, nothing on the
    /// idempotency hit.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`], [`CoreError::NotFound`],
    /// export/content/storage/emission errors.
    pub fn save_to_destination(
        &self,
        id: ArticleId,
        destination: &DestinationName,
    ) -> Result<SaveOutcome, CoreError> {
        let root = self
            .lock_destinations()
            .get(destination)
            .cloned()
            .ok_or_else(|| CoreError::UnknownDestination {
                name: destination.to_string(),
            })?;
        let article = self
            .storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })?;
        let feed = match article.feed_id {
            Some(feed_id) => self.storage.get_feed(feed_id)?,
            None => None,
        };
        let tags = self.storage.article_tags(id)?;
        let markdown = content::to_markdown(&article.content.html)?;

        let input = ExportInput {
            curio_id: article.curio_id,
            title: article.title.clone(),
            source: article.source_url.clone(),
            feed: feed.as_ref().map(|f| f.url.clone()),
            feed_title: feed.as_ref().and_then(|f| f.title.clone()),
            author: article.author.clone(),
            published: article.published_at,
            saved: article.saved_at,
            tags: tags.clone(),
            lang: article.lang.clone(),
            word_count: article.word_count,
            markdown,
        };
        let dest = Destination {
            name: destination.clone(),
            root,
        };
        // Ordering: note write → event intent → manifest write. The
        // intent is staged BEFORE the manifest commit — were it the
        // other way around, a crash between the two would make the
        // manifest's (curio_id, checksum) idempotency hit suppress the
        // event on every retry, losing article.saved/updated forever.
        let staged = export::stage_export_note(&dest, &input)?;
        let snapshot = article.snapshot(
            input.feed.clone(),
            input.feed_title.clone(),
            tags,
            destination.clone(),
            staged.outcome().path.clone(),
            staged.outcome().checksum,
        );
        match staged.outcome().disposition {
            ExportDisposition::Created => {
                self.storage.record_article_saved(snapshot)?;
            }
            ExportDisposition::Updated => {
                self.storage.record_article_updated(snapshot)?;
            }
            ExportDisposition::Unchanged => {}
        }
        let outcome = staged.commit()?;
        if outcome.disposition != ExportDisposition::Unchanged {
            self.emit()?;
        }
        Ok(SaveOutcome {
            destination: destination.clone(),
            path: outcome.path,
            checksum: outcome.checksum,
            disposition: outcome.disposition,
        })
    }

    /// Exports every article matching `filter` to a named destination —
    /// the export-everything path ("my library as markdown"). Each note
    /// goes through the same per-article pipeline as
    /// [`Self::save_to_destination`] (frontmatter, managed region,
    /// manifest idempotency, `article.saved`/`article.updated` events), so
    /// re-running a bulk export only rewrites what actually changed.
    /// `filter.before`/`limit` are pagination internals and are ignored.
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`]; the first per-article
    /// export/storage/emission error aborts the walk (already-written
    /// notes stay — each save is atomic).
    pub fn save_all_to_destination(
        &self,
        filter: &ListArticles,
        destination: &DestinationName,
    ) -> Result<BulkSaveOutcome, CoreError> {
        // Fail on an unknown destination before walking anything.
        if !self.lock_destinations().contains_key(destination) {
            return Err(CoreError::UnknownDestination {
                name: destination.to_string(),
            });
        }
        let mut page_filter = filter.clone();
        page_filter.before = None;
        page_filter.limit = 200;
        let mut outcome = BulkSaveOutcome::default();
        loop {
            let page = self.storage.list_articles(page_filter.clone())?;
            let page_len = page.len();
            for article in page {
                page_filter.before = Some(article.id);
                match self
                    .save_to_destination(article.id, destination)?
                    .disposition
                {
                    ExportDisposition::Created => outcome.created += 1,
                    ExportDisposition::Updated => outcome.updated += 1,
                    ExportDisposition::Unchanged => outcome.unchanged += 1,
                }
            }
            if page_len < page_filter.limit as usize {
                break;
            }
        }
        Ok(outcome)
    }

    /// Serializes the registry into the reserved settings key. Callers
    /// adopt the new map only after this succeeds — a failed persist must
    /// never leave an in-memory registry that vanishes on the next open.
    fn persist_destinations(
        &self,
        map: &BTreeMap<DestinationName, PathBuf>,
    ) -> Result<(), CoreError> {
        let serialized = serde_json::to_string(
            &map.iter()
                .map(|(n, p)| (n.to_string(), p.display().to_string()))
                .collect::<BTreeMap<_, _>>(),
        )?;
        self.storage.set_setting(DESTINATIONS_KEY, &serialized)?;
        Ok(())
    }

    fn lock_destinations(
        &self,
    ) -> std::sync::RwLockReadGuard<'_, BTreeMap<DestinationName, PathBuf>> {
        self.destinations
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn lock_destinations_mut(
        &self,
    ) -> std::sync::RwLockWriteGuard<'_, BTreeMap<DestinationName, PathBuf>> {
        self.destinations
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// Settings key holding the destination registry (JSON `name → root`).
pub(super) const DESTINATIONS_KEY: &str = "destinations";

pub(super) fn load_destinations(
    storage: &Storage,
) -> Result<BTreeMap<DestinationName, PathBuf>, CoreError> {
    let Some(raw) = storage.get_setting(DESTINATIONS_KEY)? else {
        return Ok(BTreeMap::new());
    };
    let parsed: BTreeMap<String, String> = serde_json::from_str(&raw)?;
    let mut map = BTreeMap::new();
    for (name, root) in parsed {
        match name.parse::<DestinationName>() {
            Ok(name) => {
                map.insert(name, PathBuf::from(root));
            }
            Err(err) => {
                tracing::warn!(%name, %err, "skipping invalid destination name in settings");
            }
        }
    }
    Ok(map)
}
