//! Favicon fetching and caching service.

use reqwest::Client;
use scraper::{Html, Selector};
use std::path::PathBuf;
use std::time::Duration;
use url::Url;

use crate::error::InfraError;

/// Favicon service for fetching and caching feed icons
pub struct FaviconService {
    client: Client,
    cache_dir: PathBuf,
}

impl FaviconService {
    /// Create a new favicon service
    pub fn new(cache_dir: PathBuf) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .user_agent("Curio Reader/1.0 (RSS Desktop Client)")
            .build()
            .expect("Failed to create HTTP client");

        Self { client, cache_dir }
    }

    /// Fetch favicon for a site URL
    pub async fn fetch_favicon(&self, site_url: &str) -> Result<String, InfraError> {
        let parsed = Url::parse(site_url)
            .map_err(|e| InfraError::FeedParse(format!("Invalid URL: {}", e)))?;

        let base_url = format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or(""));

        // Try multiple methods in order
        let favicon_url = self
            .try_favicon_ico(&base_url)
            .await
            .or_else(|_| futures::executor::block_on(self.try_html_link(site_url)))
            .or_else(|_| self.google_favicon_service(&base_url))?;

        // Download and cache the favicon
        self.download_and_cache(&favicon_url, &base_url).await
    }

    /// Try fetching /favicon.ico directly
    async fn try_favicon_ico(&self, base_url: &str) -> Result<String, InfraError> {
        let favicon_url = format!("{}/favicon.ico", base_url);

        let response = self
            .client
            .head(&favicon_url)
            .send()
            .await
            .map_err(|e| InfraError::Network(e.to_string()))?;

        if response.status().is_success() {
            Ok(favicon_url)
        } else {
            Err(InfraError::Network("Favicon not found".to_string()))
        }
    }

    /// Try parsing HTML for link rel="icon"
    async fn try_html_link(&self, site_url: &str) -> Result<String, InfraError> {
        let response = self
            .client
            .get(site_url)
            .send()
            .await
            .map_err(|e| InfraError::Network(e.to_string()))?;

        let html = response
            .text()
            .await
            .map_err(|e| InfraError::Network(e.to_string()))?;

        let document = Html::parse_document(&html);

        // Try various selectors for favicons
        let selectors = [
            r#"link[rel="icon"]"#,
            r#"link[rel="shortcut icon"]"#,
            r#"link[rel="apple-touch-icon"]"#,
            r#"link[rel="apple-touch-icon-precomposed"]"#,
        ];

        for selector_str in &selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                if let Some(element) = document.select(&selector).next() {
                    if let Some(href) = element.value().attr("href") {
                        // Resolve relative URLs
                        let base = Url::parse(site_url)
                            .map_err(|e| InfraError::FeedParse(e.to_string()))?;
                        let resolved = base
                            .join(href)
                            .map_err(|e| InfraError::FeedParse(e.to_string()))?;
                        return Ok(resolved.to_string());
                    }
                }
            }
        }

        Err(InfraError::Network(
            "No favicon link found in HTML".to_string(),
        ))
    }

    /// Fallback to Google's favicon service
    fn google_favicon_service(&self, base_url: &str) -> Result<String, InfraError> {
        let parsed = Url::parse(base_url)
            .map_err(|e| InfraError::FeedParse(format!("Invalid URL: {}", e)))?;

        let domain = parsed.host_str().unwrap_or("");
        Ok(format!(
            "https://www.google.com/s2/favicons?domain={}&sz=64",
            domain
        ))
    }

    /// Download favicon and cache it locally
    async fn download_and_cache(
        &self,
        favicon_url: &str,
        base_url: &str,
    ) -> Result<String, InfraError> {
        let response = self
            .client
            .get(favicon_url)
            .send()
            .await
            .map_err(|e| InfraError::Network(e.to_string()))?;

        if !response.status().is_success() {
            return Err(InfraError::Network(format!(
                "Failed to download favicon: {}",
                response.status()
            )));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("image/x-icon");

        let bytes = response
            .bytes()
            .await
            .map_err(|e| InfraError::Network(e.to_string()))?;

        // Determine file extension
        let ext = match content_type {
            t if t.contains("png") => "png",
            t if t.contains("jpeg") || t.contains("jpg") => "jpg",
            t if t.contains("svg") => "svg",
            t if t.contains("gif") => "gif",
            _ => "ico",
        };

        // Create cache directory if needed
        let icons_dir = self.cache_dir.join("icons");
        std::fs::create_dir_all(&icons_dir)
            .map_err(|e| InfraError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;

        // Hash the URL for filename
        let hash = format!("{:x}", md5::compute(base_url.as_bytes()));
        let filename = format!("{}.{}", hash, ext);
        let cache_path = icons_dir.join(&filename);

        // Write to cache
        std::fs::write(&cache_path, &bytes).map_err(|e| InfraError::Io(e))?;

        Ok(cache_path.to_string_lossy().to_string())
    }

    /// Get cached favicon path for a site URL
    pub fn get_cached_favicon(&self, site_url: &str) -> Option<String> {
        let parsed = Url::parse(site_url).ok()?;
        let base_url = format!("{}://{}", parsed.scheme(), parsed.host_str()?);
        let hash = format!("{:x}", md5::compute(base_url.as_bytes()));

        let icons_dir = self.cache_dir.join("icons");

        // Check for any extension
        for ext in &["png", "ico", "jpg", "svg", "gif"] {
            let path = icons_dir.join(format!("{}.{}", hash, ext));
            if path.exists() {
                return Some(path.to_string_lossy().to_string());
            }
        }

        None
    }
}
