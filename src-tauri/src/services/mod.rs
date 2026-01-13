//! Background services.

mod cache;
mod favicon;
mod refresh;

pub use cache::{CacheStats, ImageCache};
pub use favicon::FaviconService;
pub use refresh::RefreshService;
