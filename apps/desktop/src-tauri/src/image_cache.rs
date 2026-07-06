//! Content-hash LRU image cache in the OS cache directory.
//!
//! This is how feed/article images load without opening the CSP to
//! `https:`: the webview never fetches remote images itself. Rust fetches
//! through curio-core's policed client (SSRF guard, size cap, honest UA —
//! never a bare reqwest client), stores the bytes under
//! `<os-cache>/curio/images/<sha256(url)>.<ext>`, and the frontend loads
//! the returned path through the `asset:` protocol, whose scope in
//! `tauri.conf.json` is exactly this directory.
//!
//! Eviction is LRU by file mtime (touched on every hit), capped at
//! [`DEFAULT_MAX_CACHE_BYTES`] — a cache, not a hoard.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use curio_core::fetch::{FetchRequest, PolicedClient};
use sha2::{Digest as _, Sha256};

use crate::error::{CommandError, ErrorCode};

/// Default cache size cap: 256 MiB.
pub const DEFAULT_MAX_CACHE_BYTES: u64 = 256 * 1024 * 1024;

/// Extensions the `asset:` protocol can mime-type; anything else is
/// stored as `.img` (served as octet-stream — the `<img>` tag sniffs).
const KNOWN_EXTENSIONS: &[&str] = &["avif", "bmp", "gif", "ico", "jpeg", "jpg", "png", "webp"];

/// The image cache, managed as Tauri state.
#[derive(Debug)]
pub struct ImageCache {
    dir: PathBuf,
    max_bytes: u64,
    client: PolicedClient,
}

impl ImageCache {
    /// A cache rooted at `dir` (created lazily), evicting past
    /// `max_bytes`, fetching through `client`.
    #[must_use]
    pub fn new(dir: PathBuf, max_bytes: u64, client: PolicedClient) -> Self {
        Self {
            dir,
            max_bytes,
            client,
        }
    }

    /// The absolute path of the cached copy of `url`, fetching (policed)
    /// on a miss and touching the LRU clock on a hit.
    ///
    /// # Errors
    ///
    /// User-tier network errors from the policed fetch; internal IO
    /// errors writing the cache.
    pub async fn get(&self, url: &str) -> Result<PathBuf, CommandError> {
        let path = self.dir.join(cache_file_name(url));
        if path.is_file() {
            touch(&path);
            return Ok(path);
        }

        let response = self
            .client
            .fetch(&FetchRequest::new(url))
            .await
            .map_err(|error| {
                CommandError::user(
                    ErrorCode::Network,
                    format!("image fetch failed: {error}"),
                    true,
                )
            })?;
        if !response.is_success() {
            return Err(CommandError::user(
                ErrorCode::Network,
                format!("image fetch failed with HTTP {}", response.status),
                true,
            ));
        }

        std::fs::create_dir_all(&self.dir).map_err(|error| io_error(&self.dir, &error))?;
        // Write-then-rename so a torn write never serves as an image.
        let staging = path.with_extension("part");
        std::fs::write(&staging, &response.body).map_err(|error| io_error(&staging, &error))?;
        std::fs::rename(&staging, &path).map_err(|error| io_error(&path, &error))?;
        self.evict_past_cap();
        Ok(path)
    }

    /// Deletes least-recently-used entries until the cache fits the cap.
    fn evict_past_cap(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(SystemTime, u64, PathBuf)> = entries
            .flatten()
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                if !meta.is_file() {
                    return None;
                }
                let mtime = meta.modified().ok()?;
                Some((mtime, meta.len(), entry.path()))
            })
            .collect();
        let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
        if total <= self.max_bytes {
            return;
        }
        files.sort_by_key(|(mtime, _, _)| *mtime);
        for (_, len, path) in files {
            if total <= self.max_bytes {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
    }
}

/// `sha256(url)` hex + a mime-mappable extension: content-hash keyed, so
/// the same URL never stores twice and hostile URLs cannot choose names.
fn cache_file_name(url: &str) -> String {
    let hash = Sha256::digest(url.as_bytes());
    let mut name = String::with_capacity(70);
    for byte in hash {
        use std::fmt::Write as _;
        let _ = write!(name, "{byte:02x}");
    }
    name.push('.');
    name.push_str(extension_for(url));
    name
}

