# Curio Reader: RSS Reader PRD

> A fast, themeable, power-user RSS reader with native YouTube, Reddit, and podcast experiences.

## 1. Vision & Goals

### 1.1 Vision Statement

Curio Reader is a desktop RSS reader built for power users managing hundreds of feeds. It uses RSS as the discovery and sync backbone while providing rich, native-feeling experiences for YouTube (with comments and metadata), Reddit (with full thread viewing), and podcasts. It prioritizes speed at scale, visual customization, and hierarchical feed organization.

### 1.2 Design Principles

1. **RSS as backbone, rich experiences on top** — RSS for discovery and sync; full platform experiences where it matters
1. **Speed at scale** — 500 feeds should feel as fast as 50
1. **Your aesthetics, your rules** — Full theme control including transparency, colors, typography
1. **Hierarchy matches mental models** — Nested folders, tags, and profiles reflect how power users organize
1. **View modes match platforms** — Reddit feels like Reddit, YouTube feels like YouTube, articles feel readable

### 1.3 Success Metrics

|Metric                                 |Target             |
|---------------------------------------|-------------------|
|Cold start to usable                   |< 3 seconds        |
|Full feed refresh (500 feeds)          |< 30 seconds       |
|Incremental refresh (500 feeds, cached)|< 10 seconds       |
|UI interaction latency                 |< 50ms             |
|Memory usage (5000 articles cached)    |< 500MB            |
|Image cache efficiency                 |80%+ cache hit rate|

-----

## 2. Target Users

### 2.1 Primary Persona: Power Feed Consumer

- Subscribes to 300-1000+ feeds across blogs, YouTube, Reddit, podcasts
- Manages feeds across multiple contexts (work, hobbies, research) via profiles
- Values keyboard navigation and efficiency
- Has strong aesthetic preferences; wants app to feel “native” to their desktop
- Organizes information hierarchically with deep nesting
- Reads on desktop during focused sessions
- Wants YouTube and Reddit experiences without leaving the app

### 2.2 Secondary Persona: Privacy-Conscious User

- Prefers RSS over algorithmic feeds
- Wants YouTube/Reddit content without platform tracking
- Values local-first data storage with full image caching
- Appreciates transparency in how app handles data

-----

## 3. Core Features

### 3.1 Feed Management

#### 3.1.1 OPML Profiles

**Purpose:** Support multiple OPML files as distinct profiles for different contexts (work, personal, research).

**Profile Management:**

- Each profile is a separate OPML file with independent folder/feed hierarchy
- Quick switch between profiles via sidebar dropdown or keyboard (`Ctrl+1-9`)
- Profiles stored in `~/.config/curio-reader/profiles/`
- Default profile created on first launch

**Profile Structure:**

```
~/.config/curio-reader/
├── profiles/
│   ├── default.opml
│   ├── work.opml
│   └── research.opml
├── settings.json
└── themes/
```

**Import/Export:**

- Import standard OPML 1.0/2.0 files (auto-detect format)
- Import extended Curio OPML with custom attributes
- Export as standard OPML (strips custom attributes) or Curio OPML
- Merge import: add feeds to existing profile without replacing

#### 3.1.2 OPML Format Support

**Standard OPML (Import/Export):**

- Full OPML 1.0 and 2.0 compatibility
- Preserves standard attributes through round-trips
- Graceful handling of non-standard attributes from other readers

**Extended Curio OPML Schema:**

```xml
<?xml version="1.0" encoding="UTF-8"?>
<opml version="2.0">
  <head>
    <title>My Feeds</title>
    <curio:version>1.0</curio:version>
    <curio:profile>default</curio:profile>
  </head>
  <body>
    <outline text="Tech" curio:icon="cpu" curio:color="#3b82f6">
      <outline text="Rust" curio:tags="programming,rust">
        <outline 
          text="This Week in Rust" 
          type="rss" 
          xmlUrl="https://this-week-in-rust.org/rss.xml"
          curio:viewMode="article"
          curio:refreshInterval="3600"
          curio:tags="newsletter"
        />
        <outline 
          text="r/rust" 
          type="rss" 
          xmlUrl="https://www.reddit.com/r/rust/.rss"
          curio:viewMode="reddit"
          curio:subreddit="rust"
          curio:tags="community"
        />
      </outline>
      <outline text="YouTube Channels" curio:viewMode="youtube">
        <outline 
          text="Jon Gjengset" 
          type="rss" 
          xmlUrl="https://www.youtube.com/feeds/videos.xml?channel_id=UC_iD0xppBwwsrM9DegC5cQQ"
          curio:viewMode="youtube"
          curio:channelId="UC_iD0xppBwwsrM9DegC5cQQ"
          curio:customIcon="path/to/icon.png"
        />
      </outline>
    </outline>
    <outline text="Podcasts" curio:viewMode="podcast" curio:icon="headphones">
      <outline 
        text="Acquired" 
        type="rss" 
        xmlUrl="https://acquired.fm/feed"
        curio:viewMode="podcast"
      />
    </outline>
  </body>
</opml>
```

**Custom Attributes:**

|Attribute              |Scope       |Description                                         |
|-----------------------|------------|----------------------------------------------------|
|`curio:viewMode`       |Outline/Feed|`article`, `reddit`, `youtube`, `podcast`, `gallery`|
|`curio:tags`           |Outline/Feed|Comma-separated tags for filtering                  |
|`curio:icon`           |Folder      |Icon identifier from icon set                       |
|`curio:customIcon`     |Feed        |Path to user-uploaded icon override                 |
|`curio:color`          |Folder      |Hex color for folder accent                         |
|`curio:refreshInterval`|Feed        |Seconds between auto-refresh (overrides global)     |
|`curio:notify`         |Feed        |Boolean for new-item notifications                  |
|`curio:channelId`      |Feed        |YouTube channel ID for API enrichment               |
|`curio:subreddit`      |Feed        |Subreddit name for API enrichment                   |

#### 3.1.3 Feed Icons

**Automatic Favicon Scraping:**

- On feed add, attempt to fetch favicon from site root (`/favicon.ico`)
- Fall back to HTML `<link rel="icon">` parsing
- Fall back to Google Favicon service as last resort
- Cache icons locally with feed

**User Override:**

- Right-click feed → “Set custom icon…”
- Accept PNG, JPG, SVG, ICO formats
- Auto-resize and cache to standard sizes (16px, 32px, 64px)
- Custom icons stored in `~/.config/curio-reader/icons/`
- Icon override persists in OPML as `curio:customIcon`

#### 3.1.4 Feed Discovery

- Auto-detect RSS/Atom links when user pastes a URL
- YouTube: Extract channel ID from various URL formats, construct feed URL, store channel ID for API
- Reddit: Validate subreddit exists, construct feed URL with sort options, store subreddit name for API

#### 3.1.5 Feed Organization

