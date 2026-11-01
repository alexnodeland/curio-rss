//! Destinations: the named-destination registry and note export.
//!
//! **The persisted registry is the only source of truth.** Several heads
//! (the desktop app, CLI runs, cron jobs) can have one profile open at
//! once, so no handle caches the registry: every read goes to the
//! settings table, and every change is a read-modify-write inside one
//! `IMMEDIATE` transaction (`Storage::edit_settings`). A change another
//! process made a moment ago is never overwritten with a stale copy.
//!
//! Three facade-owned settings keys hold it: the registry itself
//! (`name → root`), the names that completed an export (the
//! missing-destination check of [`CoreHandle::save_to_destination`]), and
//! the pairs a head configuration last seeded (see
//! [`CoreHandle::reconcile_destinations`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use curio_types::{Destination, DestinationName};

use crate::content;
use crate::export::{self, ExportDisposition, ExportInput};
use crate::model::ArticleId;
use crate::storage::{ListArticles, SettingsTx, Storage};

use super::{BulkSaveOutcome, CoreError, CoreHandle, SaveOutcome};

impl CoreHandle {
    /// Registers (or re-roots) a named destination and persists the
    /// registry. The only place a destination path ever enters.
    ///
    /// The registry is re-read and rewritten in one transaction, so
    /// concurrent registrations all survive, whether they come from other
    /// threads or from another process on the profile.
    ///
    /// (Re-)registering also starts the destination afresh for the
    /// missing-destination check of [`Self::save_to_destination`]: the
    /// recovery path after a vault really did move, or its `.curio/`
    /// folder was deliberately deleted.
    ///
    /// # Errors
    ///
    /// Filesystem or settings errors.
    pub fn add_destination(&self, name: DestinationName, root: PathBuf) -> Result<(), CoreError> {
        create_root(&root)?;
        self.storage.edit_settings(move |settings| {
            let mut registry = Registry::load(settings)?;
            registry.register(name, root);
            registry.store(settings)
        })
    }

    /// Applies a head's own destination configuration (the CLI's
    /// `curio.toml`, on every invocation). `declared` is the whole set the
    /// configuration names.
    ///
    /// **The configuration seeds destinations; it does not own them.**
    /// Each `name → root` pair is applied once, and the registry remembers
    /// that it was. Re-applying the same pair changes nothing: whatever
    /// another head did with the destination since (removed it, re-rooted
    /// it) stands, so a destination removed in the desktop app stays
    /// removed after a CLI run. A pair the configuration did not declare
    /// before (a new name, or a changed root) is applied. Dropping a name
    /// from the configuration forgets that it was seeded, so declaring it
    /// again later applies it afresh. `docs/design/decisions.md` (D16)
    /// records the rule.
    ///
    /// Applying a pair registers it exactly as [`Self::add_destination`]
    /// does, unless the registry already maps `name` to `root`: then the
    /// destination is kept as it is, exported flag included. A root is
    /// only ever created for a destination that has not exported yet, so a
    /// vanished or unmounted vault surfaces as
    /// [`ExportError::DestinationMissing`](crate::export::ExportError::DestinationMissing)
    /// on the next save instead of being recreated empty. Roots are
    /// created outside the settings transaction, so a slow mount never
    /// holds the database write lock.
    ///
    /// # Errors
    ///
    /// Filesystem or settings errors.
    pub fn reconcile_destinations(
        &self,
        declared: BTreeMap<DestinationName, PathBuf>,
    ) -> Result<(), CoreError> {
        // Roots are created outside any transaction, as in
        // `add_destination`: a slow or hung mount must not stall this
        // process's writer thread or hold the database write lock other
        // processes wait on. They are created up front from a plain read,
        // so a root that cannot be created fails the run before anything
        // is registered.
        let current = Registry::read(&self.storage)?;
        let mut created = BTreeSet::new();
        for root in current.roots_to_create(&declared) {
            create_root(root)?;
            created.insert(root.clone());
        }
        // The common case (every CLI command): nothing new was declared.
        // Settled with reads alone, so a CLI run does not take the
        // database write lock just to find there is nothing to write.
        if current.seeded == declared {
            return Ok(());
        }
        // Something to apply: decide again on fresh values inside the
        // transaction. Another head may have changed the registry since
        // the read above, so a root the fresh decision needs that was not
        // created up front is created once the transaction has committed.
        let to_create = self.storage.edit_settings(move |settings| {
            let mut registry = Registry::load(settings)?;
            let to_create: Vec<PathBuf> = registry
                .roots_to_create(&declared)
                .into_iter()
                .cloned()
                .collect();
            registry
                .seeded
                .retain(|name, _| declared.contains_key(name));
            for (name, root) in declared {
                let registered = registry.roots.get(&name) == Some(&root);
                let applied = registry.seeded.get(&name) == Some(&root);
                // Already what the configuration says: keep it as it is.
                // Otherwise a pair not applied before registers.
                if !registered && !applied {
                    registry.register(name.clone(), root.clone());
                }
                registry.seeded.insert(name, root);
            }
            registry.store(settings)?;
            Ok::<_, CoreError>(to_create)
        })?;
        for root in to_create.iter().filter(|root| !created.contains(*root)) {
            create_root(root)?;
        }
        Ok(())
    }

