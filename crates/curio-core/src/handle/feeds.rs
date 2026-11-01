//! Feed subscriptions: add, remove, list, lookup, per-feed settings,
//! lifecycle status, fetch-filled metadata, ordering, and the fetch log.

use std::sync::PoisonError;

use crate::feeds::{DiscoverError, DiscoverOptions, Discovery, Resolution};
use crate::model::{Feed, FeedId, FeedStatus, FetchRecord, NewFeed};

use super::{CoreError, CoreHandle};

impl CoreHandle {
    /// Autodiscovers the feeds behind what a user typed — a bare domain, a
    /// homepage, a feed URL, or a page a rewrite rule knows — through the
    /// handle's policed client: every candidate returned is validated
    /// (fetched and parsed). See [`crate::feeds::discover()`]. No storage
    /// write and no event: subscribing is a separate [`Self::add_feed`].
    ///
    /// # Errors
    ///
    /// [`DiscoverError`] for input that is not a URL, or a page that could
    /// not be fetched. Finding no feed is `Ok` with empty `feeds`.
    pub async fn discover_feeds(
        &self,
        input: &str,
        options: DiscoverOptions,
    ) -> Result<Discovery, DiscoverError> {
        crate::feeds::discover(&self.client, input, options).await
    }

    /// Resolves what a user typed to the validated feed URL a subscribe
    /// should store — the shared validating subscribe path of every head
    /// (CLI `feed add`, the desktop's Subscribe): discovery, then its best
    /// candidate. See [`crate::feeds::resolve()`]. No storage write: pass
    /// the result to [`Self::add_feed`].
    ///
    /// # Errors
    ///
    /// Every [`Self::discover_feeds`] error, plus [`DiscoverError::NoFeed`]
    /// when the page answered but offers no feed.
    pub async fn resolve_feed_url(
        &self,
        input: &str,
        options: DiscoverOptions,
    ) -> Result<Resolution, DiscoverError> {
        crate::feeds::resolve(&self.client, input, options).await
    }

    /// Subscribes to a feed URL as given; emits `feed.added` (with tags).
    /// It neither normalizes nor validates: a head subscribing what a user
    /// typed resolves it with [`Self::resolve_feed_url`] first (or, on an
    /// explicit "subscribe anyway", [`crate::feeds::normalize_input`]).
    ///
    /// # Errors
    ///
    /// Storage (including duplicate URL) or emission errors.
    pub fn add_feed(&self, new: NewFeed) -> Result<Feed, CoreError> {
        let (feed, _envelope) = self.storage.add_feed(new)?;
        self.emit();
        Ok(feed)
    }