- Drag-and-drop reordering within and between folders
- Multi-select for bulk operations (move, tag, delete, change view mode)
- Smart folders based on tag queries (e.g., “all feeds tagged ‘rust’”)
- Feed health indicators (last successful fetch, error state, item count)
- Bulk actions: mark all read, refresh folder, export folder as OPML
- Feed statistics: items/day, read rate, staleness

-----

### 3.2 View Modes

#### 3.2.1 Article View (Default)

**Layout:** Three-column (sidebar, article list, reader pane) or two-column (sidebar, combined list+reader)

**List Item Display:**

- Title (truncated to 2 lines)
- Source name + favicon
- Publish date (relative: “2h ago”, “Yesterday”)
- Read/unread indicator
- Optional: First 100 chars of content preview

**Reader Pane:**

- Sanitized HTML rendering with CSS reset
- Reader-mode toggle (extracts article body, strips cruft)
- Adjustable font size, line height, max width
- Image lazy loading with lightbox on click

#### 3.2.2 Reddit View

**Purpose:** Provide a full Reddit-like experience for browsing and reading threads without leaving Curio.

**Layout:** Card-based feed with expandable thread view

**Feed Card Display:**

- Vote score (from RSS, enriched via API when available)
- Title as primary text
- Subreddit + author + relative time
- Thumbnail (if present, cached locally)
- Post flair (if available)
- Comment count indicator
- Self-text preview (first 200 chars)
- Link domain for external links
- Award indicators (gold, etc.)

**Thread View (Expanded/Detail):**

When a post is opened, fetch full thread via Reddit’s public JSON endpoint (no auth required):

```
https://www.reddit.com/r/{subreddit}/comments/{id}.json?sort={sort}&limit=500
```

**Reddit JSON API Details:**

|Endpoint                     |Purpose                |Rate Limit           |
|-----------------------------|-----------------------|---------------------|
|`/r/{sub}/comments/{id}.json`|Full thread + comments |~60 req/min (no auth)|
|`/r/{sub}/.json`             |Subreddit feed (backup)|~60 req/min          |
|`/{permalink}.json`          |Direct post access     |~60 req/min          |

**Request Headers:**

```rust
// Reddit requires a User-Agent to avoid blocking
const USER_AGENT: &str = "Curio Reader/1.0 (RSS Desktop Client)";

async fn fetch_reddit_thread(subreddit: &str, post_id: &str, sort: &str) -> Result<RedditThread> {
    let url = format!(
        "https://www.reddit.com/r/{}/comments/{}.json?sort={}&limit=500&raw_json=1",
        subreddit, post_id, sort
    );
    
    let response = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?;
    
    // Response is array: [post_data, comments_data]
    let data: Vec<RedditListing> = response.json().await?;
    parse_reddit_thread(data)
}
```

**Rate Limit Handling:**

- Queue requests with 1-second minimum spacing
- Exponential backoff on 429 responses
- Cache aggressively to minimize requests
- Background refresh during idle periods

**Thread Layout:**

```
┌─────────────────────────────────────────────────────────────┐
│ [↑ 1.2k ↓]  Post Title Here                                 │
│ r/rust • u/author • 3 hours ago • 127 comments              │
├─────────────────────────────────────────────────────────────┤
│                                                             │
│ Full post content / self-text / embedded media              │
│                                                             │
├─────────────────────────────────────────────────────────────┤
│ Sort: [Best ▼]  [Collapse All]  [Open in Browser]           │
├─────────────────────────────────────────────────────────────┤
│ ┌─────────────────────────────────────────────────────────┐ │
│ │ [↑ 234 ↓] u/commenter1 • 2h                             │ │
│ │ Comment text here...                                    │ │
│ │ [reply] [collapse]                                      │ │
│ │  ┌──────────────────────────────────────────────────┐   │ │
│ │  │ [↑ 45 ↓] u/replier • 1h                          │   │ │
│ │  │ Nested reply...                                  │   │ │
│ │  └──────────────────────────────────────────────────┘   │ │
│ └─────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

**Comment Features:**

- Threaded/nested display with collapse controls
- Sort options: Best, Top, New, Controversial, Old
- Vote display (read-only, no Reddit auth)
- User flair display
- Award indicators on comments
- “Load more comments” pagination
- Markdown rendering for comment text
- Syntax highlighting for code blocks
- Inline image/gif expansion

**Reddit-Specific Data Model:**

```rust
struct RedditPost {
    reddit_id: String,           // t3_xxxxx
    subreddit: String,
    author: String,
    author_flair: Option<String>,
    score: i32,
    upvote_ratio: f32,
    num_comments: i32,
    permalink: String,
    is_self: bool,
    selftext_html: Option<String>,
    link_url: Option<String>,
    thumbnail: Option<String>,
    preview_images: Vec<String>,
    created_utc: i64,
    flair_text: Option<String>,
    awards: Vec<RedditAward>,
}

struct RedditComment {
    reddit_id: String,           // t1_xxxxx
    author: String,
    author_flair: Option<String>,
    body_html: String,
    score: i32,
    created_utc: i64,
    depth: i32,
    replies: Vec<RedditComment>,
    is_collapsed: bool,
    awards: Vec<RedditAward>,
}
```

**Interactions:**

- Click post → expand to thread view with comments
- `j`/`k` → navigate between posts or comments
- `Enter` → expand/collapse comment thread
- `o` → open in browser
- Vote buttons visible but non-functional (display only)
- Copy permalink, share link

**Caching:**

- Thread data cached for 15 minutes
- Images cached indefinitely (local image cache)
- Stale-while-revalidate for thread refreshes

#### 3.2.3 YouTube View

**Purpose:** Provide a full YouTube-like experience for watching videos and reading comments without leaving Curio.

**Layout:** Grid of video cards with modal/panel video player

**Feed Grid Display:**

- Thumbnail (16:9, with duration overlay)
- Title (2 lines max)
- Channel name + channel icon
- Publish date (relative)
- View count
- Video quality indicator (4K, HDR badges)

**Video Player View:**

When a video is selected, open in-app player with full metadata:

```
┌─────────────────────────────────────────────────────────────────────┐
│ ┌─────────────────────────────────────────────────────────────────┐ │
│ │                                                                 │ │
│ │                    Embedded YouTube Player                      │ │
│ │                         (via webview)                           │ │
│ │                                                                 │ │
│ └─────────────────────────────────────────────────────────────────┘ │
│                                                                     │
│ Video Title Here - Full Title Without Truncation                    │
│ 123,456 views • Jan 15, 2025                                        │
│                                                                     │
│ ┌──────────┐                                                        │
│ │ Channel  │  Channel Name                      [Subscribe: 1.2M]   │
│ │  Icon    │  Channel description snippet...                        │
│ └──────────┘                                                        │
│                                                                     │
│ [👍 12K]  [👎]  [Share]  [Save]  [Open in YouTube]                   │
├─────────────────────────────────────────────────────────────────────┤
│ Description                                              [Show More]│
│ Video description text, links, timestamps...                        │
├─────────────────────────────────────────────────────────────────────┤
│ 1,234 Comments                              Sort: [Top ▼]           │
├─────────────────────────────────────────────────────────────────────┤
│ ┌─────────────────────────────────────────────────────────────────┐ │
│ │ [Avatar] @username • 2 days ago                                 │ │
│ │ Comment text here with emoji support 🎉                         │ │
│ │ [👍 234]  [👎]  [Reply]                                         │ │
│ │   └─ 12 replies [View]                                          │ │
│ └─────────────────────────────────────────────────────────────────┘ │
│ ┌─────────────────────────────────────────────────────────────────┐ │
│ │ [Avatar] @another_user • 1 day ago                              │ │
│ │ Another comment...                                              │ │
│ └─────────────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────────┘
```

**Video Player Features:**

- Embedded YouTube player via webview (iframe API)
- Playback controls: play/pause, seek, volume, speed, quality
- Picture-in-picture support
- Keyboard shortcuts: `Space` (play/pause), `←`/`→` (seek), `f` (fullscreen)
- Theater mode (wider player, collapsed sidebar)
- Mini-player mode (continue browsing while video plays)

**Metadata Display:**

- Full video title and description
- View count, like count
- Publish date
- Channel info with subscriber count
- Video chapters (if available)
- Tags

**Comment Features:**

- Threaded comment display
- Sort: Top comments, Newest first
- Reply threading (expandable)
- Like counts on comments
- Pinned comments
- Heart’d by creator indicator
- Full comment extraction (no pagination limits)

**YouTube Data Enrichment via yt-dlp:**

RSS provides basic info; enrich via bundled yt-dlp for full metadata and comments:

```rust
// Extract metadata + comments without downloading video
fn enrich_youtube_video(video_url: &str) -> Result<YtDlpMetadata> {
    let output = Command::new(yt_dlp_path())
        .args([
            "--dump-json",
            "--write-comments",
            "--no-download",
            "--no-warnings",
            video_url
        ])
        .output()?;
    
    serde_json::from_slice(&output.stdout)
}

