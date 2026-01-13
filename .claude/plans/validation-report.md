# Curio Reader: PRD/Brief Validation Report

> Generated: 2026-01-13
> Status: Implementation Phase 1 (Foundation) ~70% Complete

---

## Executive Summary

The Curio Reader implementation has strong foundations with well-architected backend code, comprehensive data models, and solid database schema. **The infrastructure for most features exists, but UI components and several integrations need completion.**

| Category | Progress | Notes |
|----------|----------|-------|
| Feed Management | 75% | CRUD complete, OPML/profiles need work |
| View Modes | 40% | Backend structures ready, UI components missing |
| Theming System | 30% | Store/settings defined, no actual themes |
| Performance | 35% | Conditional GET done, caching infrastructure missing |
| Reading Experience | 60% | Core features done, export needs polish |
| Platform Integration | 45% | Reddit/YouTube API ready, yt-dlp integration partial |

---

## Section 3.1: Feed Management

### 3.1.1 OPML Profiles

| Requirement | Status | Notes |
|-------------|--------|-------|
| Multiple OPML files as profiles | ❌ Missing | No profile switching infrastructure |
| Quick switch via sidebar/keyboard | ❌ Missing | Need UI and state management |
| Profile storage in config dir | ❌ Missing | Paths not configured |
| Default profile on first launch | ❌ Missing | |
| Import standard OPML 1.0/2.0 | ⚠️ Partial | `opml.rs` exists, parsing incomplete |
| Import extended Curio OPML | ❌ Missing | Custom attributes not parsed |
| Export standard/extended OPML | ⚠️ Partial | Export function skeleton exists |
| Merge import | ❌ Missing | |

### 3.1.2 OPML Format Support

| Requirement | Status | Notes |
|-------------|--------|-------|
| Full OPML 1.0/2.0 compatibility | ⚠️ Partial | quick-xml available, parser incomplete |
| Custom `curio:*` attributes | ❌ Missing | Schema defined in PRD only |
| viewMode attribute | ✅ Done | ViewMode enum implemented |
| tags attribute | ✅ Done | Tags as JSON array |
| icon/color attributes | ⚠️ Partial | Fields in Folder model |
| refreshInterval override | ✅ Done | Field in Feed model |
| Platform IDs (channelId, subreddit) | ✅ Done | Extracted on feed add |

### 3.1.3 Feed Icons

| Requirement | Status | Notes |
|-------------|--------|-------|
| Auto favicon scraping | ❌ Missing | No fetch_favicon implementation |
| HTML link parsing fallback | ❌ Missing | scraper crate available |
| Google Favicon fallback | ❌ Missing | |
| User icon override | ⚠️ Partial | icon_url field exists, no UI |
| Auto-resize to standard sizes | ❌ Missing | image crate available |
| Custom icons in config dir | ❌ Missing | |
| Icon in OPML | ⚠️ Partial | Field defined, not exported |

### 3.1.4 Feed Discovery

| Requirement | Status | Notes |
|-------------|--------|-------|
| Auto-detect RSS/Atom from URL | ✅ Done | feed-rs auto-detects |
| YouTube URL → channel ID | ✅ Done | `parse_youtube_url` command |
| Reddit URL → subreddit | ✅ Done | `parse_reddit_url` command |

### 3.1.5 Feed Organization

| Requirement | Status | Notes |
|-------------|--------|-------|
| Drag-and-drop reordering | ❌ Missing | No UI implementation |
| Multi-select bulk operations | ❌ Missing | |
| Smart folders (tag queries) | ❌ Missing | |
| Feed health indicators | ⚠️ Partial | last_error field exists |
| Bulk actions (mark read, etc.) | ⚠️ Partial | Some commands exist |
| Feed statistics | ❌ Missing | |

---

## Section 3.2: View Modes

### 3.2.1 Article View

| Requirement | Status | Notes |
|-------------|--------|-------|
| Three-column layout | ⚠️ Partial | Components exist, need integration |
| Title truncation (2 lines) | ❌ Missing | CSS needed |
| Source + favicon display | ⚠️ Partial | Fields exist |
| Relative date display | ✅ Done | `formatRelativeTime` utility |
| Read/unread indicator | ⚠️ Partial | Field exists, no UI style |
| Content preview | ⚠️ Partial | summary field exists |
| Sanitized HTML rendering | ✅ Done | ammonia sanitization |
| Reader-mode toggle | ❌ Missing | No readability extraction |
| Adjustable font/width | ⚠️ Partial | Settings exist, no UI |
| Image lazy loading | ❌ Missing | |
| Image lightbox | ❌ Missing | |

### 3.2.2 Reddit View

