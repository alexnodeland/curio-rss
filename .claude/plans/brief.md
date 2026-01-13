# Curio Reader: Development Brief

> Engineering standards, architecture patterns, and developer experience guidelines.

## 1. Engineering Principles

### 1.1 Core Philosophy

|Principle                        |Application                                                                                     |
|---------------------------------|------------------------------------------------------------------------------------------------|
|**DRY**                          |Single source of truth for types, validation, business logic. Share via Tauri’s type generation.|
|**KISS**                         |Prefer simple, obvious solutions. No premature abstraction. Complexity must earn its place.     |
|**Separation of Concerns**       |Clear boundaries: data fetching, parsing, storage, UI state, rendering.                         |
|**Fail Fast**                    |Validate at boundaries. Return `Result<T, E>` everywhere. No silent failures.                   |
|**Convention over Configuration**|Sensible defaults. Configuration is escape hatch, not requirement.                              |

### 1.2 Code Quality Standards

```
Test Coverage: 80% minimum (lines), 90% for core modules
Lint Errors: Zero tolerance
Type Safety: No `any` in TypeScript, no `unwrap()` in production Rust
Documentation: All public APIs documented
Complexity: Cyclomatic complexity < 10 per function
```

-----

## 2. Project Structure

### 2.1 Monorepo Layout

```
curio-reader/
├── .github/
│   ├── workflows/
│   │   ├── ci.yml              # Lint, test, build on PR
│   │   ├── release.yml         # Build + publish releases
│   │   └── security.yml        # Dependency audit
│   ├── ISSUE_TEMPLATE/
│   ├── PULL_REQUEST_TEMPLATE.md
│   └── CODEOWNERS
├── .husky/
│   ├── pre-commit              # Lint staged files
│   ├── pre-push                # Run tests
│   └── commit-msg              # Conventional commits
├── src-tauri/
│   ├── src/
│   │   ├── main.rs
│   │   ├── lib.rs
│   │   ├── commands/           # Tauri IPC handlers
│   │   ├── core/               # Business logic (platform-agnostic)
│   │   ├── platform/           # Platform-specific (YouTube, Reddit, Podcast)
│   │   ├── storage/            # Database + cache
│   │   ├── services/           # Background tasks
│   │   └── error.rs            # Unified error types
│   ├── tests/
│   │   ├── integration/
│   │   └── fixtures/
│   ├── Cargo.toml
│   └── tauri.conf.json
├── src/
│   ├── lib/
│   │   ├── api/                # Tauri command bindings
│   │   ├── stores/             # Svelte stores
│   │   ├── components/         # Reusable UI components
│   │   ├── views/              # View mode implementations
│   │   ├── utils/              # Pure utility functions
│   │   └── types/              # TypeScript types (generated + manual)
│   ├── routes/                 # SvelteKit routes (if using)
│   ├── app.html
│   ├── app.css
│   └── main.ts
├── tests/
│   ├── unit/
│   ├── integration/
│   └── e2e/
├── docs/
│   ├── architecture/
│   │   ├── overview.md
│   │   ├── data-flow.md
│   │   └── decisions/          # ADRs (Architecture Decision Records)
│   ├── api/
│   │   └── commands.md
│   └── contributing.md
├── scripts/
│   ├── setup.sh                # First-time dev setup
│   ├── gen-types.sh            # Regenerate TS types from Rust
│   └── bundle-ytdlp.sh         # Download/update yt-dlp binary
├── Makefile
├── package.json
├── svelte.config.js
├── vite.config.ts
├── tsconfig.json
├── biome.json                  # Linting + formatting (Biome)
├── rustfmt.toml
├── clippy.toml
├── lefthook.yml                # Git hooks (alternative to Husky)
├── LICENSE
├── README.md
└── CHANGELOG.md
```

### 2.2 Module Responsibilities

#### Rust Backend (`src-tauri/src/`)

|Module             |Responsibility                                                    |Dependencies      |
|-------------------|------------------------------------------------------------------|------------------|
|`commands/`        |Tauri IPC handlers. Thin layer, delegates to core.                |core, storage     |
|`core/`            |Pure business logic. No I/O, no Tauri deps. Testable in isolation.|(none)            |
|`core/feeds/`      |Feed parsing, normalization, OPML handling                        |feed-rs, quick-xml|
|`core/content/`    |HTML sanitization, readability extraction, Markdown export        |ammonia, htmd     |
|`core/models/`     |Domain types, validation, conversions                             |serde             |
|`platform/youtube/`|yt-dlp integration, metadata parsing                              |tokio::process    |
|`platform/reddit/` |Reddit JSON API client                                            |reqwest           |
|`platform/podcast/`|Audio metadata, download management                               |symphonia         |
|`storage/`         |SQLite operations, migrations, caching                            |rusqlite          |
|`storage/cache/`   |Image cache, LRU eviction                                         |tokio, image      |
|`services/`        |Background tasks: refresh, downloads, cleanup                     |tokio             |
|`error.rs`         |Unified error enum, conversions, user-facing messages             |thiserror         |

#### Svelte Frontend (`src/lib/`)

|Module            |Responsibility                                               |Dependencies   |
|------------------|-------------------------------------------------------------|---------------|
|`api/`            |Type-safe wrappers around Tauri invoke                       |@tauri-apps/api|
|`stores/`         |Reactive state management                                    |svelte/store   |
|`stores/feeds.ts` |Feed tree, articles, selection state                         |               |
|`stores/ui.ts`    |Theme, sidebar, view mode, modals                            |               |
|`stores/player.ts`|Podcast playback state                                       |               |
|`components/`     |Reusable, presentational components                          |               |
|`views/`          |View mode implementations (Article, Reddit, YouTube, Podcast)|               |
|`utils/`          |Pure functions: formatting, parsing, helpers                 |               |
|`types/`          |TypeScript types (most generated from Rust)                  |               |

-----

## 3. Architecture Patterns

### 3.1 Rust: Clean Architecture Layers

```
┌─────────────────────────────────────────────────────────────┐
│                     Commands Layer                           │
│   Tauri IPC handlers. Input validation, serialization.       │
│   No business logic. Delegates to Core.                      │
├─────────────────────────────────────────────────────────────┤
│                      Core Layer                              │
│   Pure business logic. Domain types, rules, transformations. │
│   No I/O, no external dependencies. 100% testable.           │
├─────────────────────────────────────────────────────────────┤
│                   Infrastructure Layer                       │
│   Storage (SQLite), HTTP clients, file I/O, yt-dlp process.  │
│   Implements traits defined in Core.                         │
└─────────────────────────────────────────────────────────────┘
```

### 3.2 Dependency Inversion

Core defines traits; infrastructure implements them:

```rust
// core/ports.rs - Core defines the interface
#[async_trait]
pub trait FeedRepository: Send + Sync {
    async fn get_feed(&self, id: &Uuid) -> Result<Feed, CoreError>;
    async fn save_feed(&self, feed: &Feed) -> Result<(), CoreError>;
    async fn list_feeds(&self, profile_id: &str) -> Result<Vec<Feed>, CoreError>;
    async fn delete_feed(&self, id: &Uuid) -> Result<(), CoreError>;
}

#[async_trait]
pub trait ArticleRepository: Send + Sync {
    async fn get_articles(&self, filter: &ArticleFilter, page: &Pagination) -> Result<ArticlePage, CoreError>;
    async fn upsert_articles(&self, articles: &[Article]) -> Result<UpsertResult, CoreError>;
    async fn mark_read(&self, ids: &[Uuid]) -> Result<(), CoreError>;
    async fn search(&self, query: &str, filter: &ArticleFilter) -> Result<Vec<Article>, CoreError>;
}

#[async_trait]
pub trait FeedFetcher: Send + Sync {
    async fn fetch(&self, url: &str, etag: Option<&str>) -> Result<FetchResult, CoreError>;
}

#[async_trait]
pub trait YouTubeEnricher: Send + Sync {
    async fn get_metadata(&self, video_url: &str) -> Result<YouTubeMetadata, CoreError>;
    async fn get_comments(&self, video_url: &str) -> Result<Vec<YouTubeComment>, CoreError>;
}

#[async_trait]
pub trait RedditEnricher: Send + Sync {
    async fn get_thread(&self, permalink: &str, sort: CommentSort) -> Result<RedditThread, CoreError>;
}

// storage/sqlite_feed_repo.rs - Infrastructure implements it
pub struct SqliteFeedRepository {
    pool: Arc<SqlitePool>,
}

#[async_trait]
impl FeedRepository for SqliteFeedRepository {
    async fn get_feed(&self, id: &Uuid) -> Result<Feed, CoreError> {
        // Implementation
    }
    // ...
}
```

### 3.3 Service Composition

```rust
// commands/mod.rs - Wire up dependencies
pub struct AppState {
    pub feed_repo: Arc<dyn FeedRepository>,
    pub article_repo: Arc<dyn ArticleRepository>,
    pub feed_fetcher: Arc<dyn FeedFetcher>,
    pub youtube: Arc<dyn YouTubeEnricher>,
    pub reddit: Arc<dyn RedditEnricher>,
    pub settings: Arc<RwLock<Settings>>,
}

impl AppState {
    pub fn new(db_path: &Path) -> Result<Self, SetupError> {
        let pool = Arc::new(SqlitePool::new(db_path)?);
        
        Ok(Self {
            feed_repo: Arc::new(SqliteFeedRepository::new(pool.clone())),
            article_repo: Arc::new(SqliteArticleRepository::new(pool.clone())),
            feed_fetcher: Arc::new(HttpFeedFetcher::new()),
            youtube: Arc::new(YtDlpEnricher::new(ytdlp_path())?),
            reddit: Arc::new(RedditJsonClient::new()),
            settings: Arc::new(RwLock::new(Settings::load_or_default()?)),
        })
    }
}

// Commands are thin wrappers
#[tauri::command]
pub async fn refresh_feed(
    state: State<'_, AppState>,
    feed_id: Uuid,
) -> Result<RefreshResult, CommandError> {
    let feed = state.feed_repo.get_feed(&feed_id).await?;
    let result = state.feed_fetcher.fetch(&feed.url, feed.etag.as_deref()).await?;
    
    if let Some(new_articles) = result.articles {
        let parsed = core::feeds::parse_feed(&result.body, &feed)?;
        state.article_repo.upsert_articles(&parsed.articles).await?;
    }
    
    Ok(result.into())
}
```

### 3.4 Error Handling Strategy

```rust
// error.rs - Unified error hierarchy

/// Core errors - business logic failures
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Feed not found: {0}")]
    FeedNotFound(Uuid),
    
    #[error("Invalid feed URL: {0}")]
    InvalidFeedUrl(String),
    
    #[error("Parse error: {0}")]
    ParseError(String),
    
    #[error("Validation error: {0}")]
    ValidationError(String),
}

/// Infrastructure errors - I/O and external failures
#[derive(Debug, thiserror::Error)]
pub enum InfraError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    
    #[error("yt-dlp error: {0}")]
    YtDlp(String),
    
    #[error("File I/O error: {0}")]
    Io(#[from] std::io::Error),
    
    #[error("Rate limited, retry after {retry_after:?}")]
    RateLimited { retry_after: Option<Duration> },
}

/// Command errors - user-facing, serializable
#[derive(Debug, Serialize, thiserror::Error)]
pub enum CommandError {
    #[error("{message}")]
    User { 
        message: String,
        code: ErrorCode,
        recoverable: bool,
    },
    
    #[error("Internal error")]
    Internal {
        message: String,
        #[serde(skip)]
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    },
}

#[derive(Debug, Serialize, Clone, Copy)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    NetworkError,
    RateLimited,
    ParseError,
    StorageError,
    ExternalServiceError,
}

// Conversions maintain context
impl From<CoreError> for CommandError {
    fn from(e: CoreError) -> Self {
        match e {
            CoreError::FeedNotFound(id) => CommandError::User {
                message: format!("Feed not found: {}", id),
                code: ErrorCode::NotFound,
                recoverable: false,
            },
            CoreError::InvalidFeedUrl(url) => CommandError::User {
                message: format!("Invalid feed URL: {}", url),
                code: ErrorCode::InvalidInput,
                recoverable: true,
            },
            // ...
        }
    }
}
```

### 3.5 Svelte State Management

```typescript
// stores/feeds.ts - Reactive feed state with derived stores

import { writable, derived, get } from 'svelte/store';
import type { Feed, Article, Folder, ArticleFilter } from '$lib/types';
import * as api from '$lib/api';

// Base stores (source of truth)
export const feeds = writable<Map<string, Feed>>(new Map());
export const folders = writable<Map<string, Folder>>(new Map());
export const articles = writable<Map<string, Article>>(new Map());
export const selectedFeedId = writable<string | null>(null);
export const selectedArticleId = writable<string | null>(null);
export const articleFilter = writable<ArticleFilter>({ unreadOnly: false });

// Derived stores (computed views)
export const folderTree = derived(
    [folders, feeds],
    ([$folders, $feeds]) => buildFolderTree($folders, $feeds)
);

export const currentArticles = derived(
    [articles, selectedFeedId, articleFilter],
    ([$articles, $feedId, $filter]) => {
        let list = Array.from($articles.values());
        
        if ($feedId) {
            list = list.filter(a => a.feedId === $feedId);
        }
        if ($filter.unreadOnly) {
            list = list.filter(a => !a.isRead);
        }
        
        return list.sort((a, b) => 
            new Date(b.publishedAt).getTime() - new Date(a.publishedAt).getTime()
        );
    }
);

export const unreadCounts = derived(
    [articles, feeds],
    ([$articles, $feeds]) => {
        const counts = new Map<string, number>();
        for (const [feedId, _] of $feeds) {
            counts.set(
                feedId,
                Array.from($articles.values())
                    .filter(a => a.feedId === feedId && !a.isRead)
                    .length
            );
        }
        return counts;
    }
);

// Actions (modify state via API, then update stores)
export async function refreshFeed(feedId: string): Promise<void> {
    const result = await api.refreshFeed(feedId);
    
    if (result.newArticles.length > 0) {
        articles.update(map => {
            for (const article of result.newArticles) {
                map.set(article.id, article);
            }
            return map;
        });
    }
    
    feeds.update(map => {
        const feed = map.get(feedId);
        if (feed) {
            map.set(feedId, { ...feed, lastFetched: new Date().toISOString() });
        }
        return map;
    });
}

export async function markRead(articleIds: string[]): Promise<void> {
    await api.markRead(articleIds);
    
    articles.update(map => {
        for (const id of articleIds) {
            const article = map.get(id);
            if (article) {
                map.set(id, { ...article, isRead: true, readAt: new Date().toISOString() });
            }
        }
        return map;
    });
}
```

