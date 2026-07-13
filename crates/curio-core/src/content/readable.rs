//! Full-page main-content extraction (readability) via `dom_smoothie`.
//!
//! Distinct from [`super::process`], which sanitizes feed *fragments* first:
//! a full fetched page is *extracted* first (find the article body amid nav,
//! comments and chrome), then [`super::sanitize`] is the single security gate
//! over the extractor's output. `dom_smoothie` does zero network I/O — the
//! `url` argument only absolutizes relative links — so this adds no SSRF
//! surface; all fetching stays in the policed client.

use dom_smoothie::{Config, Readability};

use super::ContentError;

/// The page's own metadata, as the readability pass discovered it
/// (`OpenGraph` / schema.org / head fallbacks). What a single-URL save
/// needs beyond the body; every field is best-effort.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PageMeta {
    /// The page title (og:title → `<title>`), if non-empty.
    pub title: Option<String>,
    /// The author byline, if declared.
    pub byline: Option<String>,
    /// The document language, if declared.
    pub lang: Option<String>,
    /// The declared publish time, verbatim (callers parse it leniently).
    pub published_time: Option<String>,
    /// The page's lead image (og:image et al.), if declared.
    pub image: Option<String>,
}

/// Extracts the main article HTML from a full page. `url` is the article's
/// final (post-redirect) URL, used to absolutize relative links so the
/// downstream sanitize step judges them against real schemes.
///
/// # Errors
///
/// [`ContentError::Readability`] if the document cannot be parsed or scored.
pub fn extract_full_page(raw_html: &str, url: &str) -> Result<String, ContentError> {
    Ok(extract_full_page_with_meta(raw_html, url)?.0)
}

/// [`extract_full_page`], also surfacing the page's own metadata.
///
/// # Errors
///
/// [`ContentError::Readability`] if the document cannot be parsed or scored.
pub fn extract_full_page_with_meta(
    raw_html: &str,
    url: &str,
) -> Result<(String, PageMeta), ContentError> {
    let mut readability = Readability::new(raw_html, Some(url), Some(Config::default()))?;
    let article = readability.parse()?;
    let non_empty = |s: String| {
        let trimmed = s.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    };
    let meta = PageMeta {
        title: non_empty(article.title.clone()),
        byline: article.byline.clone().and_then(non_empty),
        lang: article.lang.clone().and_then(non_empty),
        published_time: article.published_time.clone().and_then(non_empty),
        image: article.image.clone().and_then(non_empty),
    };
    Ok((article.content.to_string(), meta))
}