| Requirement | Status | Notes |
|-------------|--------|-------|
| Card-based feed layout | ❌ Missing | Need UI component |
| Vote score display | ✅ Done | reddit_score field |
| Subreddit + author + time | ✅ Done | Model complete |
| Thumbnail caching | ❌ Missing | No cache implementation |
| Comment count indicator | ✅ Done | reddit_num_comments field |
| Thread view via JSON API | ✅ Done | `fetch_reddit_thread` |
| Comment threading/nesting | ✅ Done | RedditComment with replies |
| Comment sorting (Best/Top/New) | ✅ Done | CommentSort enum |
| Rate limit handling | ❌ Missing | No queue/backoff |
| Thread caching (15 min) | ❌ Missing | |
| Markdown rendering | ⚠️ Partial | HTML sanitization only |

### 3.2.3 YouTube View

| Requirement | Status | Notes |
|-------------|--------|-------|
| Grid of video cards | ❌ Missing | Need UI component |
| Thumbnail with duration | ✅ Done | thumbnail_url, youtube_duration |
| Channel name + icon | ⚠️ Partial | channel field exists |
| View count | ✅ Done | In YouTubeMetadata |
| Embedded YouTube player | ❌ Missing | No webview integration |
| yt-dlp metadata extraction | ✅ Done | `fetch_youtube_metadata` |
| yt-dlp comment extraction | ✅ Done | `fetch_youtube_comments` |
| Chapter support | ✅ Done | Chapter struct defined |
| Thumbnail multiple resolutions | ✅ Done | Thumbnail struct |
| yt-dlp bundling | ⚠️ Partial | Path detection, no bundling script |
| yt-dlp auto-update | ⚠️ Partial | `check_ytdlp_version`, no download |
| Picture-in-picture | ❌ Missing | |
| Mini-player mode | ❌ Missing | |

### 3.2.4 Podcast View

| Requirement | Status | Notes |
|-------------|--------|-------|
| Episode list layout | ❌ Missing | Need UI component |
| Episode duration display | ✅ Done | podcast_duration field |
| Progress bar (partial play) | ✅ Done | podcast_progress field |
| Persistent player bar | ❌ Missing | Player store exists |
| Play/pause, skip controls | ⚠️ Partial | Functions in player.ts |
| Playback speed (0.5x-3x) | ✅ Done | setPlaybackRate function |
| Volume control | ✅ Done | setVolume function |
| Sleep timer | ❌ Missing | |
| Queue management | ✅ Done | playerQueue store |
| Download episodes | ⚠️ Partial | is_downloaded field, no logic |
| Auto-download settings | ✅ Done | Settings defined |
| Background playback | ❌ Missing | |

### 3.2.5 Gallery View

| Requirement | Status | Notes |
|-------------|--------|-------|
| Masonry grid layout | ❌ Missing | ViewMode enum only |
| Image extraction from content | ❌ Missing | |
| Title overlay on hover | ❌ Missing | |
| Lightbox navigation | ❌ Missing | |

---

## Section 3.3: Theming System

### 3.3.1 Theme Structure

| Requirement | Status | Notes |
|-------------|--------|-------|
| Theme interface defined | ✅ Done | Theme type in types/index.ts |
| Color system | ✅ Done | Full color properties |
| Typography settings | ✅ Done | Font families, sizes |
| Reader-specific typography | ✅ Done | readerFontFamily, etc. |
| Spacing/sizing system | ✅ Done | borderRadius, sidebarWidth |
| Effects (transparency, blur) | ✅ Done | Settings defined |

### 3.3.2 Built-in Themes

| Requirement | Status | Notes |
|-------------|--------|-------|
| Light theme | ❌ Missing | No theme data |
| Dark theme | ❌ Missing | No theme data |
| Nord theme | ❌ Missing | No theme data |
| Catppuccin variants (4) | ❌ Missing | No theme data |
| Solarized (Light/Dark) | ❌ Missing | No theme data |
| Dracula | ❌ Missing | No theme data |
| Transparent Dark | ❌ Missing | No theme data |

### 3.3.3 Custom Themes

| Requirement | Status | Notes |
|-------------|--------|-------|
| JSON theme editor | ❌ Missing | ThemePicker exists, no editor |
| Live preview | ❌ Missing | |
| Import/export themes | ❌ Missing | |

### 3.3.4 Transparency & Effects

| Requirement | Status | Notes |
|-------------|--------|-------|
| Window transparency slider | ⚠️ Partial | Setting exists, no CSS |
| Background blur intensity | ⚠️ Partial | Setting exists, no CSS |
| Vibrancy/Mica effects | ❌ Missing | |
| Custom titlebar | ❌ Missing | Default Tauri titlebar |

---

## Section 3.4: Performance Features

### 3.4.1 Fast Refresh (500+ Feed Scale)

