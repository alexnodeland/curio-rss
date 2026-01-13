// Feed and article state management

import * as api from '$lib/api';
import type { Article, ArticleFilter, Feed, Folder, FolderNode, Pagination } from '$lib/types';
import { derived, get, writable } from 'svelte/store';

// ============================================================================
// Base Stores (Source of Truth)
// ============================================================================

export const feeds = writable<Map<string, Feed>>(new Map());
export const folders = writable<Map<string, Folder>>(new Map());
export const folderTree = writable<FolderNode[]>([]);
export const articles = writable<Map<string, Article>>(new Map());

// Selection state
export const selectedFeedId = writable<string | null>(null);
export const selectedFolderId = writable<string | null>(null);
export const selectedArticleId = writable<string | null>(null);

// Filter state
export const articleFilter = writable<ArticleFilter>({
    unread_only: false,
    starred_only: false,
    read_later_only: false,
    archived: false,
});

// Pagination state
export const pagination = writable<Pagination>({ page: 1, per_page: 50 });
export const totalArticles = writable<number>(0);

// Loading states
export const isLoading = writable<boolean>(false);
export const isRefreshing = writable<boolean>(false);

// ============================================================================
// Derived Stores (Computed Views)
// ============================================================================

export const currentArticles = derived(
    [articles, selectedFeedId, selectedFolderId, articleFilter],
    ([$articles, $feedId, $folderId, $filter]) => {
        let list = Array.from($articles.values());

        if ($feedId) {
            list = list.filter((a) => a.feed_id === $feedId);
        }

        if ($filter.unread_only) {
            list = list.filter((a) => !a.is_read);
        }

        if ($filter.starred_only) {
            list = list.filter((a) => a.is_starred);
        }

        if ($filter.read_later_only) {
            list = list.filter((a) => a.is_read_later);
        }

        // Sort by published date (newest first)
        return list.sort((a, b) => {
            const dateA = a.published_at ? new Date(a.published_at).getTime() : 0;
            const dateB = b.published_at ? new Date(b.published_at).getTime() : 0;
            return dateB - dateA;
        });
    },
);

export const selectedArticle = derived([articles, selectedArticleId], ([$articles, $articleId]) => {
    if (!$articleId) return null;
    return $articles.get($articleId) ?? null;
});

export const selectedFeed = derived([feeds, selectedFeedId], ([$feeds, $feedId]) => {
    if (!$feedId) return null;
    return $feeds.get($feedId) ?? null;
});

export const unreadCounts = derived([articles, feeds], ([$articles, $feeds]) => {
    const counts = new Map<string, number>();

    for (const [feedId] of $feeds) {
        counts.set(
            feedId,
            Array.from($articles.values()).filter((a) => a.feed_id === feedId && !a.is_read).length,
        );
    }

    return counts;
});

export const totalUnreadCount = derived(unreadCounts, ($counts) => {
    let total = 0;
    for (const count of $counts.values()) {
        total += count;
    }
    return total;
});

// ============================================================================
// Actions
// ============================================================================

export async function loadFeeds(): Promise<void> {
    isLoading.set(true);
    try {
        const feedList = await api.getFeeds();
        const feedMap = new Map<string, Feed>();
        for (const feed of feedList) {
            feedMap.set(feed.id, feed);
        }
        feeds.set(feedMap);
    } finally {
        isLoading.set(false);
    }
}

export async function loadFolderTree(): Promise<void> {
    isLoading.set(true);
    try {
        const tree = await api.getFolderTree();
        folderTree.set(tree);
    } finally {
        isLoading.set(false);
    }
}

