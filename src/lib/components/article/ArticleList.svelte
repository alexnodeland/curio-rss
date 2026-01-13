<script lang="ts">
import {
    currentArticles,
    selectedFeed,
    selectedArticleId,
    isLoading,
    markArticlesRead,
    refreshFeed,
    selectArticle,
} from '$lib/stores/feeds';
import { formatRelativeTime } from '$lib/utils/format';
import type { Article } from '$lib/types';

function handleMarkAllRead() {
    const unreadIds = $currentArticles.filter((a: Article) => !a.is_read).map((a: Article) => a.id);
    if (unreadIds.length > 0) {
        markArticlesRead(unreadIds);
    }
}
</script>

<div class="article-list">
    <header class="list-header">
        <h2 class="list-title">
            {$selectedFeed ? $selectedFeed.title : 'All Items'}
        </h2>
        <div class="list-actions">
            {#if $selectedFeed}
                <button on:click={() => refreshFeed($selectedFeed.id)} title="Refresh feed">
                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 12a9 9 0 11-6.219-8.56"/>
                        <path d="M21 3v9h-9"/>
                    </svg>
                </button>
            {/if}
            <button on:click={handleMarkAllRead} title="Mark all as read">
                <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M22 11.08V12a10 10 0 11-5.93-9.14"/>
                    <polyline points="22 4 12 14.01 9 11.01"/>
                </svg>
            </button>
        </div>
    </header>

    <div class="list-content">
        {#if $isLoading}
            <div class="loading">Loading articles...</div>
        {:else if $currentArticles.length === 0}
            <div class="empty">No articles found</div>
        {:else}
            {#each $currentArticles as article (article.id)}
                <button
                    class="article-item"
                    class:unread={!article.is_read}
                    class:selected={$selectedArticleId === article.id}
                    on:click={() => selectArticle(article.id)}
                >
                    {#if article.thumbnail_url}
                        <img src={article.thumbnail_url} alt="" class="thumbnail" />
                    {/if}
                    <div class="article-content">
                        <h3 class="article-title">{article.title}</h3>
                        <div class="article-meta">
                            <span class="article-source">
                                {article.author ?? 'Unknown'}
                            </span>
                            <span class="article-date">
                                {formatRelativeTime(article.published_at)}
                            </span>
                        </div>
                        {#if article.summary}
                            <p class="article-summary">{article.summary}</p>
                        {/if}
                    </div>
                    <div class="article-indicators">
                        {#if article.is_starred}
                            <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="currentColor" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" class="star">
                                <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
                            </svg>
                        {/if}
                        {#if !article.is_read}
                            <span class="unread-dot" />
                        {/if}
                    </div>
                </button>
            {/each}
        {/if}
    </div>
</div>

<style>
    .article-list {
        width: var(--list-width);
        height: 100%;
        background: var(--bg-secondary);
        border-right: 1px solid var(--border);
        display: flex;
        flex-direction: column;
        overflow: hidden;
    }

    .list-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-3) var(--space-4);
        border-bottom: 1px solid var(--border);
    }

    .list-title {
        font-size: 14px;
        font-weight: 600;
        color: var(--fg);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .list-actions {
        display: flex;
        gap: var(--space-2);
    }

    .list-actions button {
        padding: var(--space-1);
        color: var(--fg-muted);
        border-radius: var(--radius-sm);
    }

    .list-actions button:hover {
        background: var(--bg-tertiary);
        color: var(--fg);
    }

    .list-content {
        flex: 1;
        overflow-y: auto;
    }

    .loading,
    .empty {
        padding: var(--space-6);
        text-align: center;
        color: var(--fg-muted);
    }

    .article-item {
        display: flex;
        gap: var(--space-3);
        padding: var(--space-3) var(--space-4);
        width: 100%;
        text-align: left;
        border-bottom: 1px solid var(--border-subtle);
        color: var(--read);
    }

    .article-item:hover {
        background: var(--bg-tertiary);
    }

    .article-item.selected {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .article-item.unread {
        color: var(--fg);
    }

    .thumbnail {
        width: 64px;
        height: 48px;
        object-fit: cover;
        border-radius: var(--radius-sm);
        flex-shrink: 0;
    }

    .article-content {
        flex: 1;
        min-width: 0;
    }

    .article-title {
        font-size: 14px;
        font-weight: 500;
        line-height: 1.4;
        margin-bottom: var(--space-1);
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }

    .article-item.selected .article-title {
        color: var(--accent-fg);
    }

    .article-meta {
        display: flex;
        gap: var(--space-2);
        font-size: 12px;
        color: var(--fg-subtle);
    }

    .article-item.selected .article-meta {
        color: var(--accent-fg);
        opacity: 0.8;
    }

    .article-summary {
        font-size: 12px;
        color: var(--fg-muted);
        margin-top: var(--space-1);
        display: -webkit-box;
        -webkit-line-clamp: 2;
        -webkit-box-orient: vertical;
        overflow: hidden;
    }

    .article-item.selected .article-summary {
        color: var(--accent-fg);
        opacity: 0.9;
    }

    .article-indicators {
        display: flex;
        flex-direction: column;
        align-items: center;
        gap: var(--space-1);
        flex-shrink: 0;
    }

    .star {
        color: var(--warning);
    }

    .article-item.selected .star {
        color: var(--accent-fg);
    }

    .unread-dot {
        width: 8px;
        height: 8px;
        background: var(--accent);
        border-radius: 50%;
    }

    .article-item.selected .unread-dot {
        background: var(--accent-fg);
    }
</style>