| Requirement | Status | Notes |
|-------------|--------|-------|
| Parallel fetching | ✅ Done | refresh.rs with concurrency |
| Configurable concurrency | ✅ Done | fetch_concurrency setting |
| Conditional GET (ETag) | ✅ Done | etag/last_modified headers |
| Delta updates | ✅ Done | upsert_articles |
| Background refresh on interval | ⚠️ Partial | RefreshService exists, no scheduler |
| Priority queue (recent feeds) | ❌ Missing | |
| Feed batching by domain | ❌ Missing | |
| Staggered refresh | ❌ Missing | |
| Smart scheduling | ❌ Missing | |
| Progress indicator | ❌ Missing | Need frontend |
| Per-feed interval override | ✅ Done | refresh_interval field |

### 3.4.2 Local Image Cache

| Requirement | Status | Notes |
|-------------|--------|-------|
| Cache directory structure | ⚠️ Partial | Paths in settings, no impl |
| Feed icons caching | ❌ Missing | |
| Article images caching | ❌ Missing | |
| Thumbnails caching | ❌ Missing | |
| Avatars caching | ❌ Missing | |
| TTL configuration | ✅ Done | Settings defined |
| Size limits | ✅ Done | Settings defined |
| Lazy loading | ❌ Missing | |
| Content hash deduplication | ❌ Missing | |
| LRU eviction | ❌ Missing | |
| Cache stats UI | ❌ Missing | |

### 3.4.3 Startup Optimization

| Requirement | Status | Notes |
|-------------|--------|-------|
| < 3 second cold start | ⚠️ Unknown | No benchmarks |
| Restore window state | ⚠️ Partial | Settings exist |
| Load feed tree fast | ✅ Done | get_folder_tree query |
| Paginated article load | ✅ Done | Pagination support |
| Virtualized lists | ❌ Missing | |
| Keyset pagination | ⚠️ Partial | Offset-based currently |
| Incremental folder tree | ❌ Missing | |

---

## Section 3.5: Reading Experience

### 3.5.1 Article Management

| Requirement | Status | Notes |
|-------------|--------|-------|
| Read/unread status | ✅ Done | is_read field, commands |
| Star/favorite | ✅ Done | is_starred field, toggle |
| Read Later queue | ✅ Done | Full implementation |
| Archive | ✅ Done | is_archived field |
| Full-text search | ✅ Done | FTS5 backend |
| Markdown export | ✅ Done | export.rs commands |

### 3.5.4 Read Later Queue

| Requirement | Status | Notes |
|-------------|--------|-------|
| Add via keyboard/menu | ⚠️ Partial | Backend ready, no UI |
| Dedicated sidebar section | ❌ Missing | Need UI |
| Order by date added | ✅ Done | read_later_at field |
| Manual sort | ✅ Done | reorder_read_later command |
| Auto-remove option | ✅ Done | read_later_auto_remove setting |
| Badge count | ⚠️ Partial | Count available, no badge UI |

### 3.5.5 Markdown Export

| Requirement | Status | Notes |
|-------------|--------|-------|
| Sanitization stage | ✅ Done | ammonia |
| Readability extraction | ❌ Missing | No Mozilla algorithm |
| Markdown conversion | ✅ Done | htmd crate |
| Clean & format | ⚠️ Partial | Basic formatting |
| YAML frontmatter | ✅ Done | Configurable |
| Batch export | ✅ Done | batch_export_markdown |
| Obsidian integration | ✅ Done | export_to_obsidian command |
| Image handling modes | ⚠️ Partial | Settings exist, not implemented |
| Export settings UI | ❌ Missing | |

### 3.5.2 Keyboard Navigation

| Requirement | Status | Notes |
|-------------|--------|-------|
| j/k navigation | ⚠️ Partial | Handler exists in +page.svelte |
| Enter to open | ⚠️ Partial | Handler exists |
| v open in browser | ⚠️ Partial | Handler exists |
| s star/unstar | ⚠️ Partial | Handler exists |
| l toggle read later | ⚠️ Partial | Handler exists |
| m toggle read/unread | ⚠️ Partial | Handler exists |
| e export to Markdown | ❌ Missing | |
| / focus search | ⚠️ Partial | Handler exists |
| g-prefix navigation | ❌ Missing | |
| View mode shortcuts (1-5) | ⚠️ Partial | Handler exists |
| t toggle sidebar | ⚠️ Partial | Handler exists |
| ? show shortcuts | ❌ Missing | Need modal |

---

## Section 3.6: System Integration

### 3.6.1 Notifications

| Requirement | Status | Notes |
|-------------|--------|-------|
| Native OS notifications | ❌ Missing | |
| Per-feed notification config | ✅ Done | notify_new field |
| Badge count on dock | ❌ Missing | |
| Notification grouping | ❌ Missing | |