struct YtDlpMetadata {
    id: String,
    title: String,
    description: String,
    channel: String,
    channel_id: String,
    channel_url: String,
    channel_follower_count: Option<i64>,
    uploader: String,
    upload_date: String,           // YYYYMMDD format
    
    // Rich metadata
    view_count: Option<i64>,
    like_count: Option<i64>,
    comment_count: Option<i64>,
    duration: i64,                 // seconds
    categories: Vec<String>,
    tags: Vec<String>,
    
    // Chapters
    chapters: Option<Vec<Chapter>>,
    
    // Thumbnails (multiple resolutions)
    thumbnails: Vec<Thumbnail>,
    
    // Comments (when --write-comments used)
    comments: Option<Vec<YtDlpComment>>,
}

struct Chapter {
    start_time: f64,
    end_time: f64,
    title: String,
}

struct Thumbnail {
    url: String,
    width: Option<i32>,
    height: Option<i32>,
    id: String,
}

struct YtDlpComment {
    id: String,
    text: String,
    author: String,
    author_id: String,
    author_thumbnail: String,
    author_is_uploader: bool,
    parent: Option<String>,        // "root" or parent comment ID
    like_count: Option<i64>,
    is_pinned: bool,
    is_favorited: bool,            // Heart'd by creator
    timestamp: Option<i64>,
}
```

**yt-dlp Bundling Strategy:**

```
Curio Reader.app/
└── Contents/
    └── Resources/
        └── bin/
            ├── yt-dlp           # macOS/Linux
            └── yt-dlp.exe       # Windows
```

|Concern         |Approach                                                  |
|----------------|----------------------------------------------------------|
|Initial bundle  |Ship current stable yt-dlp with each release              |
|Auto-update     |Check for updates weekly, download in background          |
|Update mechanism|`yt-dlp --update` or fetch from GitHub releases           |
|Fallback        |If yt-dlp missing/broken, graceful degradation to RSS-only|
|Multi-platform  |Vimeo, Twitch, PeerTube work automatically                |

**Update Check:**

```rust
async fn check_ytdlp_update() -> Result<Option<String>> {
    // Check GitHub releases API for newer version
    let current = get_ytdlp_version()?;
    let latest = fetch_latest_release("yt-dlp/yt-dlp").await?;
    
    if latest.version > current {
        Ok(Some(latest.download_url))
    } else {
        Ok(None)
    }
}
```

**Interactions:**

- Click thumbnail → open video player view
- `j`/`k` → navigate grid
- `Enter` → play video
- `c` → jump to comments
- `Esc` → close player, return to grid
- Right-click → copy link, open in browser, mark watched

**Caching:**

- Thumbnails cached locally indefinitely
- Video metadata cached for 24 hours
- Comments cached for 6 hours (full extraction, no pagination)
- Stale-while-revalidate pattern

#### 3.2.4 Podcast View

**Layout:** Episode list with persistent player bar

**Episode Display:**

- Episode title
- Show name + artwork (from feed)
- Duration
- Publish date
- Downloaded indicator
- Progress bar (if partially played)

**Player Features:**

- Play/pause, skip ±15s/30s (configurable)
- Playback speed (0.5x - 3x)
- Volume control
- Sleep timer
- Queue management
- Background playback (continues when window minimized)

**Offline Support:**

- Download episodes for offline playback
- Configurable auto-download (newest N episodes, or all)
- Storage management (auto-delete played episodes after N days)

#### 3.2.5 Gallery View

**Layout:** Masonry grid of images (for image-heavy feeds like art blogs, photography)

**Display:**

- Images extracted from feed content
- Subtle title overlay on hover
- Lightbox navigation between items

-----

### 3.3 Theming System

#### 3.3.1 Theme Structure

```typescript
interface Theme {
  id: string;
  name: string;
  author?: string;
  
  // Base colors
  colors: {
    background: string;
    backgroundSecondary: string;
    backgroundTertiary: string;
    foreground: string;
    foregroundMuted: string;
    foregroundSubtle: string;
    
    accent: string;
    accentHover: string;
    accentForeground: string;
    
    border: string;
    borderSubtle: string;
    
    error: string;
    warning: string;
    success: string;
    
    // Semantic
    unread: string;
    read: string;
    link: string;
    linkVisited: string;
  };
  
  // Typography
  typography: {
    fontFamily: string;
    fontFamilyMono: string;
    fontSizeBase: string;
    lineHeight: string;
    
    // Reader-specific
    readerFontFamily: string;
    readerFontSize: string;
    readerLineHeight: string;
    readerMaxWidth: string;
  };
  
  // Spacing and sizing
  spacing: {
    unit: string;
    borderRadius: string;
    sidebarWidth: string;
    listWidth: string;
  };
  
