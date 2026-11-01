//! Articles: listing, lookup, hydration, single-URL save, search,
//! counting, deletion, and the tag vocabulary.

use std::collections::BTreeMap;

use curio_types::Timestamp;

use crate::content;
use crate::fetch::{FetchKind, FetchRequest};
use crate::model::{
    Article, ArticleContent, ArticleId, ArticleState, ContentOrigin, FeedId, NewArticle,
};
use crate::storage::ListArticles;

use super::refresh::is_feed_own_url;
use super::{CoreError, CoreHandle, SavedUrl};

impl CoreHandle {
    /// Keyset-paginated article listing.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_articles(&self, params: ListArticles) -> Result<Vec<Article>, CoreError> {
        Ok(self.storage.list_articles(params)?)
    }

    /// One article by row id.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn get_article(&self, id: ArticleId) -> Result<Option<Article>, CoreError> {
        Ok(self.storage.get_article(id)?)
    }

    /// Fetches the article's source page and replaces its stored content with
    /// the readability-extracted main content — the on-demand "load full
    /// article" enrichment. Uses the SAME policed client as refresh (the SSRF
    /// guard applies; the private-network exemption is taken only from the
    /// owning feed's flag, never widened for a manual fetch). Never clobbers
    /// good content: a non-2xx response, or an empty/shorter extract, leaves
    /// the article unchanged. DB-local like [`Self::mark_read`] — the events
    /// contract has no article-content event, so nothing is staged.
    ///
    /// A **link-less** feed entry (its `source_url` is the feed's own URL)
    /// has no page to load: it is returned unchanged without a fetch, as is
    /// any article whose fetch lands on its feed's URL (a link-less entry
    /// stored before the feed's URL was redirect-adopted). Extracting the
    /// feed XML as if it were the article would replace the real body.
    ///
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article; fetch, content or
    /// storage errors.
    pub async fn hydrate_article(&self, id: ArticleId) -> Result<Article, CoreError> {
        let article = self
            .storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })?;
        let feed = match article.feed_id {
            Some(feed_id) => self.storage.get_feed(feed_id)?,
            None => None,
        };
        // Matched against the feed's *current* URL only: the store keeps no
        // URL history, so a link-less row saved under a URL the feed no
        // longer has — and that no longer redirects to it (the user edited
        // the feed URL, say) — is not recognized and is fetched like a page.
        // The redirect case is covered by also checking where the fetch
        // lands, below.
        let lands_on_feed =
            |url: &str| feed.as_ref().is_some_and(|feed| is_feed_own_url(url, feed));
        if lands_on_feed(&article.source_url) {
            tracing::debug!(article = %id, "link-less feed entry: nothing to hydrate");
            return Ok(article);
        }
        let allow_private_network = feed.as_ref().is_some_and(|feed| feed.allow_private_network);
        // Enrichment first (feature-gated): a source-specific provider
        // beats generic readability when it recognizes the URL. A rate
        // limit is a hard stop (no fallback fetch to the throttled
        // host); any other enrichment failure falls through to the
        // generic page fetch below.
        #[cfg(feature = "enrich-reddit")]
        if let Some(enriched) = self
            .try_reddit_enrichment(id, &article, allow_private_network)
            .await?
        {
            return Ok(enriched);
        }
        let response = self
            .client
            .fetch(&FetchRequest {
                url: article.source_url.clone(),
                allow_private_network,
                kind: FetchKind::Page,
                ..FetchRequest::default()
            })
            .await?;
        // A PDF, image or other non-HTML body has no main content to
        // extract — readability over binary data would only make noise.
        if !response.is_success() || lands_on_feed(&response.final_url) || !response.is_html_page()
        {
            return Ok(article);
        }
        let processed = content::process_full_page(&response.html_text(), &response.final_url)?;
        // A mis-scored thin page must not clobber real feed content.
        if processed.text.trim().is_empty() || processed.text.len() < article.content.text.len() {
            return Ok(article);
        }
        self.storage.update_article_content(
            id,
            &ArticleContent {
                html: processed.html,
                text: processed.text,
            },
            Some(processed.word_count),
            ContentOrigin::FullText,
        )?;
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
    }

    /// Saves a single URL as a feedless read-later article — the
    /// GoodLinks-style clip path. The page is fetched through the policed
    /// client (no private-network exemption — that flag belongs to feeds)
    /// and readability-extracted, adopting the page's own title, byline,
    /// language, publish time and lead image where declared. The page is
    /// decoded by its BOM, HTTP charset, then `<meta charset>`. A non-HTML
    /// URL (a PDF, an image) is clipped link-only without extraction. An
    /// unreachable or unextractable page still saves the bare link
    /// (`hydrated: false`) — a save must never lose the URL.
    ///
    /// Dedupe is by URL within the manual-save scope: re-saving a URL
    /// (or importing it first) re-flags it read-later and applies any new
    /// tags, without refetching. Emits `article.read_later.added` and
    /// `article.tagged` per the state changes it actually makes.
    ///
    /// # Errors
    ///
    /// [`CoreError::InvalidUrl`] for a non-http(s) input; storage or
    /// emission errors. Fetch/extract failures are NOT errors (the bare
    /// link is saved instead).
    pub async fn save_url(&self, url: &str, tags: Vec<String>) -> Result<SavedUrl, CoreError> {
        let canonical = url::Url::parse(url.trim())
            .ok()
            .filter(|parsed| matches!(parsed.scheme(), "http" | "https"))
            .map(String::from)
            .ok_or_else(|| CoreError::InvalidUrl {
                url: url.to_owned(),
            })?;
        let key = crate::feeds::dedupe_key(None, Some(&canonical), &canonical, None);
        if let Some(id) = self.storage.article_id_by_dedupe_key(None, &key)? {
            let article = self.flag_saved_url(id, &tags)?;
            return Ok(SavedUrl {
                article,
                created: false,
                hydrated: false,
            });
        }

        // Best-effort fetch + extract: the row below is written either way.
        let page = match self
            .client
            .fetch(&FetchRequest {
                url: canonical.clone(),
                kind: FetchKind::Page,
                ..FetchRequest::default()
            })
            .await
        {
            // Only an HTML page is extracted; a PDF or image URL is clipped
            // link-only, like an unreachable page.
            Ok(response) if response.is_success() && response.is_html_page() => {
                content::process_page(&response.html_text(), &response.final_url).ok()
            }
            _ => None,
        };
        let hydrated = page.is_some();
        let page = page.unwrap_or_default();
        let (body, meta) = (page.content, page.meta);
        // The page's declared image wins when it's a fetchable URL; else
        // fall back to the first inline <img> of the sanitized body.
        let lead_image = meta
            .image
            .as_deref()
            .map(str::trim)
            .filter(|src| src.starts_with("http://") || src.starts_with("https://"))
            .map(ToOwned::to_owned)
            .or_else(|| content::first_image(&body.html));
        self.storage.upsert_articles(vec![NewArticle {
            feed_id: None,
            dedupe_key: key.clone(),
            title: meta.title.unwrap_or_else(|| canonical.clone()),
            source_url: canonical,
            author: meta.byline,
            published_at: meta
                .published_time
                .and_then(|raw| raw.parse::<Timestamp>().ok()),
            lang: meta.lang,
            word_count: hydrated.then_some(body.word_count),
            content: ArticleContent {
                html: body.html,
                text: body.text,
            },
            source_updated_at: None,
            lead_image,
        }])?;
        let id = self
            .storage
            .article_id_by_dedupe_key(None, &key)?
            .ok_or(CoreError::NotFound {
                entity: "saved article",
            })?;
        let article = self.flag_saved_url(id, &tags)?;
        Ok(SavedUrl {
            article,
            created: true,
            hydrated,
        })
    }

    /// Flags a saved-URL row read-later and applies its tags (each staging
    /// its event), flushing one emission; returns the fresh row.
    fn flag_saved_url(&self, id: ArticleId, tags: &[String]) -> Result<Article, CoreError> {
        self.storage.add_read_later(id)?;
        for tag in tags {
            self.storage.tag_article(id, tag)?;
        }
        self.emit();
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
    }

    /// BYO OAuth (D15): trades the installed credentials for a bearer.
    /// `Ok((None, _))` = no credentials, or a failed grant, which degrades
    /// to the unauthenticated endpoint; the second value is the status of
    /// a grant the token endpoint *refused* (400/401/403), remembered so
    /// that if reddit then blocks the anonymous request the user hears
    /// their credentials were refused, not that they have none. A 429 from
    /// the token endpoint is a rate limit like any other: it trips the
    /// breaker and is returned as [`CoreError::RateLimited`].
    #[cfg(feature = "enrich-reddit")]
    async fn reddit_bearer(&self) -> Result<(Option<String>, Option<u16>), CoreError> {
        use crate::enrich::EnrichError;

        match self.reddit_auth.bearer(&self.client).await {
            Ok(bearer) => Ok((bearer, None)),
            Err(EnrichError::Http {
                status: 429,
                retry_after,
            }) => {
                self.enrich_breaker.trip(retry_after);
                tracing::warn!("reddit token endpoint rate limit; enrichment breaker opened");
                Err(reddit_rate_limited(self.enrich_breaker.open_remaining()))
            }
            Err(err) => {
                let refused = match err {
                    EnrichError::Http {
                        status: status @ (400 | 401 | 403),
                        ..
                    } => Some(status),
                    _ => None,
                };
                tracing::warn!(%err, "reddit token grant failed; using the public endpoint");
                Ok((None, refused))
            }
        }
    }

    /// The reddit enrichment attempt (D14/D15), split out of
    /// [`Self::hydrate_article`]: `Ok(Some(_))` = enriched and stored;
    /// `Ok(None)` = not a reddit post / soft failure, use the generic
    /// path; `Err(RateLimited)` = hard stop, make no further request to
    /// the throttled host. Provider output passes the SAME sanitize gate
    /// as everything else. A 429 — from the token grant or the API call
    /// — trips the breaker immediately, honoring `Retry-After`.
    #[cfg(feature = "enrich-reddit")]
    async fn try_reddit_enrichment(
        &self,
        id: ArticleId,
        article: &Article,
        allow_private_network: bool,
    ) -> Result<Option<Article>, CoreError> {
        use crate::RedditAuthReason;
        use crate::enrich::{EnrichError, reddit};

        if !reddit::is_reddit_post(&article.source_url) {
            return Ok(None);
        }

        if !self.enrich_breaker.closed() {
            return Err(reddit_rate_limited(self.enrich_breaker.open_remaining()));
        }
        let (bearer, grant_refused) = self.reddit_bearer().await?;
        let api_origin = self.reddit_auth.api_origin();
        let auth = match (bearer.as_deref(), api_origin.as_deref()) {
            (Some(bearer), Some(api_origin)) => Some(reddit::PostAuth { bearer, api_origin }),
            _ => None,
        };
        let authenticated = auth.is_some();
        match reddit::fetch_post(
            &self.client,
            &article.source_url,
            allow_private_network,
            auth,
        )
        .await
        {
            Ok(enriched) => {
                self.enrich_breaker.record_success();
                self.store_enriched(id, &article.source_url, &enriched.html)
            }
            Err(EnrichError::Http {
                status: 429,
                retry_after,
            }) => {
                self.enrich_breaker.trip(retry_after);
                tracing::warn!(
                    article = %id,
                    retry_after_secs = retry_after.map(|d| d.as_secs()),
                    "reddit rate limit hit; enrichment breaker opened"
                );
                Err(reddit_rate_limited(self.enrich_breaker.open_remaining()))
            }
            Err(EnrichError::Http {
                status: status @ (401 | 403),
                ..
            }) if authenticated => {
                // A stale or revoked token: drop it so the next call
                // re-grants, and tell the user — falling back to an HTML
                // fetch of the same host would only produce junk.
                self.reddit_auth.invalidate();
                tracing::warn!(article = %id, status, "reddit bearer rejected; token dropped");
                Err(CoreError::RedditAuth {
                    reason: RedditAuthReason::TokenRevoked,
                    status,
                })
            }
            Err(EnrichError::Http {
                status: status @ (401 | 403),
                ..
            }) => {
                // Reddit's 2026 lockdown: anonymous .json requests are
                // 403-blocked outright. Deterministic — no breaker, no
                // fallback fetch; the guidance IS the answer. Installed
                // credentials the token endpoint refused are what to fix.
                tracing::warn!(article = %id, status, "anonymous reddit JSON refused");
                Err(match grant_refused {
                    Some(grant_status) => CoreError::RedditAuth {
                        reason: RedditAuthReason::CredentialsRejected,
                        status: grant_status,
                    },
                    None => CoreError::RedditAuth {
                        reason: RedditAuthReason::CredentialsMissing,
                        status,
                    },
                })
            }
            Err(err) => {
                self.enrich_breaker.record_failure();
                tracing::warn!(
                    article = %id,
                    %err,
                    "reddit enrichment failed; falling back to the page fetch"
                );
                Ok(None)
            }
        }
    }

    /// Stores a provider's enriched body. Provider output IS the main
    /// content already — it only passes the sanitize gate (readability's
    /// boilerplate extraction would drop the appended media figures).
    /// The API is authoritative for its own posts, so it may replace
    /// longer feed content — but never with nothing (`Ok(None)` sends
    /// the caller down the generic path instead).
    #[cfg(feature = "enrich-reddit")]
    fn store_enriched(
        &self,
        id: ArticleId,
        source_url: &str,
        raw_html: &str,
    ) -> Result<Option<Article>, CoreError> {
        let html = content::sanitize(raw_html, Some(source_url));
        let text = content::plain_text(&html);
        let word_count = u32::try_from(text.split_whitespace().count()).unwrap_or(u32::MAX);
        if text.trim().is_empty() && content::first_image(&html).is_none() {
            return Ok(None);
        }
        self.storage.update_article_content(
            id,
            &ArticleContent { html, text },
            Some(word_count),
            ContentOrigin::Enriched,
        )?;
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
            .map(Some)
    }

    /// Full-text search (escaped FTS5 — user input is never raw MATCH).
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn search_articles(&self, query: &str, limit: u32) -> Result<Vec<Article>, CoreError> {
        Ok(self.storage.search_articles(query, limit)?)
    }

    /// How many articles are stored in total.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn count_articles(&self) -> Result<u64, CoreError> {
        Ok(self.storage.count_articles()?)
    }

    /// Deletes an article (state, tags, and the FTS index entry go with
    /// it via cascade + trigger). DB-local, no contract event.
    ///
    /// # Errors
    ///
    /// [`CoreError::Storage`] wrapping `NotFound` if the article does not
    /// exist; storage errors.
    pub fn delete_article(&self, id: ArticleId) -> Result<(), CoreError> {
        Ok(self.storage.delete_article(id)?)
    }

    /// Every known tag name, sorted — the tag vocabulary for pickers and
    /// autocomplete.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn list_tags(&self) -> Result<Vec<String>, CoreError> {
        Ok(self.storage.list_tags()?)
    }

    /// The article's current flag projection.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn article_state(&self, id: ArticleId) -> Result<ArticleState, CoreError> {
        Ok(self.storage.article_state(id)?)
    }

    /// The article's tags, sorted.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn article_tags(&self, id: ArticleId) -> Result<Vec<String>, CoreError> {
        Ok(self.storage.article_tags(id)?)
    }

    /// Unread-article counts grouped by feed (`None` = articles without
    /// a feed; total = the sum of the values). Counts are backend-owned:
    /// heads render them, they never re-derive them client-side.
    ///
    /// # Errors
    ///
    /// Storage errors.
    pub fn unread_counts(&self) -> Result<BTreeMap<Option<FeedId>, u64>, CoreError> {
        Ok(self.storage.unread_counts()?)
    }
}

/// The hard stop for a throttled reddit: no request until the breaker's
/// `remaining` cool-down (a minute when unknown) has passed.
#[cfg(feature = "enrich-reddit")]
fn reddit_rate_limited(remaining: Option<std::time::Duration>) -> CoreError {
    CoreError::RateLimited {
        host: "reddit.com".to_owned(),
        retry_after_secs: remaining.map_or(60, |d| d.as_secs().max(1)),
    }
}