-----

## 4. Type Safety & Code Generation

### 4.1 Rust → TypeScript Type Generation

Use `ts-rs` to generate TypeScript types from Rust structs:

```rust
// core/models/feed.rs
use ts_rs::TS;

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../src/lib/types/generated/")]
pub struct Feed {
    pub id: Uuid,
    pub url: String,
    pub title: String,
    pub description: Option<String>,
    #[ts(type = "string")] // Custom type mapping
    pub view_mode: ViewMode,
    pub tags: Vec<String>,
    pub folder_id: Option<Uuid>,
    // ...
}

#[derive(Debug, Clone, Serialize, Deserialize, TS)]
#[ts(export, export_to = "../src/lib/types/generated/")]
pub struct Article {
    pub id: Uuid,
    pub feed_id: Uuid,
    pub title: String,
    // ...
}
```

```bash
# scripts/gen-types.sh
#!/bin/bash
set -e

cd src-tauri
cargo test export_bindings_ --features ts-rs

echo "Types generated in src/lib/types/generated/"
```

### 4.2 API Type Safety

```typescript
// src/lib/api/index.ts - Type-safe Tauri command wrappers
import { invoke } from '@tauri-apps/api/core';
import type { 
    Feed, Article, Folder, 
    ArticleFilter, Pagination, ArticlePage,
    RefreshResult, CommandError 
} from '$lib/types';

// Generic invoke wrapper with error handling
async function typedInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
        return await invoke<T>(cmd, args);
    } catch (error) {
        // Error is already CommandError from Rust
        throw error as CommandError;
    }
}

// Feed operations
export const getFeeds = () => 
    typedInvoke<Feed[]>('get_feeds');

export const addFeed = (url: string, folderId?: string, tags?: string[]) =>
    typedInvoke<Feed>('add_feed', { url, folderId, tags });

export const refreshFeed = (feedId: string) =>
    typedInvoke<RefreshResult>('refresh_feed', { feedId });

export const refreshAllFeeds = () =>
    typedInvoke<RefreshResult[]>('refresh_all_feeds');

// Article operations
export const getArticles = (filter: ArticleFilter, pagination: Pagination) =>
    typedInvoke<ArticlePage>('get_articles', { filter, pagination });

export const markRead = (articleIds: string[]) =>
    typedInvoke<void>('mark_read', { articleIds });

export const markUnread = (articleIds: string[]) =>
    typedInvoke<void>('mark_unread', { articleIds });

export const toggleStarred = (articleId: string) =>
    typedInvoke<boolean>('toggle_starred', { articleId });

// YouTube (via yt-dlp)
export const fetchYoutubeMetadata = (videoUrl: string) =>
    typedInvoke<YtDlpMetadata>('fetch_youtube_metadata', { videoUrl });

export const fetchYoutubeComments = (videoUrl: string) =>
    typedInvoke<YtDlpComment[]>('fetch_youtube_comments', { videoUrl });

// Reddit
export const fetchRedditThread = (permalink: string) =>
    typedInvoke<RedditThread>('fetch_reddit_thread', { permalink });
```

-----

## 5. Testing Strategy

### 5.1 Test Pyramid

```
                    ┌─────────────┐
                    │    E2E      │  5%  - Critical user journeys
                    │  (Playwright)│
                ┌───┴─────────────┴───┐
                │    Integration      │  25% - API boundaries, DB
                │   (Rust + Vitest)   │
            ┌───┴─────────────────────┴───┐
            │         Unit Tests          │  70% - Core logic, utils
            │      (Rust + Vitest)        │
            └─────────────────────────────┘
```

### 5.2 Rust Testing

```rust
// core/feeds/parser_test.rs
#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;
    
    #[test]
    fn parse_rss_2_feed() {
        let xml = fixtures::load("rss2_simple.xml");
        let result = parse_feed(&xml, FeedFormat::Rss2).unwrap();
        
        assert_eq!(result.title, "Example Feed");
        assert_eq!(result.articles.len(), 3);
        assert!(result.articles[0].published_at.is_some());
    }
    
    #[test]
    fn parse_atom_feed() {
        let xml = fixtures::load("atom_simple.xml");
        let result = parse_feed(&xml, FeedFormat::Atom).unwrap();
        
        assert_eq!(result.title, "Example Atom Feed");
    }
    
    #[test]
    fn parse_youtube_rss() {
        let xml = fixtures::load("youtube_channel.xml");
        let result = parse_feed(&xml, FeedFormat::Atom).unwrap();
        
        // Should extract video ID from entry ID
        assert!(result.articles[0].guid.starts_with("yt:video:"));
    }
    
    #[test]
    fn parse_reddit_rss() {
        let xml = fixtures::load("reddit_subreddit.xml");
        let result = parse_feed(&xml, FeedFormat::Atom).unwrap();
        
        // Should extract post ID
        assert!(result.articles[0].url.unwrap().contains("/comments/"));
    }
    
    #[test]
    fn handles_malformed_xml_gracefully() {
        let xml = "not valid xml at all";
        let result = parse_feed(xml, FeedFormat::Rss2);
        
        assert!(matches!(result, Err(CoreError::ParseError(_))));
    }
}

// Integration tests with real database
// tests/integration/feed_refresh_test.rs
#[tokio::test]
async fn refresh_feed_stores_new_articles() {
    let db = TestDb::new().await;
    let state = test_app_state(db.pool()).await;
    
    // Seed a feed
    let feed = state.feed_repo.save_feed(&Feed {
        id: Uuid::new_v4(),
        url: "https://example.com/feed.xml".into(),
        title: "Test Feed".into(),
        ..Default::default()
    }).await.unwrap();
    
    // Mock the fetcher to return test data
    state.feed_fetcher.mock_response(
        &feed.url,
        fixtures::load("rss2_simple.xml"),
    );
    
    // Execute
    let result = refresh_feed(State(&state), feed.id).await.unwrap();
    
    // Verify
    assert_eq!(result.new_count, 3);
    
    let articles = state.article_repo
        .get_articles(&ArticleFilter { feed_id: Some(feed.id), ..Default::default() }, &Pagination::default())
        .await
        .unwrap();
    
    assert_eq!(articles.items.len(), 3);
}
```