  // Effects
  effects: {
    transparency: number; // 0-100
    blur: number; // px
    shadow: string;
  };
}
```

#### 3.3.2 Built-in Themes

1. **Light** — Clean, minimal, high contrast
1. **Dark** — Easy on eyes, OLED-friendly option
1. **Nord** — Nord color palette
1. **Catppuccin** (Latte, Frappé, Macchiato, Mocha variants)
1. **Solarized** (Light, Dark)
1. **Dracula**
1. **Transparent Dark** — For desktop widget aesthetic

#### 3.3.3 Custom Themes

- Full JSON theme editor with live preview
- Import/export themes as JSON files
- Theme sharing via JSON (future: community theme repository)

#### 3.3.4 Transparency & Effects

- Window transparency slider (0-100%)
- Background blur intensity (0-50px)
- Option: Vibrancy/Mica effect on supported platforms
- Custom titlebar with traffic lights / window controls

-----

### 3.4 Performance Features

#### 3.4.1 Fast Refresh (500+ Feed Scale)

**Architecture:**

- Parallel feed fetching with configurable concurrency (default: 20, max: 50)
- Conditional GET (If-Modified-Since, ETag) to skip unchanged feeds
- Delta updates — only new items inserted, no full reparse
- Background refresh on interval (configurable, default: 15 min)
- Priority queue: recently-viewed feeds refresh first

**Scaling Strategies:**

- Feed batching: group requests by domain to respect rate limits
- Staggered refresh: spread 500 feeds over refresh interval, not all at once
- Smart scheduling: high-activity feeds refresh more often than stale feeds
- Request deduplication: feeds with shared assets (YouTube channels from same account) batched

**User Controls:**

- Manual refresh: single feed, folder, or all
- Refresh indicator: progress bar with count (47/500 feeds)
- Last refresh timestamp always visible
- Per-feed refresh interval override
- Pause/resume background refresh

**Performance Targets:**

|Operation                           |Target (500 feeds)      |
|------------------------------------|------------------------|
|Full refresh (cold, no cache)       |< 60 seconds            |
|Full refresh (warm, conditional GET)|< 15 seconds            |
|Single feed refresh                 |< 2 seconds             |
|Background refresh cycle            |Continuous, non-blocking|

#### 3.4.2 Local Image Cache

**Philosophy:** All images are cached locally for privacy, offline access, and speed.

**Cache Architecture:**

```
~/.config/curio-reader/cache/
├── images/
│   ├── feeds/           # Feed content images
│   │   └── {hash}.{ext}
│   ├── thumbnails/      # YouTube/Reddit thumbnails
│   │   └── {hash}.{ext}
│   ├── avatars/         # User avatars (Reddit, YouTube)
│   │   └── {hash}.{ext}
│   └── icons/           # Feed favicons
│       └── {hash}.{ext}
├── podcasts/            # Downloaded audio
│   └── {feed_id}/
│       └── {episode_id}.{ext}
└── index.db             # Cache metadata
```

|Data          |Storage            |TTL                           |Max Size |
|--------------|-------------------|------------------------------|---------|
|Feed icons    |Disk               |Indefinite                    |100MB    |
|Article images|Disk               |Configurable (default 30 days)|2GB      |
|Thumbnails    |Disk               |Configurable (default 30 days)|1GB      |
|Avatars       |Disk               |7 days                        |200MB    |
|Podcast audio |Disk (user-managed)|Manual or auto-cleanup        |Unlimited|

**Cache Behavior:**

- Images fetched on first view, not on feed refresh (lazy loading)
- Deduplicated by content hash (same image from multiple feeds = one file)
- Background prefetch for visible + next page of items
- LRU eviction when size limits reached
- Cache stats in settings: size, hit rate, cleanup button

**Privacy Benefits:**

- No requests to external image hosts after initial fetch
- Tracking pixels rendered once then cached
- Works fully offline after initial cache population

#### 3.4.3 Startup Optimization

**Goal:** UI usable in < 3 seconds even with 500+ feeds and 5000+ cached articles.

**Startup Sequence:**

1. Load settings and theme (< 100ms)
1. Restore window state and show shell UI (< 500ms)
1. Load feed tree from SQLite (< 500ms)
1. Load visible articles for current view (paginated, < 1s)
1. Display UI as interactive
1. Background: load remaining data, start refresh cycle

**Techniques:**

- SQLite indexes optimized for common queries
- Lazy-load images (request on scroll into view)
- Virtualized lists for article/feed displays (only render visible items)
- Index-based pagination (no OFFSET, use keyset pagination)
- Incremental folder tree loading (expand on demand for deep nesting)
- Service worker-style caching for frequently accessed data

-----

### 3.5 Reading Experience

#### 3.5.1 Article Management

- Read/unread status (persisted)
- Star/favorite articles
- Read later queue (see 3.5.4)
- Archive (hidden from main views but searchable)
- Full-text search across all cached articles
- Markdown export (see 3.5.5)

#### 3.5.4 Read Later Queue

**Purpose:** A dedicated queue for articles you intend to read, separate from stars (which are for saving/archiving).

**Behavior:**

- Add to Read Later via keyboard (`l`), right-click menu, or button
- Read Later items appear in dedicated sidebar section
- Queue is ordered by date added (newest first) or manually sortable
- Reading an article from Read Later can optionally auto-remove it (configurable)
- Badge count shows unread items in Read Later queue

**Data Model Addition:**

```rust
struct Article {
    // ... existing fields ...
    is_read_later: bool,
    read_later_at: Option<DateTime<Utc>>,
    read_later_position: Option<i32>, // For manual ordering
}
```

**Sync with Stars:**

- Stars and Read Later are independent (an article can be both)
- Stars = “I want to keep this”
- Read Later = “I need to read this”

#### 3.5.5 Markdown Export & Clean Parsing

**Purpose:** Export articles as clean, readable Markdown for use in note-taking apps (Obsidian, Logseq), archival, or offline reading.

**Article Parsing Pipeline:**

```
Raw HTML → Sanitize → Readability Extract → Markdown Convert → Clean & Format
```

**Stage 1: Sanitization**

- Remove scripts, styles, iframes (except YouTube/Vimeo embeds)
- Strip tracking pixels and analytics
- Remove ads and navigation elements
- Preserve semantic HTML (headers, lists, blockquotes, code blocks)

**Stage 2: Readability Extraction**

- Use Mozilla Readability algorithm (via `readability` crate or port)
- Extract article body, title, author, publish date
- Remove sidebars, footers, related articles
- Handle multi-page articles (single-page preference)

**Stage 3: Markdown Conversion**

- Convert clean HTML to CommonMark Markdown
- Preserve structure: headers, lists, blockquotes, code blocks with language hints
- Handle images: configurable inline, reference-style, or stripped
- Convert tables to Markdown tables (with fallback to HTML for complex tables)
- Preserve links with proper reference formatting

**Stage 4: Clean & Format**

- Normalize whitespace
- Remove empty paragraphs
- Ensure consistent header hierarchy
- Add metadata frontmatter (optional)

**Export Formats:**

**Single Article Export:**

```markdown
---
title: "Article Title"
author: "Author Name"
source: "https://example.com/article"
feed: "Example Blog"
published: 2025-01-15
saved: 2025-01-16
tags: [rust, programming]
---

# Article Title

Article content in clean Markdown...

## Section Header