### 3.6.2 Global Shortcuts

| Requirement | Status | Notes |
|-------------|--------|-------|
| Show/hide window hotkey | ❌ Missing | |
| Quick-add feed shortcut | ❌ Missing | |

### 3.6.3 Protocol Handler

| Requirement | Status | Notes |
|-------------|--------|-------|
| feed:// handler | ❌ Missing | |
| rss:// handler | ❌ Missing | |

---

## Section 4: Technical Architecture

### 4.2 Data Models

| Model | Status | Notes |
|-------|--------|-------|
| Feed | ✅ Complete | All fields implemented |
| ViewMode enum | ✅ Complete | All 5 variants |
| Folder | ✅ Complete | Parent-child support |
| Article | ✅ Complete | All fields + platform-specific |
| MediaAttachment | ✅ Complete | Download tracking |
| Settings | ✅ Complete | Comprehensive |
| ExportSettings | ✅ Complete | |
| StartupBehavior | ✅ Complete | |
| ViewState | ⚠️ Partial | Not serialized for restore |
| RedditPost/Comment | ✅ Complete | |
| YouTubeMetadata/Comment | ✅ Complete | |

### 4.3 Database Schema

| Table | Status | Notes |
|-------|--------|-------|
| schema_version | ✅ Complete | Migration tracking |
| folders | ✅ Complete | Cascade deletes |
| feeds | ✅ Complete | All fields |
| articles | ✅ Complete | FTS triggers |
| media_attachments | ✅ Complete | |
| articles_fts | ✅ Complete | FTS5 with sync |
| Indexes | ✅ Complete | All query optimizations |

### 4.4 Tauri Commands

| Category | Implemented | Total | Notes |
|----------|-------------|-------|-------|
| Feed Management | 7/7 | 100% | Complete |
| Folder Management | 4/4 | 100% | Complete |
| Article Operations | 8/10 | 80% | archive_articles missing |
| Read Later | 4/4 | 100% | Complete |
| Markdown Export | 5/6 | 83% | parse_article_content missing |
| Reddit | 2/2 | 100% | Complete |
| YouTube | 4/5 | 80% | update_ytdlp missing |
| Profile Management | 0/5 | 0% | Not implemented |
| Feed Icons | 0/3 | 0% | Not implemented |
| Image Cache | 0/3 | 0% | Not implemented |
| OPML | 0/2 | 0% | Partially started |
| Settings | 2/2 | 100% | Complete |
| Themes | 0/2 | 0% | Not implemented |
| Podcast | 0/3 | 0% | Not implemented |

---

## Critical Gaps for MVP (Phase 1)

### Must Have (Blockers)

1. **OPML Import** - Users need to import existing subscriptions
2. **Feed Favicon** - Visual identity for feeds
3. **Built-in Themes** - At least Light/Dark
4. **Article View UI** - List + reader pane components
5. **Sidebar UI** - Folder tree, quick access sections
6. **Search UI** - Connect FTS to frontend

### Should Have (Important)

1. **Background Refresh** - Schedule the existing RefreshService
2. **Read Later UI** - Badge count, dedicated section
3. **Keyboard Shortcuts** - Complete j/k navigation
4. **Basic Settings UI** - Theme picker, interval config
5. **Relative Time Display** - "2h ago" formatting

### Nice to Have (Can Defer)

1. Feed statistics
2. Custom icons
3. Notifications
4. Export settings UI
5. Reader mode

---

## Recommendations

### Immediate Priority (Next Sprint)

1. **UI Component Build-out**
   - ArticleList.svelte - virtual list of article cards
   - Sidebar.svelte - folder tree with quick access
   - ReaderPane.svelte - article content display
   - SearchBar.svelte - connect to FTS

2. **Theme Implementation**
   - Create Light/Dark theme JSON
   - Apply CSS variables from theme
   - ThemePicker component

3. **OPML Import**
   - Complete `opml.rs` parsing
   - Add `import_opml` command
   - Handle folder structure

4. **Background Refresh**
   - Wire up interval timer in RefreshService
   - Emit events for UI progress

### Technical Debt

1. Add integration tests for database operations
2. Add E2E tests for critical flows
3. Implement proper error toasts in UI
4. Add loading states for async operations

---

## Conclusion

The Curio Reader backend is **well-architected** with ~70% of Phase 1 infrastructure complete. The data models, database schema, and API layer are solid foundations.

**Primary gaps are:**
1. Frontend UI components (30% implemented)
2. Theme data (0% implemented)
3. OPML import (50% implemented)
4. Image caching (0% implemented)
5. Profile system (0% implemented)

The codebase is ready for rapid frontend development. Focus should be on UI components and connecting existing backend capabilities to the user interface.