### 5.3 Frontend Testing

```typescript
// tests/unit/stores/feeds.test.ts
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { get } from 'svelte/store';
import { feeds, articles, currentArticles, unreadCounts, markRead } from '$lib/stores/feeds';
import * as api from '$lib/api';

// Mock the API module
vi.mock('$lib/api');

describe('feeds store', () => {
    beforeEach(() => {
        feeds.set(new Map());
        articles.set(new Map());
        vi.resetAllMocks();
    });
    
    describe('currentArticles', () => {
        it('filters by selected feed', () => {
            articles.set(new Map([
                ['1', { id: '1', feedId: 'feed-a', isRead: false, publishedAt: '2025-01-01' }],
                ['2', { id: '2', feedId: 'feed-b', isRead: false, publishedAt: '2025-01-02' }],
                ['3', { id: '3', feedId: 'feed-a', isRead: false, publishedAt: '2025-01-03' }],
            ]));
            
            selectedFeedId.set('feed-a');
            
            const current = get(currentArticles);
            expect(current).toHaveLength(2);
            expect(current.every(a => a.feedId === 'feed-a')).toBe(true);
        });
        
        it('sorts by publish date descending', () => {
            articles.set(new Map([
                ['1', { id: '1', feedId: 'feed-a', publishedAt: '2025-01-01' }],
                ['2', { id: '2', feedId: 'feed-a', publishedAt: '2025-01-03' }],
                ['3', { id: '3', feedId: 'feed-a', publishedAt: '2025-01-02' }],
            ]));
            
            selectedFeedId.set('feed-a');
            
            const current = get(currentArticles);
            expect(current.map(a => a.id)).toEqual(['2', '3', '1']);
        });
    });
    
    describe('markRead', () => {
        it('updates local state optimistically', async () => {
            vi.mocked(api.markRead).mockResolvedValue(undefined);
            
            articles.set(new Map([
                ['1', { id: '1', feedId: 'feed-a', isRead: false }],
            ]));
            
            await markRead(['1']);
            
            const article = get(articles).get('1');
            expect(article?.isRead).toBe(true);
            expect(article?.readAt).toBeDefined();
        });
    });
});

// tests/unit/components/ArticleCard.test.ts
import { render, screen } from '@testing-library/svelte';
import { userEvent } from '@testing-library/user-event';
import ArticleCard from '$lib/components/ArticleCard.svelte';

describe('ArticleCard', () => {
    const article = {
        id: '1',
        title: 'Test Article',
        feedId: 'feed-1',
        feedTitle: 'Test Feed',
        publishedAt: '2025-01-15T10:00:00Z',
        isRead: false,
        isStarred: false,
    };
    
    it('renders article title and feed name', () => {
        render(ArticleCard, { article });
        
        expect(screen.getByText('Test Article')).toBeInTheDocument();
        expect(screen.getByText('Test Feed')).toBeInTheDocument();
    });
    
    it('shows unread indicator when unread', () => {
        render(ArticleCard, { article: { ...article, isRead: false } });
        
        expect(screen.getByTestId('unread-indicator')).toBeInTheDocument();
    });
    
    it('emits select event on click', async () => {
        const user = userEvent.setup();
        const { component } = render(ArticleCard, { article });
        
        const selectHandler = vi.fn();
        component.$on('select', selectHandler);
        
        await user.click(screen.getByRole('article'));
        
        expect(selectHandler).toHaveBeenCalledWith(
            expect.objectContaining({ detail: article })
        );
    });
});
```

### 5.4 E2E Testing

```typescript
// tests/e2e/feed-management.spec.ts
import { test, expect } from '@playwright/test';

test.describe('Feed Management', () => {
    test.beforeEach(async ({ page }) => {
        // Start with clean state
        await page.evaluate(() => window.__TAURI_TEST_RESET__());
    });
    
    test('add feed via URL', async ({ page }) => {
        await page.goto('/');
        
        // Open add feed modal
        await page.click('[data-testid="add-feed-button"]');
        
        // Enter feed URL
        await page.fill('[data-testid="feed-url-input"]', 'https://example.com/feed.xml');
        await page.click('[data-testid="add-feed-submit"]');
        
        // Verify feed appears in sidebar
        await expect(page.locator('[data-testid="feed-list"]')).toContainText('Example Feed');
    });
    
    test('refresh all feeds', async ({ page }) => {
        await page.goto('/');
        
        // Seed some feeds
        await page.evaluate(async () => {
            await window.__TAURI_TEST_SEED_FEEDS__([
                { url: 'https://example1.com/feed.xml', title: 'Feed 1' },
                { url: 'https://example2.com/feed.xml', title: 'Feed 2' },
            ]);
        });
        
        // Click refresh all
        await page.click('[data-testid="refresh-all-button"]');
        
        // Wait for refresh to complete
        await expect(page.locator('[data-testid="refresh-progress"]')).toBeHidden({ timeout: 30000 });
        
        // Verify unread counts updated
        await expect(page.locator('[data-testid="unread-badge"]').first()).toHaveText(/\d+/);
    });
});
```

### 5.5 Coverage Configuration

```toml
# src-tauri/Cargo.toml
[dev-dependencies]
cargo-llvm-cov = "0.5"

# .cargo/config.toml
[alias]
cov = "llvm-cov --html --output-dir coverage/rust"
cov-ci = "llvm-cov --lcov --output-path coverage/rust/lcov.info"
```

```javascript
// vitest.config.ts
import { defineConfig } from 'vitest/config';
import { svelte } from '@sveltejs/vite-plugin-svelte';

export default defineConfig({
    plugins: [svelte({ hot: false })],
    test: {
        include: ['tests/unit/**/*.test.ts', 'src/**/*.test.ts'],
        environment: 'jsdom',
        coverage: {
            provider: 'v8',
            reporter: ['text', 'lcov', 'html'],
            reportsDirectory: 'coverage/frontend',
            exclude: [
                'node_modules/',
                'tests/',
                '**/*.d.ts',
                'src/lib/types/generated/**',
            ],
            thresholds: {
                lines: 80,
                branches: 75,
                functions: 80,
                statements: 80,
            },
        },
    },
});
```

-----

## 6. Developer Experience

### 6.1 Makefile