More content...
```

**Batch Export Options:**

- Single file per article
- Combined file with separators
- Folder structure mirroring feed/folder hierarchy

**Export Destinations:**

- Save to file (single or batch)
- Copy to clipboard
- Direct integration: Obsidian vault path (configurable)
- Export with images: download and embed as local files or base64

**Export Settings:**

```rust
struct ExportSettings {
    include_frontmatter: bool,
    frontmatter_fields: Vec<String>, // Which fields to include
    image_handling: ImageExportMode,
    link_style: LinkStyle, // Inline or reference
    include_source_link: bool,
    filename_template: String, // e.g., "{date}-{title}"
    obsidian_vault_path: Option<String>,
}

enum ImageExportMode {
    Inline,           // ![](url)
    LocalCopy,        // Download to ./images/, reference locally
    Base64Embed,      // Inline base64 (for single-file portability)
    Strip,            // Remove images entirely
}

enum LinkStyle {
    Inline,     // [text](url)
    Reference,  // [text][1] ... [1]: url
}
```

**Keyboard Shortcuts:**

|Key           |Action                            |
|--------------|----------------------------------|
|`e`           |Export current article to Markdown|
|`E`           |Export selected articles (batch)  |
|`Ctrl+Shift+C`|Copy as Markdown to clipboard     |

**Rust Crates for Parsing:**

|Stage        |Crate                                            |
|-------------|-------------------------------------------------|
|HTML parsing |`scraper` or `kuchiki`                           |
|Sanitization |`ammonia`                                        |
|Readability  |`readability` (or custom port)                   |
|HTML→Markdown|`htmd` or custom with `pulldown-cmark` for output|

**Quality Signals:**

- Parsing confidence score (0-100) based on extraction success
- Fallback to raw content with warning if parsing fails
- Preview before export to verify quality

#### 3.5.2 Keyboard Navigation

|Key           |Action                 |
|--------------|-----------------------|
|`j` / `k`     |Next / previous item   |
|`o` / `Enter` |Open selected item     |
|`v`           |Open in browser        |
|`s`           |Star/unstar            |
|`l`           |Toggle read later      |
|`m`           |Toggle read/unread     |
|`e`           |Export to Markdown     |
|`E`           |Batch export selected  |
|`Ctrl+Shift+C`|Copy as Markdown       |
|`r`           |Refresh current view   |
|`R`           |Refresh all feeds      |
|`/`           |Focus search           |
|`g` then `h`  |Go home (all items)    |
|`g` then `s`  |Go to starred          |
|`g` then `u`  |Go to unread           |
|`g` then `l`  |Go to read later       |
|`1-5`         |Switch view modes      |
|`t`           |Toggle sidebar         |
|`?`           |Show keyboard shortcuts|

#### 3.5.3 Filtering & Sorting

**Filters:**

- Unread only
- Starred only
- By tag
- By date range
- By feed/folder

**Sort Options:**

- Newest first (default)
- Oldest first
- By feed (grouped)
- By score (Reddit view)

-----

### 3.6 System Integration

#### 3.6.1 Notifications

- Native OS notifications for new items (configurable per-feed)
- Badge count on dock/taskbar icon
- Notification grouping by feed/folder

#### 3.6.2 Global Shortcuts

- Configurable global hotkey to show/hide window
- Quick-add feed shortcut from anywhere

#### 3.6.3 Protocol Handler

- Register `feed://` and `rss://` protocol handlers
- One-click subscribe from browsers

-----

## 4. Technical Architecture

### 4.1 Stack Overview

```
┌─────────────────────────────────────────────────────────────┐
│                    Svelte Frontend                           │
│  ┌─────────────┬─────────────┬───────────────────────────┐  │
│  │   Stores    │  Components │       View Modes          │  │
│  │  (Runes)    │  (Shadcn)   │ Article/Reddit/YT/Podcast │  │
│  └─────────────┴─────────────┴───────────────────────────┘  │
│  ┌─────────────────────────────────────────────────────────┐│
│  │              Embedded Players                           ││
│  │    YouTube (iframe API) • Podcast (HTML5 Audio)         ││
│  └─────────────────────────────────────────────────────────┘│
└────────────────────────┬────────────────────────────────────┘
                         │ Tauri IPC
┌────────────────────────┴────────────────────────────────────┐
│                    Rust Backend                              │
│  ┌──────────┬──────────┬──────────┬────────────────────┐    │
│  │  Feeds   │  OPML    │  Storage │     Commands       │    │
│  │ (feed-rs)│(quick-xml)│ (SQLite) │   (Tauri IPC)      │    │
│  └──────────┴──────────┴──────────┴────────────────────┘    │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Platform Enrichment                      │   │
│  │   yt-dlp (bundled binary) • Reddit JSON (no auth)     │   │
│  └──────────────────────────────────────────────────────┘   │
│  ┌──────────────────────────────────────────────────────┐   │
│  │              Background Services                      │   │
│  │   Auto-refresh • Image Cache • Downloads • Notify     │   │
│  └──────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────┘
```

### 4.1.1 Rust Crates

|Concern               |Crate                               |
|----------------------|------------------------------------|
|RSS/Atom parsing      |`feed-rs`                           |
|OPML parsing          |`quick-xml` + serde                 |
|HTTP client           |`reqwest` with `tokio`              |
|Database              |`rusqlite` with `tokio-rusqlite`    |
|HTML sanitization     |`ammonia`                           |
|Readability           |`readability` or custom             |
|HTML→Markdown         |`htmd`                              |
|Image processing      |`image` (for icon resizing)         |
|Favicon fetching      |Custom + `scraper` for HTML parsing |
|Reddit JSON           |Custom client with `reqwest` + serde|
|Podcast audio metadata|`symphonia`                         |

### 4.1.2 External Binaries

|Binary|Purpose                               |Source                           |
|------|--------------------------------------|---------------------------------|
|yt-dlp|YouTube metadata + comments extraction|Bundled, auto-updated from GitHub|

### 4.2 Data Models

#### 4.2.1 Core Entities