    /// Unregisters a named destination and persists the registry.
    /// Exported notes under the old root are untouched — removal only
    /// forgets the name (the event stream already told their story).
    /// Re-read and rewritten in one transaction, as in
    /// [`Self::add_destination`].
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownDestination`] if no destination is registered
    /// under `name`; settings errors.
    pub fn remove_destination(&self, name: &DestinationName) -> Result<(), CoreError> {
        let name = name.clone();
        self.storage.edit_settings(move |settings| {
            let mut registry = Registry::load(settings)?;
            if registry.roots.remove(&name).is_none() {
                return Err(CoreError::UnknownDestination {
                    name: name.to_string(),
                });
            }
            registry.exported.remove(&name);
            registry.store(settings)
        })
    }

    /// The registered destinations, as persisted now — changes another
    /// process made since this handle opened included.
    ///
    /// # Errors
    ///
    /// Storage or settings errors.
    pub fn destinations(&self) -> Result<Vec<Destination>, CoreError> {
        Ok(load_roots(&self.storage)?
            .into_iter()
            .map(|(name, root)| Destination { name, root })
            .collect())
    }

    /// Exports an article to a named destination and emits the matching
    /// contract event: `article.saved` on first export,
    /// `article.updated` on a changed re-export, nothing on the
    /// idempotency hit.
    ///
    /// Once a destination has completed an export, its root and its
    /// `.curio/` marker must still be there: a vault that moved, was
    /// renamed or is unmounted is refused with
    /// [`ExportError::DestinationMissing`](crate::export::ExportError::DestinationMissing)
    /// rather than recreated empty at the old path and fully re-exported.
    /// Registering the destination again (at its new location, or the same
    /// one) starts it afresh. A missing root is refused even before the
    /// first export — the export layer never creates it.
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
        let root = load_roots(&self.storage)?
            .remove(destination)
            .ok_or_else(|| CoreError::UnknownDestination {
                name: destination.to_string(),
            })?;
        let exported_before = load_exported(&self.storage)?.contains(destination);
        if exported_before {
            export::require_marker(&root)?;
        }
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
            root: root.clone(),
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
        if !exported_before {
            self.mark_exported(destination, root);
        }
        if outcome.disposition != ExportDisposition::Unchanged {
            self.emit();
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
    /// notes stay — each save is atomic). That includes
    /// [`crate::export::ExportError::NoteParse`] for a note the user
    /// edited into something that no longer parses, even when the article
    /// itself is unchanged.
    pub fn save_all_to_destination(
        &self,
        filter: &ListArticles,
        destination: &DestinationName,
    ) -> Result<BulkSaveOutcome, CoreError> {
        // Fail on an unknown destination before walking anything.
        if !load_roots(&self.storage)?.contains_key(destination) {
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

    /// Records that `name`, registered at `root`, has completed an
    /// export. Best effort: the export itself already succeeded, and a
    /// failed persist only means the missing-destination check stays off
    /// for this destination until the next successful export records it.
    fn mark_exported(&self, name: &DestinationName, root: PathBuf) {
        let recorded = self.storage.edit_settings({
            let name = name.clone();
            move |settings| {
                let mut registry = Registry::load(settings)?;
                // Removed or re-rooted by another head while this export
                // ran: the export says nothing about the destination now
                // registered under the name.
                if registry.roots.get(&name) != Some(&root) || !registry.exported.insert(name) {
                    return Ok(());
                }
                registry.store(settings)
            }
        });
        if let Err(err) = recorded {
            tracing::warn!(destination = %name, %err, "could not record the destination as exported");
        }
    }
}

/// Settings key holding the destination registry (JSON `name → root`).
pub(super) const DESTINATIONS_KEY: &str = "destinations";

/// Settings key holding the names of destinations that completed an
/// export since they were registered (JSON array of names).
pub(super) const EXPORTED_DESTINATIONS_KEY: &str = "destinations.exported";

/// Settings key holding the `name → root` pairs a head configuration last
/// seeded (JSON `name → root`; see [`CoreHandle::reconcile_destinations`]).
pub(super) const SEEDED_DESTINATIONS_KEY: &str = "destinations.seeded";

/// The persisted registry: the three facade-owned settings values.
struct Registry {
    roots: BTreeMap<DestinationName, PathBuf>,
    exported: BTreeSet<DestinationName>,
    seeded: BTreeMap<DestinationName, PathBuf>,
}

impl Registry {
    /// Loads it inside an edit transaction.
    fn load(settings: &SettingsTx<'_>) -> Result<Self, CoreError> {
        Ok(Self {
            roots: parse_roots(settings.get(DESTINATIONS_KEY)?.as_deref())?,
            exported: parse_names(settings.get(EXPORTED_DESTINATIONS_KEY)?.as_deref())?,
            seeded: parse_roots(settings.get(SEEDED_DESTINATIONS_KEY)?.as_deref())?,
        })
    }

    /// Reads it outside any transaction — a hint only: the three values
    /// are read one after the other, so another process can change the
    /// registry in between.
    fn read(storage: &Storage) -> Result<Self, CoreError> {
        Ok(Self {
            roots: load_roots(storage)?,
            exported: load_exported(storage)?,
            seeded: parse_roots(storage.get_setting(SEEDED_DESTINATIONS_KEY)?.as_deref())?,
        })
    }

    /// The roots of `declared` that reconciling it needs on disk: a
    /// pair not yet applied (it is about to register, as
    /// `add_destination` would), and a registered destination that has
    /// not exported yet. An exported destination's root is never created,
    /// so a vanished vault is refused rather than recreated empty.
    fn roots_to_create<'a>(
        &self,
        declared: &'a BTreeMap<DestinationName, PathBuf>,
    ) -> Vec<&'a PathBuf> {
        declared
            .iter()
            .filter(|(name, root)| {
                if self.roots.get(*name) == Some(*root) {
                    !self.exported.contains(*name)
                } else {
                    self.seeded.get(*name) != Some(*root)
                }
            })
            .map(|(_, root)| root)
            .collect()
    }

    /// (Re-)registers `name` at `root`, starting it afresh for the
    /// missing-destination check.
    fn register(&mut self, name: DestinationName, root: PathBuf) {
        self.exported.remove(&name);
        self.roots.insert(name, root);
    }

    fn store(&self, settings: &SettingsTx<'_>) -> Result<(), CoreError> {
        settings.set(DESTINATIONS_KEY, &serialize_roots(&self.roots)?)?;
        settings.set(EXPORTED_DESTINATIONS_KEY, &serialize_names(&self.exported)?)?;
        settings.set(SEEDED_DESTINATIONS_KEY, &serialize_roots(&self.seeded)?)?;
        Ok(())
    }
}

/// Creates a destination root (and its parents) if absent.
fn create_root(root: &Path) -> Result<(), CoreError> {
    std::fs::create_dir_all(root).map_err(|source| CoreError::Io {
        path: root.to_path_buf(),
        source,
    })
}

fn load_roots(storage: &Storage) -> Result<BTreeMap<DestinationName, PathBuf>, CoreError> {
    parse_roots(storage.get_setting(DESTINATIONS_KEY)?.as_deref())
}

fn load_exported(storage: &Storage) -> Result<BTreeSet<DestinationName>, CoreError> {
    parse_names(storage.get_setting(EXPORTED_DESTINATIONS_KEY)?.as_deref())
}

/// Parses a JSON `name → root` map; an absent key is an empty map.
fn parse_roots(raw: Option<&str>) -> Result<BTreeMap<DestinationName, PathBuf>, CoreError> {
    let Some(raw) = raw else {
        return Ok(BTreeMap::new());
    };
    let parsed: BTreeMap<String, String> = serde_json::from_str(raw)?;
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

fn serialize_roots(map: &BTreeMap<DestinationName, PathBuf>) -> Result<String, CoreError> {
    Ok(serde_json::to_string(
        &map.iter()
            .map(|(name, root)| (name.to_string(), root.display().to_string()))
            .collect::<BTreeMap<_, _>>(),
    )?)
}

/// Parses a JSON array of names; an absent key is an empty set.
fn parse_names(raw: Option<&str>) -> Result<BTreeSet<DestinationName>, CoreError> {
    let Some(raw) = raw else {
        return Ok(BTreeSet::new());
    };
    let names: Vec<String> = serde_json::from_str(raw)?;
    Ok(names.iter().filter_map(|name| name.parse().ok()).collect())
}

fn serialize_names(set: &BTreeSet<DestinationName>) -> Result<String, CoreError> {
    Ok(serde_json::to_string(
        &set.iter().map(ToString::to_string).collect::<Vec<_>>(),
    )?)
}
