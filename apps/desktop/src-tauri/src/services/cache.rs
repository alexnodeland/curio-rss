//! Image caching service.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use crate::error::InfraError;

/// Cache statistics
#[derive(Debug, Clone, serde::Serialize)]
pub struct CacheStats {
    pub total_size_bytes: u64,
    pub file_count: usize,
    pub icons_size: u64,
    pub thumbnails_size: u64,
    pub images_size: u64,
    pub oldest_file: Option<String>,
}

/// Image cache service
pub struct ImageCache {
    cache_dir: PathBuf,
}

impl ImageCache {
    /// Create new image cache service
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get cache directory path
    pub fn cache_path(&self) -> &PathBuf {
        &self.cache_dir
    }

    /// Ensure cache directories exist
    pub fn init(&self) -> Result<(), InfraError> {
        let dirs = ["icons", "thumbnails", "images", "avatars"];

        for dir in &dirs {
            let path = self.cache_dir.join(dir);
            std::fs::create_dir_all(&path)?;
        }

        Ok(())
    }

    /// Get cache statistics
    pub fn get_stats(&self) -> Result<CacheStats, InfraError> {
        let mut stats = CacheStats {
            total_size_bytes: 0,
            file_count: 0,
            icons_size: 0,
            thumbnails_size: 0,
            images_size: 0,
            oldest_file: None,
        };

        let mut oldest_time: Option<SystemTime> = None;

        // Calculate sizes for each subdirectory
        for (dir, size_field) in [
            ("icons", &mut stats.icons_size),
            ("thumbnails", &mut stats.thumbnails_size),
            ("images", &mut stats.images_size),
        ] {
            let path = self.cache_dir.join(dir);
            if path.exists() {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.is_file() {
                                let size = metadata.len();
                                *size_field += size;
                                stats.total_size_bytes += size;
                                stats.file_count += 1;

                                if let Ok(modified) = metadata.modified() {
                                    if oldest_time.is_none() || modified < oldest_time.unwrap() {
                                        oldest_time = Some(modified);
                                        stats.oldest_file =
                                            Some(entry.path().to_string_lossy().to_string());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(stats)
    }

    /// Clear cache by type (None = all)
    pub fn clear(&self, cache_type: Option<&str>) -> Result<usize, InfraError> {
        let dirs: Vec<&str> = match cache_type {
            Some("icons") => vec!["icons"],
            Some("thumbnails") => vec!["thumbnails"],
            Some("images") => vec!["images"],
            Some("avatars") => vec!["avatars"],
            _ => vec!["icons", "thumbnails", "images", "avatars"],
        };

        let mut deleted = 0;

        for dir in dirs {
            let path = self.cache_dir.join(dir);
            if path.exists() {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        if entry.metadata().map(|m| m.is_file()).unwrap_or(false)
                            && std::fs::remove_file(entry.path()).is_ok()
                        {
                            deleted += 1;
                        }
                    }
                }
            }
        }

        Ok(deleted)
    }

    /// Clean up files older than the specified duration
    pub fn cleanup_older_than(&self, duration: Duration) -> Result<usize, InfraError> {
        let cutoff = SystemTime::now() - duration;
        let mut deleted = 0;

        for dir in &["thumbnails", "images", "avatars"] {
            let path = self.cache_dir.join(dir);
            if path.exists() {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.is_file() {
                                if let Ok(modified) = metadata.modified() {
                                    if modified < cutoff
                                        && std::fs::remove_file(entry.path()).is_ok()
                                    {
                                        deleted += 1;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(deleted)
    }

    /// Cleanup cache to stay under size limit (in bytes)
    pub fn enforce_size_limit(&self, max_bytes: u64) -> Result<usize, InfraError> {
        let stats = self.get_stats()?;

        if stats.total_size_bytes <= max_bytes {
            return Ok(0);
        }

        // Collect all files with their metadata
        let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();

        for dir in &["thumbnails", "images", "avatars"] {
            let path = self.cache_dir.join(dir);
            if path.exists() {
                if let Ok(entries) = std::fs::read_dir(&path) {
                    for entry in entries.flatten() {
                        if let Ok(metadata) = entry.metadata() {
                            if metadata.is_file() {
                                if let Ok(modified) = metadata.modified() {
                                    files.push((entry.path(), metadata.len(), modified));
                                }
                            }
                        }
                    }
                }
            }
        }

        // Sort by modification time (oldest first)
        files.sort_by_key(|(_, _, time)| *time);

        // Delete oldest files until we're under the limit
        let mut current_size = stats.total_size_bytes;
        let mut deleted = 0;

        for (path, size, _) in files {
            if current_size <= max_bytes {
                break;
            }

            if std::fs::remove_file(&path).is_ok() {
                current_size -= size;
                deleted += 1;
            }
        }

        Ok(deleted)
    }

    /// Store an image in the cache
    pub fn store(
        &self,
        category: &str,
        key: &str,
        data: &[u8],
        ext: &str,
    ) -> Result<PathBuf, InfraError> {
        let dir = self.cache_dir.join(category);
        std::fs::create_dir_all(&dir)?;

        let filename = format!("{}.{}", key, ext);
        let path = dir.join(&filename);

        std::fs::write(&path, data)?;

        Ok(path)
    }

    /// Get an image from cache
    pub fn get(&self, category: &str, key: &str) -> Option<PathBuf> {
        let dir = self.cache_dir.join(category);

        // Try common extensions
        for ext in &["png", "jpg", "jpeg", "gif", "webp", "ico", "svg"] {
            let path = dir.join(format!("{}.{}", key, ext));
            if path.exists() {
                return Some(path);
            }
        }

        None
    }
}