```rust
// Feed source
struct Feed {
    id: Uuid,
    url: String,
    title: String,
    description: Option<String>,
    site_url: Option<String>,
    icon_url: Option<String>,
    
    // Classification
    view_mode: ViewMode,
    tags: Vec<String>,
    folder_id: Option<Uuid>,
    
    // Fetch metadata
    etag: Option<String>,
    last_modified: Option<DateTime<Utc>>,
    last_fetched: Option<DateTime<Utc>>,
    last_error: Option<String>,
    refresh_interval: Option<i32>, // seconds, None = use global
    
    // Settings
    notify_new: bool,
    
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

enum ViewMode {
    Article,
    Reddit,
    YouTube,
    Podcast,
    Gallery,
}

// Folder for organization
struct Folder {
    id: Uuid,
    name: String,
    parent_id: Option<Uuid>, // For nesting
    icon: Option<String>,
    color: Option<String>,
    view_mode: Option<ViewMode>, // Inherited by children
    position: i32, // Sort order
}

// Individual content item
struct Article {
    id: Uuid,
    feed_id: Uuid,
    guid: String, // Original GUID from feed
    
    // Content
    title: String,
    url: Option<String>,
    content_html: Option<String>,
    content_text: Option<String>,
    summary: Option<String>,
    author: Option<String>,
    
    // Media
    thumbnail_url: Option<String>,
    media: Vec<MediaAttachment>,
    
    // Metadata
    published_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
    fetched_at: DateTime<Utc>,
    
    // User state
    is_read: bool,
    is_starred: bool,
    is_archived: bool,
    read_at: Option<DateTime<Utc>>,
    
    // Platform-specific
    reddit_score: Option<i32>,
    reddit_num_comments: Option<i32>,
    youtube_duration: Option<i32>, // seconds
    podcast_duration: Option<i32>, // seconds
    podcast_progress: Option<i32>, // seconds played
}

struct MediaAttachment {
    url: String,
    mime_type: String,
    size_bytes: Option<i64>,
    title: Option<String>,
    
    // For podcasts
    is_downloaded: bool,
    local_path: Option<String>,
}

// User preferences
struct Settings {
    // Appearance
    theme_id: String,
    custom_css: Option<String>,
    window_transparency: i32, // 0-100
    window_blur: i32, // px
    
    // Startup
    startup_behavior: StartupBehavior,
    last_view_state: Option<ViewState>, // Serialized state for "restore last"
    
    // Behavior
    refresh_interval: i32, // seconds (default: 900 = 15 min)
    fetch_concurrency: i32, // default: 20, max: 50 for 500+ feeds
    mark_read_on_scroll: bool,
    mark_read_delay: i32, // ms
    open_links_in_browser: bool,
    read_later_auto_remove: bool, // Remove from queue after reading
    
    // Podcast
    podcast_playback_speed: f32,
    podcast_skip_forward: i32, // seconds
    podcast_skip_back: i32,
    podcast_auto_download: bool,
    podcast_auto_cleanup_days: Option<i32>,
    
    // Notifications
    notifications_enabled: bool,
    notification_sound: bool,
    
    // Cache
    image_cache_days: i32,
    thumbnail_cache_days: i32,
    image_cache_max_gb: f32,
    article_retention_days: Option<i32>,
    
    // yt-dlp
    ytdlp_auto_update: bool,           // Check for updates weekly
    ytdlp_update_check_days: i32,      // Days between update checks (default: 7)
    
    // Export
    export_settings: ExportSettings,
    obsidian_vault_path: Option<String>,
}

enum StartupBehavior {
    RestoreLastView,    // Open to exact scroll position and view
    AllUnread,          // Open to All Items, filtered to unread
    SpecificFolder(Uuid), // Open to a specific folder
    SpecificFeed(Uuid),   // Open to a specific feed
}

struct ViewState {
    profile_id: String,
    view_type: ViewType,          // AllItems, Unread, Starred, ReadLater, Folder, Feed
    view_id: Option<Uuid>,        // Folder or Feed ID if applicable
    view_mode: ViewMode,          // Article, Reddit, YouTube, etc.
    scroll_position: f32,
    selected_item_id: Option<Uuid>,
    filters: ArticleFilter,
    sort: SortOrder,
}
```

### 4.3 Database Schema

```sql
-- Core tables
CREATE TABLE folders (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    parent_id TEXT REFERENCES folders(id) ON DELETE CASCADE,
    icon TEXT,
    color TEXT,
    view_mode TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE feeds (
    id TEXT PRIMARY KEY,
    url TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT,
    site_url TEXT,
    icon_url TEXT,
    view_mode TEXT NOT NULL DEFAULT 'article',
    tags TEXT, -- JSON array
    folder_id TEXT REFERENCES folders(id) ON DELETE SET NULL,
    etag TEXT,
    last_modified TEXT,
    last_fetched TEXT,
    last_error TEXT,
    refresh_interval INTEGER,
    notify_new INTEGER NOT NULL DEFAULT 0,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE articles (
    id TEXT PRIMARY KEY,
    feed_id TEXT NOT NULL REFERENCES feeds(id) ON DELETE CASCADE,
    guid TEXT NOT NULL,
    title TEXT NOT NULL,
    url TEXT,
    content_html TEXT,
    content_text TEXT,
    summary TEXT,
    author TEXT,
    thumbnail_url TEXT,
    published_at TEXT,
    updated_at TEXT,
    fetched_at TEXT NOT NULL DEFAULT (datetime('now')),
    is_read INTEGER NOT NULL DEFAULT 0,
    is_starred INTEGER NOT NULL DEFAULT 0,
    is_read_later INTEGER NOT NULL DEFAULT 0,
    is_archived INTEGER NOT NULL DEFAULT 0,
    read_at TEXT,
    read_later_at TEXT,
    read_later_position INTEGER,
    reddit_score INTEGER,
    reddit_num_comments INTEGER,
    youtube_duration INTEGER,
    podcast_duration INTEGER,
    podcast_progress INTEGER,
    UNIQUE(feed_id, guid)
);

CREATE TABLE media_attachments (
    id TEXT PRIMARY KEY,
    article_id TEXT NOT NULL REFERENCES articles(id) ON DELETE CASCADE,
    url TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    size_bytes INTEGER,
    title TEXT,
    is_downloaded INTEGER NOT NULL DEFAULT 0,
    local_path TEXT
);

-- Full-text search
CREATE VIRTUAL TABLE articles_fts USING fts5(
    title,
    content_text,
    author,
    content='articles',
    content_rowid='rowid'
);

-- Triggers for FTS sync
CREATE TRIGGER articles_ai AFTER INSERT ON articles BEGIN
    INSERT INTO articles_fts(rowid, title, content_text, author)
    VALUES (NEW.rowid, NEW.title, NEW.content_text, NEW.author);
END;

CREATE TRIGGER articles_ad AFTER DELETE ON articles BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text, author)
    VALUES ('delete', OLD.rowid, OLD.title, OLD.content_text, OLD.author);
END;

CREATE TRIGGER articles_au AFTER UPDATE ON articles BEGIN
    INSERT INTO articles_fts(articles_fts, rowid, title, content_text, author)
    VALUES ('delete', OLD.rowid, OLD.title, OLD.content_text, OLD.author);
    INSERT INTO articles_fts(rowid, title, content_text, author)
    VALUES (NEW.rowid, NEW.title, NEW.content_text, NEW.author);
END;

-- Indexes
CREATE INDEX idx_articles_feed_id ON articles(feed_id);
CREATE INDEX idx_articles_published_at ON articles(published_at DESC);
CREATE INDEX idx_articles_is_read ON articles(is_read);
CREATE INDEX idx_articles_is_starred ON articles(is_starred);
CREATE INDEX idx_articles_is_read_later ON articles(is_read_later, read_later_position);
CREATE INDEX idx_feeds_folder_id ON feeds(folder_id);
CREATE INDEX idx_folders_parent_id ON folders(parent_id);
```

### 4.4 Tauri Commands