fn extension_for(url: &str) -> &'static str {
    let path = url.split(['?', '#']).next().unwrap_or_default();
    let ext = path
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    KNOWN_EXTENSIONS
        .iter()
        .find(|known| **known == ext)
        .copied()
        .unwrap_or("img")
}

fn touch(path: &Path) {
    if let Ok(file) = std::fs::File::options().append(true).open(path) {
        let _ = file.set_modified(SystemTime::now());
    }
}

fn io_error(path: &Path, error: &std::io::Error) -> CommandError {
    CommandError::internal(
        ErrorCode::Io,
        format!("image cache io at {}: {error}", path.display()),
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::collections::HashSet;
    use std::net::{IpAddr, Ipv4Addr};
    use std::time::Duration;

    use curio_core::fetch::FetchConfig;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn test_client() -> PolicedClient {
        PolicedClient::new(FetchConfig {
            politeness_delay: Duration::ZERO,
            trusted_addrs: HashSet::from([IpAddr::V4(Ipv4Addr::LOCALHOST)]),
            ..FetchConfig::default()
        })
    }

    #[tokio::test]
    async fn fetches_once_and_serves_the_cached_copy() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/logo.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0x89, 0x50, 0x4e, 0x47]))
            .expect(1) // the second get must hit the cache
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::new(
            dir.path().join("images"),
            DEFAULT_MAX_CACHE_BYTES,
            test_client(),
        );
        let url = format!("{}/logo.png", server.uri());

        let first = cache.get(&url).await.unwrap();
        assert!(first.is_file());
        assert_eq!(first.extension().unwrap(), "png");
        assert!(first.starts_with(dir.path().join("images")));

        let second = cache.get(&url).await.unwrap();
        assert_eq!(first, second);
    }

    #[tokio::test]
    async fn http_errors_surface_as_user_network_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/gone.png"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().unwrap();
        let cache = ImageCache::new(
            dir.path().join("images"),
            DEFAULT_MAX_CACHE_BYTES,
            test_client(),
        );
        let error = cache
            .get(&format!("{}/gone.png", server.uri()))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Network);
        assert_eq!(error.kind, crate::error::ErrorKind::User);
    }

    #[tokio::test]
    async fn evicts_oldest_entries_past_the_cap() {
        let server = MockServer::start().await;
        for name in ["a", "b", "c"] {
            Mock::given(method("GET"))
                .and(path(format!("/{name}.png")))
                .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 1024]))
                .mount(&server)
                .await;
        }

        let dir = tempfile::tempdir().unwrap();
        // Cap fits two 1 KiB entries, not three.
        let cache = ImageCache::new(dir.path().join("images"), 2048, test_client());
        let first = cache.get(&format!("{}/a.png", server.uri())).await.unwrap();
        // Ensure distinct mtimes so LRU order is deterministic.
        std::thread::sleep(Duration::from_millis(20));
        let second = cache.get(&format!("{}/b.png", server.uri())).await.unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let third = cache.get(&format!("{}/c.png", server.uri())).await.unwrap();

        assert!(!first.exists(), "oldest entry should have been evicted");
        assert!(second.exists());
        assert!(third.exists());
    }

    #[test]
    fn unknown_extensions_fall_back_to_img() {
        assert_eq!(extension_for("https://x.test/pic.png"), "png");
        assert_eq!(extension_for("https://x.test/pic.JPG?w=100"), "jpg");
        assert_eq!(extension_for("https://x.test/pic.svg"), "img");
        assert_eq!(extension_for("https://x.test/noext"), "img");
    }

    #[test]
    fn cache_names_are_hash_keyed_not_url_derived() {
        let a = cache_file_name("https://x.test/../../../etc/passwd.png");
        let ext = std::path::Path::new(&a).extension().unwrap();
        assert_eq!(ext, "png");
        assert_eq!(a.len(), 64 + 4);
        assert!(!a.contains('/'));
    }
}
