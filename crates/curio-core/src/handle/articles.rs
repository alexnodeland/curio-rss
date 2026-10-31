//! Articles: listing, lookup, hydration, single-URL save, search,
//! counting, deletion, and the tag vocabulary.

use std::collections::BTreeMap;

use curio_types::Timestamp;

use crate::content;
use crate::fetch::FetchRequest;
use crate::model::{Article, ArticleContent, ArticleId, ArticleState, FeedId, NewArticle};
use crate::storage::ListArticles;

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
    /// # Errors
    ///
    /// [`CoreError::NotFound`] for an unknown article; fetch, content or
    /// storage errors.
    pub async fn hydrate_article(&self, id: ArticleId) -> Result<Article, CoreError> {
        let article = self
            .storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })?;
        let allow_private_network = match article.feed_id {
            Some(feed_id) => self
                .storage
                .get_feed(feed_id)?
                .is_some_and(|feed| feed.allow_private_network),
            None => false,
        };
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
                ..FetchRequest::default()
            })
            .await?;
        if !response.is_success() {
            return Ok(article);
        }
        let raw = String::from_utf8_lossy(&response.body);
        let processed = content::process_full_page(&raw, &response.final_url)?;
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
        )?;
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
    }

    /// Saves a single URL as a feedless read-later article — the
    /// GoodLinks-style clip path. The page is fetched through the policed
    /// client (no private-network exemption — that flag belongs to feeds)
    /// and readability-extracted, adopting the page's own title, byline,
    /// language, publish time and lead image where declared. An
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
                ..FetchRequest::default()
            })
            .await
        {
            Ok(response) if response.is_success() => {
                let raw = String::from_utf8_lossy(&response.body);
                content::process_page(&raw, &response.final_url).ok()
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
        self.emit()?;
        self.storage
            .get_article(id)?
            .ok_or(CoreError::NotFound { entity: "article" })
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
        use crate::enrich::{EnrichError, reddit};

        if !reddit::is_reddit_post(&article.source_url) {
            return Ok(None);
        }
        let rate_limited = |remaining: Option<std::time::Duration>| CoreError::RateLimited {
            host: "reddit.com".to_owned(),
            retry_after_secs: remaining.map_or(60, |d| d.as_secs().max(1)),
        };
        if !self.enrich_breaker.closed() {
            return Err(rate_limited(self.enrich_breaker.open_remaining()));
        }
        // BYO OAuth (D15): trade the installed credentials for a bearer.
        // A 429 from the token endpoint is a rate limit like any other;
        // a failed grant (bad credentials) degrades to unauthenticated.
        let bearer = match self.reddit_auth.bearer(&self.client).await {
            Ok(bearer) => bearer,
            Err(EnrichError::Http {
                status: 429,
                retry_after,
            }) => {
                self.enrich_breaker.trip(retry_after);
                tracing::warn!("reddit token endpoint rate limit; enrichment breaker opened");
                return Err(rate_limited(self.enrich_breaker.open_remaining()));
            }
            Err(err) => {
                tracing::warn!(%err, "reddit token grant failed; using the public endpoint");
                None
            }
        };
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
                Err(rate_limited(self.enrich_breaker.open_remaining()))
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
                    detail: format!(
                        "reddit rejected the stored API credentials (HTTP {status}) — \
                         check the client id and secret, or re-create the app"
                    ),
                })
            }
            Err(EnrichError::Http {
                status: status @ (401 | 403),
                ..
            }) => {
                // Reddit's 2026 lockdown: anonymous .json requests are
                // 403-blocked outright. Deterministic — no breaker, no
                // fallback fetch; the guidance IS the answer.
                tracing::warn!(article = %id, status, "anonymous reddit JSON refused");
                Err(CoreError::RedditAuth {
                    detail: format!(
                        "reddit blocks anonymous full-post requests (HTTP {status}) — \
                         add your own free Reddit API credentials to load full posts"
                    ),
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
