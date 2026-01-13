// Type-safe Tauri command wrappers

import { invoke } from '@tauri-apps/api/core';
import type {
    Feed,
    FeedUpdate,
    Article,
    ArticleFilter,
    ArticlePage,
    Pagination,
    Folder,
    FolderNode,
    RefreshResult,
    CommandError,
    RedditThread,
    RedditUrlInfo,
    CommentSort,
    YouTubeMetadata,
    YouTubeComment,
    YouTubeUrlInfo,
} from '$lib/types';

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

export async function addFeed(
    url: string,
    folderId?: string,
    tags?: string[],
): Promise<Feed> {
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