    /// Unsubscribes a feed; emits the `feed.removed` negation. Articles
    /// keep their rows.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn remove_feed(&self, id: FeedId) -> Result<(), CoreError> {
        self.storage.remove_feed(id)?;
        self.refresh_locks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        // Its refresh lock file goes too, best effort. Safe even while
        // another process holds it: a lock file deleted under a holder is
        // re-created, not trusted (see `crate::lock`). A refresh of this
        // feed already under way in another process can re-create it
        // after this delete (it only finds the feed gone once it holds
        // the lock), leaving one empty, gitignored lock file behind.
        let lock_file = super::refresh::feed_lock_path(&self.profile_dir, id);
        if let Err(err) = std::fs::remove_file(&lock_file)
            && err.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(path = %lock_file.display(), %err, "could not delete a feed's refresh lock file");
        }
        self.emit();
        Ok(())
    }

    /// Every subscription.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_feeds(&self) -> Result<Vec<Feed>, CoreError> {
        Ok(self.storage.list_feeds()?)
    }

    /// One feed by id.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_feed(&self, id: FeedId) -> Result<Option<Feed>, CoreError> {
        Ok(self.storage.get_feed(id)?)
    }

    /// Contract W1: flips the per-feed private-network exemption. This
    /// is the *only* path to the flag — explicit configuration, never
    /// feed content.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn set_feed_allow_private_network(&self, id: FeedId, allow: bool) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_allow_private_network(id, allow)?)
    }

    /// Flips a feed's full-text mode: when on, every refresh hydrates the
    /// feed's *new* articles from their source pages (readability-extract
    /// through the policed client) — the upgrade for feeds that ship only
    /// excerpts. DB-local, no event: how a feed's content is fetched is a
    /// local reading preference, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed.
    pub fn set_feed_full_text(&self, id: FeedId, enabled: bool) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_full_text(id, enabled)?)
    }

    /// Replaces a feed's tags — the move-to-folder / re-tag path (folders
    /// are a `/`-path-tag rendering in the head). A wholesale overwrite that
    /// is DB-local, like the feed-metadata edit: it stages no event, since
    /// consumers key feeds by URL and folder membership is a local
    /// reading-organization concern, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_tags(&self, id: FeedId, tags: Vec<String>) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_tags(id, tags)?)
    }

    /// Renames a feed — an unconditional title overwrite (an empty title
    /// clears it, falling back to the URL). DB-local, no event: the display
    /// title is not part of the published contract, and consumers key feeds
    /// by URL. This is the user-rename counterpart to the fetch-fill
    /// `update_feed_metadata`, which can only fill a NULL title.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_title(&self, id: FeedId, title: Option<String>) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_title(id, title)?)
    }

    /// Overwrites a feed's site URL and description — the edit-feed modal's
    /// user-edit path. DB-local, no contract event (the head's command layer
    /// emits the UI `FeedsChanged`): site URL / description are not part of
    /// the published contract, and consumers key feeds by URL. This is the
    /// user-overwrite counterpart to the fetch-fill `update_feed_metadata`,
    /// which only fills a NULL field so a refresh never clobbers a human edit.
    /// An empty value clears the field. The display title stays on
    /// [`Self::set_feed_title`].
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown feed; storage errors.
    pub fn set_feed_metadata(
        &self,
        id: FeedId,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_metadata(id, site_url, description)?)
    }

    /// Looks a feed up by its subscription URL (exact match).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_feed_by_url(&self, url: &str) -> Result<Option<Feed>, CoreError> {
        Ok(self.storage.get_feed_by_url(url)?)
    }

    /// Sets a feed's lifecycle status (pause, dead-on-410, reactivate).
    /// DB-local, no contract event: feed status is a local scheduling
    /// concern, not part of the published contract.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `NotFound` for an unknown feed;
    /// storage errors.
    pub fn set_feed_status(&self, id: FeedId, status: FeedStatus) -> Result<(), CoreError> {
        Ok(self.storage.set_feed_status(id, status)?)
    }

    /// Fills in feed metadata learned from a fetch, without clobbering
    /// anything already set (human edits win over parsed values) — the
    /// fetch-fill counterpart to [`Self::set_feed_metadata`]. DB-local, no
    /// contract event.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `NotFound` for an unknown feed;
    /// storage errors.
    pub fn update_feed_metadata(
        &self,
        id: FeedId,
        title: Option<String>,
        site_url: Option<String>,
        description: Option<String>,
    ) -> Result<(), CoreError> {
        Ok(self
            .storage
            .update_feed_metadata(id, title, site_url, description)?)
    }

    /// The most recent fetch attempts for a feed (the `fetch_log`),
    /// newest first.
    ///
    /// # Errors
    ///
    /// Database or stored-value corruption errors.
    pub fn recent_fetches(&self, id: FeedId, limit: u32) -> Result<Vec<FetchRecord>, CoreError> {
        Ok(self.storage.recent_fetches(id, limit)?)
    }

    /// Rewrites the sidebar feed order to `ordered` (drag-to-reorder). The
    /// caller passes the complete new sequence. DB-local, no event: feed
    /// ordering is a local reading-organization preference, not part of the
    /// published contract.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn reorder_feeds(&self, ordered: &[FeedId]) -> Result<(), CoreError> {
        Ok(self.storage.reorder_feeds(ordered)?)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::CoreOptions;
    use crate::fetch::FetchConfig;

    const RSS: &str =
        r#"<?xml version="1.0"?><rss version="2.0"><channel><title>Blog</title></channel></rss>"#;

    /// A handle whose policed client trusts the loopback fixture server.
    fn core_trusting_localhost(profile: &std::path::Path) -> CoreHandle {
        CoreHandle::open_with(
            profile,
            CoreOptions {
                fetch: FetchConfig {
                    politeness_delay: Duration::ZERO,
                    trusted_addrs: HashSet::from([IpAddr::V4(Ipv4Addr::LOCALHOST)]),
                    ..FetchConfig::default()
                },
                ..CoreOptions::default()
            },
        )
        .unwrap()
    }

    /// The handle's validating subscribe path resolves a homepage to the
    /// feed it declares — through the handle's own policed client — and
    /// writes nothing until `add_feed`.
    #[tokio::test]
    async fn resolve_feed_url_finds_the_declared_feed_without_subscribing() {
        let server = MockServer::start().await;
        let page = format!(
            r#"<link rel="alternate" type="application/rss+xml" href="{}/feed.xml">"#,
            server.uri()
        );
        for (at, body) in [("/", page.as_str()), ("/feed.xml", RSS)] {
            Mock::given(method("GET"))
                .and(path(at))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
        }
        let profile = tempfile::tempdir().unwrap();
        let core = core_trusting_localhost(profile.path());

        let resolution = core
            .resolve_feed_url(&format!("{}/", server.uri()), DiscoverOptions::default())
            .await
            .unwrap();
        assert_eq!(resolution.url, format!("{}/feed.xml", server.uri()));
        assert_eq!(resolution.alternates, Vec::<String>::new());
        assert_eq!(core.list_feeds().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn resolve_feed_url_reports_a_feedless_page() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<p>hello</p>"))
            .mount(&server)
            .await;
        let profile = tempfile::tempdir().unwrap();
        let core = core_trusting_localhost(profile.path());

        let error = core
            .resolve_feed_url(&format!("{}/", server.uri()), DiscoverOptions::default())
            .await
            .unwrap_err();
        assert!(matches!(error, DiscoverError::NoFeed { .. }), "{error}");
    }
}