```makefile
# Makefile

.PHONY: help setup dev build test lint fmt clean release

# Default target
help:
	@echo "Curio Reader Development Commands"
	@echo ""
	@echo "Setup:"
	@echo "  make setup       - First-time development setup"
	@echo "  make deps        - Install/update dependencies"
	@echo ""
	@echo "Development:"
	@echo "  make dev         - Start development server"
	@echo "  make build       - Build for production"
	@echo "  make build-debug - Build debug version"
	@echo ""
	@echo "Quality:"
	@echo "  make test        - Run all tests"
	@echo "  make test-rust   - Run Rust tests only"
	@echo "  make test-ts     - Run TypeScript tests only"
	@echo "  make test-e2e    - Run E2E tests"
	@echo "  make cov         - Generate coverage report"
	@echo "  make lint        - Run all linters"
	@echo "  make lint-fix    - Run linters and fix issues"
	@echo "  make fmt         - Format all code"
	@echo "  make check       - Run all checks (lint + test)"
	@echo ""
	@echo "Utilities:"
	@echo "  make gen-types   - Regenerate TypeScript types from Rust"
	@echo "  make db-reset    - Reset development database"
	@echo "  make clean       - Clean build artifacts"
	@echo "  make ytdlp-update - Update bundled yt-dlp"
	@echo ""
	@echo "Release:"
	@echo "  make release     - Build release for all platforms"
	@echo "  make release-mac - Build macOS release"
	@echo "  make release-win - Build Windows release"
	@echo "  make release-linux - Build Linux release"

# ─────────────────────────────────────────────────────────────
# Setup
# ─────────────────────────────────────────────────────────────

setup: deps setup-hooks setup-ytdlp gen-types
	@echo "✅ Development environment ready!"
	@echo "Run 'make dev' to start developing."

deps:
	@echo "📦 Installing dependencies..."
	pnpm install
	cd src-tauri && cargo fetch

setup-hooks:
	@echo "🪝 Setting up git hooks..."
	pnpm exec lefthook install

setup-ytdlp:
	@echo "📺 Setting up yt-dlp..."
	./scripts/bundle-ytdlp.sh

# ─────────────────────────────────────────────────────────────
# Development
# ─────────────────────────────────────────────────────────────

dev:
	pnpm tauri dev

build:
	pnpm tauri build

build-debug:
	pnpm tauri build --debug

# ─────────────────────────────────────────────────────────────
# Testing
# ─────────────────────────────────────────────────────────────

test: test-rust test-ts

test-rust:
	@echo "🦀 Running Rust tests..."
	cd src-tauri && cargo test

test-ts:
	@echo "📜 Running TypeScript tests..."
	pnpm test

test-e2e:
	@echo "🎭 Running E2E tests..."
	pnpm test:e2e

test-watch:
	pnpm test:watch

cov: cov-rust cov-ts
	@echo "📊 Coverage reports generated in ./coverage/"

cov-rust:
	@echo "🦀 Generating Rust coverage..."
	cd src-tauri && cargo llvm-cov --html --output-dir ../coverage/rust

cov-ts:
	@echo "📜 Generating TypeScript coverage..."
	pnpm test:coverage

# ─────────────────────────────────────────────────────────────
# Code Quality
# ─────────────────────────────────────────────────────────────

lint: lint-rust lint-ts

lint-rust:
	@echo "🦀 Linting Rust..."
	cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings

lint-ts:
	@echo "📜 Linting TypeScript/Svelte..."
	pnpm exec biome check .

lint-fix: lint-fix-rust lint-fix-ts

lint-fix-rust:
	cd src-tauri && cargo clippy --fix --allow-dirty --allow-staged

lint-fix-ts:
	pnpm exec biome check --write .

fmt: fmt-rust fmt-ts

fmt-rust:
	@echo "🦀 Formatting Rust..."
	cd src-tauri && cargo fmt

fmt-ts:
	@echo "📜 Formatting TypeScript/Svelte..."
	pnpm exec biome format --write .

check: lint test
	@echo "✅ All checks passed!"

# Pre-commit hook target (fast checks only)
pre-commit: fmt lint-staged

lint-staged:
	pnpm exec lint-staged

# ─────────────────────────────────────────────────────────────
# Utilities
# ─────────────────────────────────────────────────────────────

gen-types:
	@echo "🔄 Generating TypeScript types from Rust..."
	./scripts/gen-types.sh

db-reset:
	@echo "🗄️ Resetting development database..."
	rm -f ~/.config/curio-reader-dev/curio.db
	@echo "Database reset. Will be recreated on next run."

clean:
	@echo "🧹 Cleaning build artifacts..."
	rm -rf target/
	rm -rf dist/
	rm -rf node_modules/.vite
	rm -rf coverage/

ytdlp-update:
	@echo "📺 Updating yt-dlp..."
	./scripts/bundle-ytdlp.sh --force

# ─────────────────────────────────────────────────────────────
# Release
# ─────────────────────────────────────────────────────────────

release:
	pnpm tauri build --release

release-mac:
	pnpm tauri build --target universal-apple-darwin

release-win:
	pnpm tauri build --target x86_64-pc-windows-msvc

release-linux:
	pnpm tauri build --target x86_64-unknown-linux-gnu

# ─────────────────────────────────────────────────────────────
# Documentation
# ─────────────────────────────────────────────────────────────

docs:
	@echo "📚 Generating documentation..."
	cd src-tauri && cargo doc --no-deps --open

docs-api:
	@echo "📚 Generating API documentation..."
	pnpm exec typedoc
```

### 6.2 Git Hooks (Lefthook)

```yaml
# lefthook.yml

pre-commit:
  parallel: true
  commands:
    rust-fmt:
      glob: "src-tauri/**/*.rs"
      run: cd src-tauri && cargo fmt -- --check
      
    rust-clippy:
      glob: "src-tauri/**/*.rs"
      run: cd src-tauri && cargo clippy --all-targets -- -D warnings
      
    biome-check:
      glob: "*.{ts,js,svelte,json}"
      run: pnpm exec biome check --staged
      
    typescript:
      glob: "*.{ts,svelte}"
      run: pnpm exec tsc --noEmit

pre-push:
  parallel: true
  commands:
    rust-tests:
      run: cd src-tauri && cargo test
      
    ts-tests:
      run: pnpm test

commit-msg:
  commands:
    conventional:
      run: |
        if ! echo "{1}" | grep -qE "^(feat|fix|docs|style|refactor|perf|test|chore|ci|build|revert)(\(.+\))?: .{1,100}$"; then
          echo "❌ Commit message must follow Conventional Commits format:"
          echo "   type(scope): description"
          echo ""
          echo "   Types: feat, fix, docs, style, refactor, perf, test, chore, ci, build, revert"
          exit 1
        fi
```

### 6.3 CI/CD Pipeline

