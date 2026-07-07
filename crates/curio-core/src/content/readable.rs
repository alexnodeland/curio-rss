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

/// Extracts the main article HTML from a full page. `url` is the article's
/// final (post-redirect) URL, used to absolutize relative links so the
/// downstream sanitize step judges them against real schemes.
///
/// # Errors
///
/// [`ContentError::Readability`] if the document cannot be parsed or scored.
pub fn extract_full_page(raw_html: &str, url: &str) -> Result<String, ContentError> {
    let mut readability = Readability::new(raw_html, Some(url), Some(Config::default()))?;
    let article = readability.parse()?;
    Ok(article.content.to_string())
}