export async function loadArticles(): Promise<void> {
    isLoading.set(true);
    try {
        const filter = get(articleFilter);
        const feedId = get(selectedFeedId);
        const folderId = get(selectedFolderId);
        const page = get(pagination);

        const effectiveFilter: ArticleFilter = {
            ...filter,
            feed_id: feedId,
            folder_id: folderId,
        };

        const result = await api.getArticles(effectiveFilter, page);

        const articleMap = new Map<string, Article>();
        for (const article of result.items) {
            articleMap.set(article.id, article);
        }
        articles.set(articleMap);
        totalArticles.set(result.total);
    } finally {
        isLoading.set(false);
    }
}

export async function addFeed(url: string, folderId?: string): Promise<Feed> {
    const feed = await api.addFeed(url, folderId);
    feeds.update((map) => {
        map.set(feed.id, feed);
        return map;
    });
    return feed;
}

export async function deleteFeed(feedId: string): Promise<void> {
    await api.deleteFeed(feedId);
    feeds.update((map) => {
        map.delete(feedId);
        return map;
    });

    // Clear selection if deleted feed was selected
    if (get(selectedFeedId) === feedId) {
        selectedFeedId.set(null);
    }
}

export async function refreshFeed(feedId: string): Promise<void> {
    isRefreshing.set(true);
    try {
        const result = await api.refreshFeed(feedId);

        if (result.new_count > 0 || result.updated_count > 0) {
            // Reload articles if we got new ones
            await loadArticles();
        }

        // Update feed metadata
        const feed = await api.getFeed(feedId);
        feeds.update((map) => {
            map.set(feedId, feed);
            return map;
        });
    } finally {
        isRefreshing.set(false);
    }
}

export async function refreshAllFeeds(): Promise<void> {
    isRefreshing.set(true);
    try {
        await api.refreshAllFeeds();
        await loadFeeds();
        await loadArticles();
    } finally {
        isRefreshing.set(false);
    }
}

export async function markArticlesRead(articleIds: string[]): Promise<void> {
    await api.markRead(articleIds);

    articles.update((map) => {
        for (const id of articleIds) {
            const article = map.get(id);
            if (article) {
                map.set(id, {
                    ...article,
                    is_read: true,
                    read_at: new Date().toISOString(),
                });
            }
        }
        return map;
    });
}

export async function markArticlesUnread(articleIds: string[]): Promise<void> {
    await api.markUnread(articleIds);

    articles.update((map) => {
        for (const id of articleIds) {
            const article = map.get(id);
            if (article) {
                map.set(id, {
                    ...article,
                    is_read: false,
                    read_at: null,
                });
            }
        }
        return map;
    });
}

export async function toggleArticleStarred(articleId: string): Promise<boolean> {
    const isStarred = await api.toggleStarred(articleId);

    articles.update((map) => {
        const article = map.get(articleId);
        if (article) {
            map.set(articleId, { ...article, is_starred: isStarred });
        }
        return map;
    });

    return isStarred;
}

export function selectFeed(feedId: string | null): void {
    selectedFeedId.set(feedId);
    selectedFolderId.set(null);
    selectedArticleId.set(null);
    pagination.set({ page: 1, per_page: 50 });
    loadArticles();
}

export function selectFolder(folderId: string | null): void {
    selectedFolderId.set(folderId);
    selectedFeedId.set(null);
    selectedArticleId.set(null);
    pagination.set({ page: 1, per_page: 50 });
    loadArticles();
}

export function selectArticle(articleId: string | null): void {
    selectedArticleId.set(articleId);

    // Mark as read when selected
    if (articleId) {
        const article = get(articles).get(articleId);
        if (article && !article.is_read) {
            markArticlesRead([articleId]);
        }
    }
}

export function setFilter(filter: Partial<ArticleFilter>): void {
    articleFilter.update((current) => ({ ...current, ...filter }));
    pagination.set({ page: 1, per_page: 50 });
    loadArticles();
}

export function clearFilter(): void {
    articleFilter.set({
        unread_only: false,
        starred_only: false,
        read_later_only: false,
        archived: false,
    });
    pagination.set({ page: 1, per_page: 50 });
    loadArticles();
}