```yaml
# .github/workflows/ci.yml

name: CI

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]

env:
  CARGO_TERM_COLOR: always
  RUST_BACKTRACE: 1

jobs:
  # ─────────────────────────────────────────────────────────────
  # Rust Checks
  # ─────────────────────────────────────────────────────────────
  rust:
    name: Rust ${{ matrix.check }}
    runs-on: ubuntu-latest
    strategy:
      matrix:
        check: [fmt, clippy, test]
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust toolchain
        uses: dtolnay/rust-action@stable
        with:
          components: rustfmt, clippy
          
      - name: Cache cargo
        uses: Swatinem/rust-cache@v2
        with:
          workspaces: src-tauri
          
      - name: Install system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev
          
      - name: Check formatting
        if: matrix.check == 'fmt'
        run: cd src-tauri && cargo fmt -- --check
        
      - name: Clippy
        if: matrix.check == 'clippy'
        run: cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
        
      - name: Tests
        if: matrix.check == 'test'
        run: cd src-tauri && cargo test --all-features

  rust-coverage:
    name: Rust Coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Install Rust toolchain
        uses: dtolnay/rust-action@stable
        with:
          components: llvm-tools-preview
          
      - name: Install cargo-llvm-cov
        uses: taiki-e/install-action@cargo-llvm-cov
        
      - name: Install system dependencies
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev
          
      - name: Generate coverage
        run: cd src-tauri && cargo llvm-cov --all-features --lcov --output-path lcov.info
        
      - name: Upload to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: src-tauri/lcov.info
          flags: rust
          fail_ci_if_error: true

  # ─────────────────────────────────────────────────────────────
  # Frontend Checks
  # ─────────────────────────────────────────────────────────────
  frontend:
    name: Frontend ${{ matrix.check }}
    runs-on: ubuntu-latest
    strategy:
      matrix:
        check: [lint, typecheck, test]
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup pnpm
        uses: pnpm/action-setup@v3
        with:
          version: 9
          
      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
        
      - name: Lint
        if: matrix.check == 'lint'
        run: pnpm exec biome check .
        
      - name: Typecheck
        if: matrix.check == 'typecheck'
        run: pnpm exec tsc --noEmit
        
      - name: Test
        if: matrix.check == 'test'
        run: pnpm test

  frontend-coverage:
    name: Frontend Coverage
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup pnpm
        uses: pnpm/action-setup@v3
        with:
          version: 9
          
      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
        
      - name: Test with coverage
        run: pnpm test:coverage
        
      - name: Upload to Codecov
        uses: codecov/codecov-action@v4
        with:
          files: coverage/frontend/lcov.info
          flags: frontend
          fail_ci_if_error: true

  # ─────────────────────────────────────────────────────────────
  # Build Verification
  # ─────────────────────────────────────────────────────────────
  build:
    name: Build (${{ matrix.os }})
    needs: [rust, frontend]
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup pnpm
        uses: pnpm/action-setup@v3
        with:
          version: 9
          
      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          
      - name: Install Rust toolchain
        uses: dtolnay/rust-action@stable
        
      - name: Install system dependencies (Linux)
        if: matrix.os == 'ubuntu-latest'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev
          
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
        
      - name: Build
        run: pnpm tauri build
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          
      - name: Upload artifacts
        uses: actions/upload-artifact@v4
        with:
          name: build-${{ matrix.os }}
          path: |
            src-tauri/target/release/bundle/
          retention-days: 7

  # ─────────────────────────────────────────────────────────────
  # E2E Tests
  # ─────────────────────────────────────────────────────────────
  e2e:
    name: E2E Tests
    needs: [build]
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup pnpm
        uses: pnpm/action-setup@v3
        with:
          version: 9
          
      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
        
      - name: Install Playwright browsers
        run: pnpm exec playwright install --with-deps chromium
        
      - name: Download build artifact
        uses: actions/download-artifact@v4
        with:
          name: build-ubuntu-latest
          path: src-tauri/target/release/bundle/
          
      - name: Run E2E tests
        run: pnpm test:e2e
        
      - name: Upload test results
        if: failure()
        uses: actions/upload-artifact@v4
        with:
          name: e2e-results
          path: test-results/
```

```yaml
# .github/workflows/release.yml

name: Release

on:
  push:
    tags:
      - 'v*'

permissions:
  contents: write

jobs:
  release:
    name: Release (${{ matrix.target }})
    runs-on: ${{ matrix.os }}
    strategy:
      matrix:
        include:
          - os: macos-latest
            target: universal-apple-darwin
            name: macOS
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
            name: Linux
          - os: windows-latest
            target: x86_64-pc-windows-msvc
            name: Windows
    steps:
      - uses: actions/checkout@v4
      
      - name: Setup pnpm
        uses: pnpm/action-setup@v3
        with:
          version: 9
          
      - name: Setup Node
        uses: actions/setup-node@v4
        with:
          node-version: 20
          cache: pnpm
          
      - name: Install Rust toolchain
        uses: dtolnay/rust-action@stable
        with:
          targets: ${{ matrix.target }}
          
      - name: Install system dependencies (Linux)
        if: matrix.os == 'ubuntu-latest'
        run: |
          sudo apt-get update
          sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev
          
      - name: Install dependencies
        run: pnpm install --frozen-lockfile
        
      - name: Bundle yt-dlp
        run: ./scripts/bundle-ytdlp.sh
        
      - name: Build release
        run: pnpm tauri build --target ${{ matrix.target }}
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}
          
      - name: Upload release assets
        uses: softprops/action-gh-release@v1
        with:
          files: |
            src-tauri/target/${{ matrix.target }}/release/bundle/**/*
          draft: true
```

### 6.4 Linting & Formatting Configuration

```toml
# rustfmt.toml
edition = "2021"
max_width = 100
tab_spaces = 4
use_small_heuristics = "Default"
imports_granularity = "Module"
group_imports = "StdExternalCrate"
reorder_imports = true
reorder_modules = true
```

```toml
# clippy.toml
cognitive-complexity-threshold = 10
too-many-arguments-threshold = 7
type-complexity-threshold = 250
```

```json
// biome.json
{
  "$schema": "https://biomejs.dev/schemas/1.9.0/schema.json",
  "organizeImports": {
    "enabled": true
  },
  "linter": {
    "enabled": true,
    "rules": {
      "recommended": true,
      "complexity": {
        "noBannedTypes": "error",
        "noExcessiveCognitiveComplexity": {
          "level": "warn",
          "options": { "maxAllowedComplexity": 10 }
        }
      },
      "correctness": {
        "noUnusedImports": "error",
        "noUnusedVariables": "error",
        "useExhaustiveDependencies": "warn"
      },
      "style": {
        "noNonNullAssertion": "warn",
        "useConst": "error",
        "useImportType": "error"
      },
      "suspicious": {
        "noExplicitAny": "error",
        "noConsoleLog": "warn"
      }
    }
  },
  "formatter": {
    "enabled": true,
    "indentStyle": "space",
    "indentWidth": 4,
    "lineWidth": 100
  },
  "javascript": {
    "formatter": {
      "quoteStyle": "single",
      "trailingCommas": "all",
      "semicolons": "always"
    }
  },
  "files": {
    "ignore": [
      "node_modules",
      "dist",
      "target",
      "coverage",
      "src/lib/types/generated"
    ]
  }
}
```