```rust
// Feed management
#[tauri::command]
async fn add_feed(url: String, folder_id: Option<Uuid>, tags: Vec<String>) -> Result<Feed, Error>;

#[tauri::command]
async fn remove_feed(feed_id: Uuid) -> Result<(), Error>;

#[tauri::command]
async fn update_feed(feed_id: Uuid, updates: FeedUpdate) -> Result<Feed, Error>;

#[tauri::command]
async fn get_feeds() -> Result<Vec<Feed>, Error>;

#[tauri::command]
async fn refresh_feed(feed_id: Uuid) -> Result<RefreshResult, Error>;

#[tauri::command]
async fn refresh_all_feeds() -> Result<Vec<RefreshResult>, Error>;

// Folder management
#[tauri::command]
async fn create_folder(name: String, parent_id: Option<Uuid>) -> Result<Folder, Error>;

#[tauri::command]
async fn update_folder(folder_id: Uuid, updates: FolderUpdate) -> Result<Folder, Error>;

#[tauri::command]
async fn delete_folder(folder_id: Uuid) -> Result<(), Error>;

#[tauri::command]
async fn get_folder_tree() -> Result<Vec<FolderNode>, Error>;

// Article operations
#[tauri::command]
async fn get_articles(filter: ArticleFilter, pagination: Pagination) -> Result<ArticlePage, Error>;

#[tauri::command]
async fn mark_read(article_ids: Vec<Uuid>) -> Result<(), Error>;

#[tauri::command]
async fn mark_unread(article_ids: Vec<Uuid>) -> Result<(), Error>;

#[tauri::command]
async fn toggle_starred(article_id: Uuid) -> Result<bool, Error>;

#[tauri::command]
async fn archive_articles(article_ids: Vec<Uuid>) -> Result<(), Error>;

#[tauri::command]
async fn search_articles(query: String, filter: ArticleFilter) -> Result<Vec<Article>, Error>;

// Read Later
#[tauri::command]
async fn add_to_read_later(article_ids: Vec<Uuid>) -> Result<(), Error>;

#[tauri::command]
async fn remove_from_read_later(article_ids: Vec<Uuid>) -> Result<(), Error>;

#[tauri::command]
async fn get_read_later(pagination: Pagination) -> Result<ArticlePage, Error>;

#[tauri::command]
async fn reorder_read_later(article_id: Uuid, new_position: i32) -> Result<(), Error>;

// Markdown Export
#[tauri::command]
async fn parse_article_content(article_id: Uuid) -> Result<ParsedContent, Error>;

#[tauri::command]
async fn export_to_markdown(article_ids: Vec<Uuid>, settings: ExportSettings) -> Result<ExportResult, Error>;

#[tauri::command]
async fn copy_as_markdown(article_id: Uuid, settings: ExportSettings) -> Result<String, Error>;

#[tauri::command]
async fn export_to_obsidian(article_ids: Vec<Uuid>, vault_path: String, settings: ExportSettings) -> Result<Vec<String>, Error>;

// Reddit JSON API
#[tauri::command]
async fn fetch_reddit_thread(permalink: String) -> Result<RedditThread, Error>;

#[tauri::command]
async fn fetch_reddit_comments(post_id: String, sort: CommentSort) -> Result<Vec<RedditComment>, Error>;

// YouTube via yt-dlp
#[tauri::command]
async fn fetch_youtube_metadata(video_url: String) -> Result<YtDlpMetadata, Error>;

#[tauri::command]
async fn fetch_youtube_comments(video_url: String) -> Result<Vec<YtDlpComment>, Error>;

#[tauri::command]
async fn get_ytdlp_version() -> Result<String, Error>;

#[tauri::command]
async fn check_ytdlp_update() -> Result<Option<YtDlpUpdateInfo>, Error>;

#[tauri::command]
async fn update_ytdlp() -> Result<String, Error>; // Returns new version

// Profile management
#[tauri::command]
async fn get_profiles() -> Result<Vec<Profile>, Error>;

#[tauri::command]
async fn create_profile(name: String) -> Result<Profile, Error>;

#[tauri::command]
async fn delete_profile(profile_id: String) -> Result<(), Error>;

#[tauri::command]
async fn switch_profile(profile_id: String) -> Result<(), Error>;

#[tauri::command]
async fn duplicate_profile(profile_id: String, new_name: String) -> Result<Profile, Error>;

// Feed icons
#[tauri::command]
async fn fetch_favicon(url: String) -> Result<String, Error>; // Returns cached path

#[tauri::command]
async fn set_custom_icon(feed_id: Uuid, icon_path: String) -> Result<(), Error>;

#[tauri::command]
async fn clear_custom_icon(feed_id: Uuid) -> Result<(), Error>;

// Image cache
#[tauri::command]
async fn get_cache_stats() -> Result<CacheStats, Error>;

#[tauri::command]
async fn clear_image_cache(cache_type: Option<CacheType>) -> Result<(), Error>;

#[tauri::command]
async fn prefetch_images(article_ids: Vec<Uuid>) -> Result<(), Error>;

// OPML
#[tauri::command]
async fn import_opml(path: String) -> Result<ImportResult, Error>;

#[tauri::command]
async fn export_opml(path: String) -> Result<(), Error>;

// Settings
#[tauri::command]
async fn get_settings() -> Result<Settings, Error>;

#[tauri::command]
async fn update_settings(settings: Settings) -> Result<(), Error>;

// Themes
#[tauri::command]
async fn get_themes() -> Result<Vec<Theme>, Error>;

#[tauri::command]
async fn save_custom_theme(theme: Theme) -> Result<(), Error>;

// Podcast
#[tauri::command]
async fn download_episode(article_id: Uuid) -> Result<(), Error>;

#[tauri::command]
async fn delete_download(article_id: Uuid) -> Result<(), Error>;

#[tauri::command]
async fn update_playback_progress(article_id: Uuid, progress: i32) -> Result<(), Error>;
```

-----

## 5. User Interface

### 5.1 Layout Structure

```
┌──────────────────────────────────────────────────────────────────────┐
│ ─ ─ ─  Flowfeed                               [Search] [⚙] [─][□][×] │
├────────────┬─────────────────────────────────────────────────────────┤
│            │                                                          │
│  Sidebar   │                    Content Area                          │
│            │                                                          │
│ ┌────────┐ │  ┌─────────────────────────────────────────────────┐    │
│ │All Items│ │  │                                                 │    │
│ │Unread   │ │  │                                                 │    │
│ │Starred  │ │  │          View Mode Content                      │    │
│ │Read Ltr │ │  │          (Article/Reddit/YouTube/Podcast)       │    │
│ └────────┘ │  │                                                 │    │
│            │  │                                                 │    │
│ Folders    │  │                                                 │    │
│ ┌────────┐ │  │                                                 │    │
│ │▼ Tech  │ │  │                                                 │    │
│ │  Rust  │ │  │                                                 │    │
│ │  Web   │ │  └─────────────────────────────────────────────────┘    │
│ │▶ News  │ │                                                          │
│ └────────┘ │                                                          │
│            │                                                          │
│ [+] Add    │                                                          │
├────────────┴─────────────────────────────────────────────────────────┤
│ [⏮][▶][⏭]  Now Playing: Podcast Episode Title          ▁▂▃▅▂  12:34 │
└──────────────────────────────────────────────────────────────────────┘
```

