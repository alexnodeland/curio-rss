// Type-safe Tauri command wrappers

import type {
    Article,
    ArticleFilter,
    ArticlePage,
    CommandError,
    CommentSort,
    Feed,
    FeedUpdate,
    Folder,
    FolderNode,
    Pagination,
    RedditThread,
    RedditUrlInfo,
    RefreshResult,
    YouTubeComment,
    YouTubeMetadata,
    YouTubeUrlInfo,
} from '$lib/types';
import { invoke } from '@tauri-apps/api/core';

// Generic invoke wrapper with error handling
async function typedInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
    try {
        return await invoke<T>(cmd, args);
    } catch (error) {
        throw error as CommandError;
    }
}

// ============================================================================
// Feed Operations
// ============================================================================

export async function addFeed(url: string, folderId?: string, tags?: string[]): Promise<Feed> {
    return typedInvoke<Feed>('add_feed', { url, folder_id: folderId, tags });
}

export async function getFeeds(): Promise<Feed[]> {
    return typedInvoke<Feed[]>('get_feeds');
}

export async function getFeed(feedId: string): Promise<Feed> {
    return typedInvoke<Feed>('get_feed', { feed_id: feedId });
}

export async function updateFeed(feedId: string, updates: FeedUpdate): Promise<Feed> {
    return typedInvoke<Feed>('update_feed', { feed_id: feedId, updates });
}

export async function deleteFeed(feedId: string): Promise<void> {
    return typedInvoke<void>('delete_feed', { feed_id: feedId });
}

export async function refreshFeed(feedId: string): Promise<RefreshResult> {
    return typedInvoke<RefreshResult>('refresh_feed', { feed_id: feedId });
}

export async function refreshAllFeeds(): Promise<RefreshResult[]> {
    return typedInvoke<RefreshResult[]>('refresh_all_feeds');
}

// ============================================================================
// Article Operations
// ============================================================================

export async function getArticles(
    filter: ArticleFilter,
    pagination: Pagination,
): Promise<ArticlePage> {
    return typedInvoke<ArticlePage>('get_articles', { filter, pagination });
}

export async function getArticle(articleId: string): Promise<Article> {
    return typedInvoke<Article>('get_article', { article_id: articleId });
}

export async function markRead(articleIds: string[]): Promise<void> {
    return typedInvoke<void>('mark_read', { article_ids: articleIds });
}

export async function markUnread(articleIds: string[]): Promise<void> {
    return typedInvoke<void>('mark_unread', { article_ids: articleIds });
}

export async function toggleStarred(articleId: string): Promise<boolean> {
    return typedInvoke<boolean>('toggle_starred', { article_id: articleId });
}

export async function searchArticles(query: string, limit?: number): Promise<Article[]> {
    return typedInvoke<Article[]>('search_articles', { query, limit });
}

// ============================================================================
// Read Later Operations
// ============================================================================

export async function addToReadLater(articleId: string): Promise<number> {
    return typedInvoke<number>('add_to_read_later', { article_id: articleId });
}

export async function removeFromReadLater(articleId: string): Promise<void> {
    return typedInvoke<void>('remove_from_read_later', { article_id: articleId });
}

export async function getReadLater(): Promise<Article[]> {
    return typedInvoke<Article[]>('get_read_later');
}

export async function reorderReadLater(articleIds: string[]): Promise<void> {
    return typedInvoke<void>('reorder_read_later', { article_ids: articleIds });
}

// ============================================================================
// Folder Operations
// ============================================================================

export async function createFolder(name: string, parentId?: string): Promise<Folder> {
    return typedInvoke<Folder>('create_folder', { name, parent_id: parentId });
}

export async function getFolders(): Promise<Folder[]> {
    return typedInvoke<Folder[]>('get_folders');
}

export async function getFolderTree(): Promise<FolderNode[]> {
    return typedInvoke<FolderNode[]>('get_folder_tree');
}

export async function deleteFolder(folderId: string): Promise<void> {
    return typedInvoke<void>('delete_folder', { folder_id: folderId });
}

// ============================================================================
// Reddit Operations
// ============================================================================

export async function fetchRedditThread(
    subreddit: string,
    postId: string,
    sort?: CommentSort,
): Promise<RedditThread> {
    return typedInvoke<RedditThread>('fetch_reddit_thread', {
        subreddit,
        post_id: postId,
        sort,
    });
}

export async function parseRedditUrl(url: string): Promise<RedditUrlInfo> {
    return typedInvoke<RedditUrlInfo>('parse_reddit_url', { url });
}

// ============================================================================
// YouTube Operations
// ============================================================================

export async function fetchYouTubeMetadata(videoUrl: string): Promise<YouTubeMetadata> {
    return typedInvoke<YouTubeMetadata>('fetch_youtube_metadata', { video_url: videoUrl });
}

export async function fetchYouTubeComments(videoUrl: string): Promise<YouTubeComment[]> {
    return typedInvoke<YouTubeComment[]>('fetch_youtube_comments', { video_url: videoUrl });
}

export async function checkYtdlpVersion(): Promise<string> {
    return typedInvoke<string>('check_ytdlp_version');
}

export async function parseYouTubeUrl(url: string): Promise<YouTubeUrlInfo> {
    return typedInvoke<YouTubeUrlInfo>('parse_youtube_url', { url });
}

// ============================================================================
// Export Operations
// ============================================================================

export async function exportToMarkdown(
    articleId: string,
    destinationPath?: string,
): Promise<string> {
    return typedInvoke<string>('export_to_markdown', {
        article_id: articleId,
        destination_path: destinationPath,
    });
}

export async function copyAsMarkdown(articleId: string): Promise<string> {
    return typedInvoke<string>('copy_as_markdown', { article_id: articleId });
}

export async function generateExportFilename(articleId: string): Promise<string> {
    return typedInvoke<string>('generate_export_filename', { article_id: articleId });
}

export async function exportToObsidian(articleId: string): Promise<string> {
    return typedInvoke<string>('export_to_obsidian', { article_id: articleId });
}

export async function batchExportMarkdown(
    articleIds: string[],
    destinationDir: string,
): Promise<string[]> {
    return typedInvoke<string[]>('batch_export_markdown', {
        article_ids: articleIds,
        destination_dir: destinationDir,
    });
}