```json
// tsconfig.json
{
  "extends": "./.svelte-kit/tsconfig.json",
  "compilerOptions": {
    "strict": true,
    "noImplicitAny": true,
    "strictNullChecks": true,
    "strictFunctionTypes": true,
    "strictBindCallApply": true,
    "strictPropertyInitialization": true,
    "noImplicitThis": true,
    "useUnknownInCatchVariables": true,
    "alwaysStrict": true,
    "noUnusedLocals": true,
    "noUnusedParameters": true,
    "exactOptionalPropertyTypes": true,
    "noImplicitReturns": true,
    "noFallthroughCasesInSwitch": true,
    "noUncheckedIndexedAccess": true,
    "noImplicitOverride": true,
    "noPropertyAccessFromIndexSignature": true,
    "allowUnusedLabels": false,
    "allowUnreachableCode": false,
    "moduleResolution": "bundler",
    "target": "ES2022",
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "paths": {
      "$lib/*": ["./src/lib/*"],
      "$components/*": ["./src/lib/components/*"]
    }
  },
  "include": ["src/**/*", "tests/**/*"],
  "exclude": ["node_modules", "src/lib/types/generated"]
}
```

-----

## 7. Documentation Standards

### 7.1 Code Documentation

**Rust:**

```rust
/// Parses an RSS or Atom feed from XML content.
///
/// Automatically detects feed format and extracts articles with normalized fields.
/// Handles various edge cases including malformed dates, missing fields, and
/// platform-specific extensions (YouTube, Reddit).
///
/// # Arguments
///
/// * `content` - Raw XML content of the feed
/// * `source_url` - Original URL (used for resolving relative links)
///
/// # Returns
///
/// A `ParsedFeed` containing feed metadata and articles, or a `ParseError`
/// if the content cannot be parsed.
///
/// # Examples
///
/// ```rust
/// let xml = r#"<?xml version="1.0"?>
///     <rss version="2.0">
///         <channel><title>My Feed</title></channel>
///     </rss>"#;
///
/// let feed = parse_feed(xml, "https://example.com/feed.xml")?;
/// assert_eq!(feed.title, "My Feed");
/// ```
///
/// # Errors
///
/// Returns `ParseError::InvalidXml` if the content is not valid XML.
/// Returns `ParseError::UnsupportedFormat` if the feed format is not recognized.
pub fn parse_feed(content: &str, source_url: &str) -> Result<ParsedFeed, ParseError> {
    // ...
}
```

**TypeScript:**

```typescript
/**
 * Fetches articles with filtering and pagination.
 * 
 * @param filter - Filter criteria for articles
 * @param pagination - Pagination parameters
 * @returns Paginated list of articles matching the filter
 * @throws {CommandError} If the backend request fails
 * 
 * @example
 * ```typescript
 * const page = await getArticles(
 *   { unreadOnly: true, feedId: 'abc-123' },
 *   { page: 1, perPage: 50 }
 * );
 * console.log(`Found ${page.total} articles`);
 * ```
 */
export async function getArticles(
    filter: ArticleFilter,
    pagination: Pagination,
): Promise<ArticlePage> {
    return typedInvoke('get_articles', { filter, pagination });
}
```

### 7.2 Architecture Decision Records (ADRs)

```markdown
# ADR-001: Use yt-dlp Instead of YouTube Data API

## Status
Accepted

## Context
We need to fetch YouTube video metadata and comments for the YouTube view mode.
Options considered:
1. YouTube Data API v3 (official API)
2. yt-dlp (open-source extraction tool)
3. Invidious API (third-party proxy)

## Decision
Use yt-dlp bundled with the application.

## Rationale
- **No API key required**: Users don't need to set up credentials
- **No quota limits**: YouTube API has strict daily quotas (10k units)
- **Richer data**: yt-dlp extracts more metadata than the official API
- **Comment extraction**: Gets all comments without pagination limits
- **Multi-platform**: Works with Vimeo, Twitch, etc. automatically
- **Active maintenance**: Quick updates when YouTube changes

## Consequences
- Binary size increases (~20MB)
- Need to implement auto-update mechanism
- Must handle process spawning and JSON parsing
- Potential fragility if YouTube blocks yt-dlp

## Alternatives Rejected
- **YouTube API**: Quota limits, requires user API key, less data
- **Invidious**: Third-party dependency, inconsistent availability
```

### 7.3 README Structure

```markdown
# Curio Reader

A fast, themeable RSS reader with native YouTube, Reddit, and podcast experiences.

[Screenshot]

## Features

- 📰 **RSS-first**: Subscribe to 500+ feeds with sub-second refresh
- 📺 **YouTube integration**: Watch videos, read comments, all in-app
- 💬 **Reddit threads**: Browse posts and comments without leaving
- 🎧 **Podcast player**: Queue, download, playback speed control
- 🎨 **Themeable**: Full customization, transparency, blur effects
- 📁 **Profiles**: Organize feeds into separate contexts
- 📝 **Markdown export**: Clean exports for Obsidian, Logseq

## Installation

### macOS
```bash
brew install --cask curio-reader
```

### Windows

Download the latest `.msi` from [Releases](https://github.com/xxx/curio-reader/releases).

### Linux

```bash
# Debian/Ubuntu
sudo dpkg -i curio-reader_*.deb

# Fedora
sudo rpm -i curio-reader-*.rpm

# AppImage
chmod +x Curio-Reader-*.AppImage
./Curio-Reader-*.AppImage
```

## Development

### Prerequisites

- Rust 1.75+
- Node.js 20+
- pnpm 9+

### Setup

```bash
git clone https://github.com/xxx/curio-reader
cd curio-reader
make setup
```

### Development server

```bash
make dev
```

### Run tests

```bash
make test
```

### Build for production

```bash
make build
```

## Documentation

- [Architecture Overview](docs/architecture/overview.md)
- [Contributing Guide](docs/contributing.md)
- [API Reference](docs/api/commands.md)

## License

MIT

```
---

## 8. Performance Guidelines

### 8.1 Rust Performance

```rust
// DO: Use iterators, avoid allocations in hot paths
pub fn count_unread(articles: &[Article]) -> usize {
    articles.iter().filter(|a| !a.is_read).count()
}

// DON'T: Collect into intermediate vectors
pub fn count_unread_bad(articles: &[Article]) -> usize {
    articles
        .iter()
        .filter(|a| !a.is_read)
        .collect::<Vec<_>>() // Unnecessary allocation!
        .len()
}

// DO: Use connection pooling
pub struct SqlitePool {
    pool: r2d2::Pool<SqliteConnectionManager>,
}

// DO: Batch database operations
pub async fn upsert_articles(&self, articles: &[Article]) -> Result<(), Error> {
    let mut conn = self.pool.get()?;
    let tx = conn.transaction()?;
    
    let mut stmt = tx.prepare_cached(
        "INSERT OR REPLACE INTO articles (id, feed_id, title, ...) VALUES (?, ?, ?, ...)"
    )?;
    
    for article in articles {
        stmt.execute(params![article.id, article.feed_id, article.title, ...])?;
    }
    
    tx.commit()?;
    Ok(())
}

// DO: Use channels for background work
pub fn start_refresh_worker(state: Arc<AppState>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(900));
        
        loop {
            interval.tick().await;
            if let Err(e) = refresh_all_feeds(&state).await {
                tracing::error!("Background refresh failed: {}", e);
            }
        }
    })
}
```

### 8.2 Frontend Performance

```typescript
// DO: Use virtual lists for long lists
<script lang="ts">
    import VirtualList from 'svelte-virtual-list-ce';
    import ArticleRow from './ArticleRow.svelte';
    
    export let articles: Article[];