### 5.2 Component Hierarchy

```
App
├── Titlebar
│   ├── Logo
│   ├── GlobalSearch
│   └── WindowControls
├── Sidebar
│   ├── QuickAccess
│   │   ├── AllItems
│   │   ├── Unread
│   │   ├── Starred
│   │   └── ReadLater
│   ├── FolderTree
│   │   └── FolderNode (recursive)
│   │       ├── FolderHeader
│   │       └── FeedItem
│   └── AddButton
├── ContentArea
│   ├── Toolbar
│   │   ├── ViewModeSelector
│   │   ├── FilterDropdown
│   │   ├── SortDropdown
│   │   └── RefreshButton
│   └── ViewContainer
│       ├── ArticleView
│       │   ├── ArticleList
│       │   └── ReaderPane
│       ├── RedditView
│       │   └── RedditCard (repeated)
│       ├── YouTubeView
│       │   └── VideoCard (repeated)
│       ├── PodcastView
│       │   └── EpisodeRow (repeated)
│       └── GalleryView
│           └── ImageCard (repeated)
├── PodcastPlayer (fixed bottom)
│   ├── PlayControls
│   ├── ProgressBar
│   └── VolumeControl
└── Modals
    ├── AddFeedModal
    ├── SettingsModal
    ├── ThemeEditorModal
    └── KeyboardShortcutsModal
```

### 5.3 Responsive Behavior

|Width     |Layout                                |
|----------|--------------------------------------|
|< 768px   |Sidebar collapsed, single column      |
|768-1200px|Sidebar + single content column       |
|> 1200px  |Sidebar + list + reader (article view)|

-----

## 6. Implementation Phases

### Phase 1: Foundation (MVP)

**Duration:** 5-7 weeks

**Deliverables:**

- [ ] Tauri + Svelte project scaffold
- [ ] SQLite database with core schema
- [ ] Basic feed fetching and parsing (optimized for 500+ feeds)
- [ ] Simple OPML import (flat structure)
- [ ] Profile system (basic: create, switch, delete)
- [ ] Article view with list and reader
- [ ] Read/unread tracking
- [ ] Read Later queue
- [ ] Favicon scraping with custom icon override
- [ ] Basic dark/light theme
- [ ] Manual refresh with progress indicator
- [ ] Local image caching infrastructure

### Phase 2: Rich Content Views

**Duration:** 4-5 weeks

**Deliverables:**

- [ ] Reddit view mode (card layout)
- [ ] Reddit thread viewer with comments (JSON API)
- [ ] YouTube view mode (grid layout)
- [ ] YouTube embedded player
- [ ] YouTube video details and comments (Data API v3)
- [ ] API key configuration UI
- [ ] Nested folder support
- [ ] Extended OPML with custom attributes
- [ ] Tag system

### Phase 3: Polish & Export

**Duration:** 3-4 weeks

**Deliverables:**

- [ ] Background auto-refresh (500 feed scale)
- [ ] Conditional GET optimization
- [ ] Full theming system
- [ ] Transparency and blur effects
- [ ] Custom theme editor
- [ ] Keyboard navigation (full coverage)
- [ ] Markdown export (single article)
- [ ] Copy as Markdown to clipboard
- [ ] Full-text search
- [ ] Notifications

### Phase 4: Podcast Support

**Duration:** 2-3 weeks

**Deliverables:**

- [ ] Podcast view mode
- [ ] Audio player with queue
- [ ] Download management
- [ ] Playback progress sync
- [ ] Playback speed control
- [ ] Background playback

### Phase 5: Advanced Features

**Duration:** 3-4 weeks

**Deliverables:**

- [ ] Gallery view mode
- [ ] Smart folders (tag-based)
- [ ] OPML export (standard + extended)
- [ ] Global shortcuts
- [ ] Protocol handler
- [ ] Statistics/analytics view
- [ ] Batch Markdown export
- [ ] Obsidian vault integration
- [ ] Export with local images
- [ ] Cache management UI
- [ ] Profile merge/duplicate

-----

## 7. Future Considerations

### 7.1 Potential Features (Post-MVP)

- **Sync**: Optional cloud sync via user’s own storage (Dropbox, S3, etc.)
- **Sharing**: Generate shareable reading lists
- **Annotations**: Highlight and annotate articles
- **AI summaries**: LLM-powered article summarization
- **Social features**: Share to Mastodon/Bluesky
- **Mobile companion**: Read-only mobile app synced via cloud
- **Plugin system**: User-extensible with custom view modes or sources

### 7.2 Non-Goals (Explicitly Out of Scope)

- Web application version
- Server-side components
- Full Reddit/YouTube account auth (posting, voting, subscribing via API)
- RSS feed hosting/publishing
- Email newsletter ingestion (though could add later via Kill the Newsletter)

-----

## 8. Design Decisions (Resolved)

|Question           |Decision                                                                                  |
|-------------------|------------------------------------------------------------------------------------------|
|Feed icons         |Scrape favicons automatically + allow user-uploaded overrides                             |
|Image handling     |Cache all images locally (privacy-first, offline support)                                 |
|YouTube enrichment |Bundled yt-dlp for metadata and comments (no API key required)                            |
|YouTube playback   |In-app webview player with full metadata display                                          |
|Reddit enrichment  |Public JSON endpoint (`/comments/{id}.json`) for threads/comments                         |
|Reddit rate limits |Request queuing with 1s spacing, exponential backoff, aggressive caching                  |
|Startup behavior   |User configurable (last view or unread items)                                             |
|OPML profiles      |Yes—multiple OPML files as switchable profiles, support both standard and extended formats|
|yt-dlp distribution|Bundle binary with app, auto-update from GitHub releases                                  |

-----

## 9. Remaining Open Questions

1. **Obsidian integration depth**: Simple file export, or deeper integration (backlinks, tags as Obsidian tags, daily notes append)?
1. **Read Later auto-clear**: Auto-remove from queue after reading, or require manual removal?
1. **Export image handling default**: Should local image download be opt-in or opt-out?
1. **Profile sync**: Should profiles be syncable independently, or always as a complete set?
1. **Image cache size limit**: Hard cap on cache size, or let it grow unbounded with manual cleanup?
1. **yt-dlp update frequency**: Check weekly, on app start, or manual only?
1. **yt-dlp fallback**: If extraction fails, show error inline or silently degrade to RSS-only?

-----

## Appendix A: Keyboard Shortcuts Reference

See section 3.5.2

## Appendix B: Theme JSON Schema

See section 3.3.1

## Appendix C: Extended OPML Schema

See section 3.1.2

## Appendix D: Reddit & YouTube Data Models

See sections 3.2.2 and 3.2.3