</script>

<VirtualList items={articles} let:item>
    <ArticleRow article={item} />
</VirtualList>

// DO: Debounce expensive operations
import { debounce } from '$lib/utils';

const debouncedSearch = debounce(async (query: string) => {
    results = await api.searchArticles(query);
}, 300);

// DO: Use derived stores for computed values (automatic memoization)
export const unreadCount = derived(articles, $articles => 
    $articles.filter(a => !a.isRead).length
);

// DON'T: Compute in template
{#each $articles.filter(a => !a.isRead) as article}  <!-- Recomputes every render! -->

// DO: Use transitions sparingly
<div in:fade={{ duration: 150 }}>  <!-- Fast, subtle -->

// DON'T: Heavy animations on lists
{#each items as item (item.id)}
    <div in:fly={{ y: 50, duration: 500 }}>  <!-- 500ms * N items = slow -->
```

-----

## 9. Security Guidelines

### 9.1 Input Validation

```rust
// Validate at API boundaries
#[tauri::command]
pub async fn add_feed(
    state: State<'_, AppState>,
    url: String,
    folder_id: Option<String>,
) -> Result<Feed, CommandError> {
    // Validate URL format
    let parsed_url = Url::parse(&url)
        .map_err(|_| CommandError::validation("Invalid URL format"))?;
    
    // Validate scheme
    if !["http", "https"].contains(&parsed_url.scheme()) {
        return Err(CommandError::validation("URL must use HTTP or HTTPS"));
    }
    
    // Validate folder_id format if provided
    let folder_uuid = folder_id
        .map(|id| Uuid::parse_str(&id))
        .transpose()
        .map_err(|_| CommandError::validation("Invalid folder ID"))?;
    
    // Proceed with validated input
    core::feeds::add_feed(&state, parsed_url, folder_uuid).await
}
```

### 9.2 Content Sanitization

```rust
// Always sanitize HTML before display
use ammonia::Builder;

pub fn sanitize_html(html: &str) -> String {
    Builder::default()
        .tags(hashset!["p", "br", "a", "strong", "em", "code", "pre", "blockquote", "ul", "ol", "li", "h1", "h2", "h3", "img"])
        .tag_attributes(hashmap![
            "a" => hashset!["href", "title"],
            "img" => hashset!["src", "alt", "title"],
        ])
        .url_schemes(hashset!["http", "https"])
        .link_rel(Some("noopener noreferrer"))
        .clean(html)
        .to_string()
}
```

### 9.3 Secrets Management

```rust
// Never log sensitive data
impl std::fmt::Debug for Settings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Settings")
            .field("theme_id", &self.theme_id)
            // Omit sensitive fields
            .finish_non_exhaustive()
    }
}

// Use secure storage for credentials
#[cfg(target_os = "macos")]
fn store_secret(key: &str, value: &str) -> Result<(), KeychainError> {
    keychain::set_password("com.curio-reader", key, value)
}
```

-----

## 10. Appendix

### 10.1 Development Scripts

```bash
#!/bin/bash
# scripts/setup.sh

set -e

echo "🚀 Setting up Curio Reader development environment..."

# Check prerequisites
command -v rustc >/dev/null 2>&1 || { echo "❌ Rust is required. Install from rustup.rs"; exit 1; }
command -v node >/dev/null 2>&1 || { echo "❌ Node.js is required."; exit 1; }
command -v pnpm >/dev/null 2>&1 || { echo "❌ pnpm is required. Run: npm i -g pnpm"; exit 1; }

# Install Tauri CLI
cargo install tauri-cli

# Install dependencies
pnpm install
cd src-tauri && cargo fetch && cd ..

# Setup git hooks
pnpm exec lefthook install

# Generate types
./scripts/gen-types.sh

# Bundle yt-dlp
./scripts/bundle-ytdlp.sh

echo "✅ Setup complete! Run 'make dev' to start developing."
```

```bash
#!/bin/bash
# scripts/bundle-ytdlp.sh

set -e

YTDLP_VERSION="2024.01.01"  # Update periodically
BIN_DIR="src-tauri/bin"

mkdir -p "$BIN_DIR"

case "$(uname -s)" in
    Darwin*)
        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp_macos"
        YTDLP_BIN="$BIN_DIR/yt-dlp-aarch64-apple-darwin"
        ;;
    Linux*)
        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp"
        YTDLP_BIN="$BIN_DIR/yt-dlp-x86_64-unknown-linux-gnu"
        ;;
    MINGW*|CYGWIN*|MSYS*)
        YTDLP_URL="https://github.com/yt-dlp/yt-dlp/releases/download/${YTDLP_VERSION}/yt-dlp.exe"
        YTDLP_BIN="$BIN_DIR/yt-dlp-x86_64-pc-windows-msvc.exe"
        ;;
esac

if [ ! -f "$YTDLP_BIN" ] || [ "$1" == "--force" ]; then
    echo "📥 Downloading yt-dlp ${YTDLP_VERSION}..."
    curl -L "$YTDLP_URL" -o "$YTDLP_BIN"
    chmod +x "$YTDLP_BIN"
    echo "✅ yt-dlp installed to $YTDLP_BIN"
else
    echo "ℹ️  yt-dlp already installed. Use --force to re-download."
fi
```

```bash
#!/bin/bash
# scripts/gen-types.sh

set -e

echo "🔄 Generating TypeScript types from Rust..."

cd src-tauri

# Run tests that export types
cargo test export_bindings --features ts-rs -- --nocapture

# Format generated types
cd ..
pnpm exec biome format --write src/lib/types/generated/

echo "✅ Types generated in src/lib/types/generated/"
```

### 10.2 VS Code Configuration

```json
// .vscode/settings.json
{
    "editor.formatOnSave": true,
    "editor.defaultFormatter": "biomejs.biome",
    "[rust]": {
        "editor.defaultFormatter": "rust-lang.rust-analyzer"
    },
    "rust-analyzer.check.command": "clippy",
    "rust-analyzer.check.extraArgs": ["--all-features"],
    "rust-analyzer.cargo.features": "all",
    "typescript.preferences.importModuleSpecifier": "relative",
    "svelte.enable-ts-plugin": true,
    "files.exclude": {
        "**/target": true,
        "**/node_modules": true
    }
}
```

```json
// .vscode/extensions.json
{
    "recommendations": [
        "rust-lang.rust-analyzer",
        "svelte.svelte-vscode",
        "biomejs.biome",
        "tauri-apps.tauri-vscode",
        "bradlc.vscode-tailwindcss",
        "aaron-bond.better-comments"
    ]
}
```