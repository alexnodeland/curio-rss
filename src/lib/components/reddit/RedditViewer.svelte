<script lang="ts">
import { fetchRedditThread } from '$lib/api';
import type { Article, CommentSort, RedditThread } from '$lib/types';
import { formatRelativeTime, formatNumber } from '$lib/utils/format';
import RedditComment from './RedditComment.svelte';

export let article: Article;

let thread: RedditThread | null = null;
let isLoading = true;
let error = '';
let commentSort: CommentSort = 'best';

// Extract post info from article URL
function extractPostInfo(url: string): { subreddit: string; postId: string } | null {
    const match = url.match(/reddit\.com\/r\/([^/]+)\/comments\/([^/]+)/);
    if (match) {
        return { subreddit: match[1], postId: match[2] };
    }
    return null;
}

async function loadThread() {
    if (!article.url) {
        error = 'No Reddit URL available';
        isLoading = false;
        return;
    }

    const postInfo = extractPostInfo(article.url);
    if (!postInfo) {
        error = 'Invalid Reddit URL';
        isLoading = false;
        return;
    }

    isLoading = true;
    error = '';

    try {
        thread = await fetchRedditThread(postInfo.subreddit, postInfo.postId, commentSort);
    } catch (e) {
        error = e instanceof Error ? e.message : 'Failed to load Reddit thread';
    } finally {
        isLoading = false;
    }
}

async function changeSort(newSort: CommentSort) {
    commentSort = newSort;
    await loadThread();
}

function getPostTime(): string {
    if (!thread) return '';
    const date = new Date(thread.post.created_utc * 1000);
    return formatRelativeTime(date.toISOString());
}

function openInReddit() {
    if (thread?.post.permalink) {
        window.open(`https://reddit.com${thread.post.permalink}`, '_blank');
    } else if (article.url) {
        window.open(article.url, '_blank');
    }
}

// Load thread when article changes
$: if (article) loadThread();
</script>

<div class="reddit-viewer">
    {#if isLoading}
        <div class="loading">
            <div class="loading-spinner"></div>
            <span>Loading Reddit thread...</span>
        </div>
    {:else if error}
        <div class="error">
            <p>{error}</p>
            <button on:click={loadThread}>Try Again</button>
        </div>
    {:else if thread}
        <article class="reddit-post">
            <header class="post-header">
                <div class="post-subreddit">
                    <span class="subreddit-link">r/{thread.post.subreddit}</span>
                    <span class="post-meta">
                        Posted by u/{thread.post.author}
                        {#if thread.post.author_flair}
                            <span class="author-flair">{thread.post.author_flair}</span>
                        {/if}
                        {getPostTime()}
                    </span>
                </div>
            </header>

            <h1 class="post-title">
                {#if thread.post.flair_text}
                    <span class="post-flair">{thread.post.flair_text}</span>
                {/if}
                {thread.post.title}
            </h1>

            <div class="post-stats">
                <span class="stat" class:positive={thread.post.score > 0} class:negative={thread.post.score < 0}>
                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M12 19V5M5 12l7-7 7 7"/>
                    </svg>
                    {formatNumber(thread.post.score)}
                </span>
                <span class="stat">
                    <svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>
                    </svg>
                    {formatNumber(thread.post.num_comments)} comments
                </span>
                <span class="stat upvote-ratio">
                    {Math.round(thread.post.upvote_ratio * 100)}% upvoted
                </span>
                <button class="open-btn" on:click={openInReddit}>
                    Open in Reddit
                </button>
            </div>

            {#if thread.post.selftext_html}
                <div class="post-body">
                    {@html thread.post.selftext_html}
                </div>
            {:else if thread.post.link_url && !thread.post.is_self}
                <div class="post-link">
                    <a href={thread.post.link_url} target="_blank" rel="noopener noreferrer">
                        {#if thread.post.thumbnail}
                            <img src={thread.post.thumbnail} alt="" class="link-thumbnail" />
                        {/if}
                        <span class="link-url">{thread.post.link_url}</span>
                    </a>
                </div>
            {/if}
        </article>

        <div class="comments-section">
            <header class="comments-header">
                <h2>Comments ({formatNumber(thread.post.num_comments)})</h2>
                <div class="sort-options">
                    <span>Sort by:</span>
                    <select bind:value={commentSort} on:change={() => changeSort(commentSort)}>
                        <option value="best">Best</option>
                        <option value="top">Top</option>
                        <option value="new">New</option>
                        <option value="controversial">Controversial</option>
                        <option value="old">Old</option>
                    </select>
                </div>
            </header>

            <div class="comments-list">
                {#if thread.comments.length === 0}
                    <p class="no-comments">No comments yet</p>
                {:else}
                    {#each thread.comments as comment (comment.reddit_id)}
                        <RedditComment {comment} />
                    {/each}
                {/if}
            </div>
        </div>
    {/if}
</div>

<style>
    .reddit-viewer {
        flex: 1;
        height: 100%;
        overflow-y: auto;
        background: var(--bg);
    }

    .loading,
    .error {
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: var(--space-3);
        padding: var(--space-8);
        color: var(--fg-muted);
    }

    .loading-spinner {
        width: 32px;
        height: 32px;
        border: 3px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 1s linear infinite;
    }

    @keyframes spin {
        to { transform: rotate(360deg); }
    }

    .error {
        color: var(--error);
    }

    .error button {
        padding: var(--space-2) var(--space-4);
        background: var(--accent);
        color: var(--accent-fg);
        border-radius: var(--radius-md);
    }

    .reddit-post {
        padding: var(--space-6);
        border-bottom: 1px solid var(--border);
    }

    .post-header {
        margin-bottom: var(--space-3);
    }

    .post-subreddit {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
    }

    .subreddit-link {
        font-weight: 600;
        color: var(--fg);
    }

    .post-meta {
        font-size: 12px;
        color: var(--fg-muted);
    }

    .author-flair {
        display: inline-block;
        padding: 0 var(--space-1);
        background: var(--bg-tertiary);
        border-radius: var(--radius-sm);
        font-size: 11px;
        margin: 0 var(--space-1);
    }

    .post-title {
        font-size: 20px;
        font-weight: 600;
        line-height: 1.3;
        color: var(--fg);
        margin-bottom: var(--space-3);
    }

    .post-flair {
        display: inline-block;
        padding: var(--space-1) var(--space-2);
        background: var(--accent-muted);
        color: var(--accent);
        border-radius: var(--radius-sm);
        font-size: 12px;
        font-weight: 500;
        margin-right: var(--space-2);
    }

    .post-stats {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-4);
        margin-bottom: var(--space-4);
        font-size: 13px;
        color: var(--fg-muted);
    }

    .stat {
        display: flex;
        align-items: center;
        gap: var(--space-1);
    }

    .stat.positive {
        color: var(--success);
    }

    .stat.negative {
        color: var(--error);
    }

    .open-btn {
        margin-left: auto;
        padding: var(--space-1) var(--space-3);
        font-size: 12px;
        color: var(--accent);
        border: 1px solid var(--accent);
        border-radius: var(--radius-md);
    }

    .open-btn:hover {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .post-body {
        line-height: 1.6;
        color: var(--fg);
    }

    .post-body :global(p) {
        margin-bottom: 1em;
    }

    .post-body :global(a) {
        color: var(--link);
    }

    .post-body :global(blockquote) {
        border-left: 4px solid var(--border);
        margin: var(--space-3) 0;
        padding-left: var(--space-3);
        color: var(--fg-muted);
    }

    .post-link {
        display: block;
        padding: var(--space-3);
        background: var(--bg-secondary);
        border-radius: var(--radius-md);
        border: 1px solid var(--border);
    }

    .post-link a {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        color: var(--link);
    }

    .link-thumbnail {
        width: 64px;
        height: 48px;
        object-fit: cover;
        border-radius: var(--radius-sm);
    }

    .link-url {
        font-size: 13px;
        word-break: break-all;
    }

    .comments-section {
        padding: var(--space-4);
    }

    .comments-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        margin-bottom: var(--space-4);
    }

    .comments-header h2 {
        font-size: 16px;
        font-weight: 600;
        color: var(--fg);
    }

    .sort-options {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        font-size: 13px;
        color: var(--fg-muted);
    }

    .sort-options select {
        padding: var(--space-1) var(--space-2);
        background: var(--bg-secondary);
        border: 1px solid var(--border);
        border-radius: var(--radius-sm);
        color: var(--fg);
        font-size: 13px;
    }

    .no-comments {
        padding: var(--space-6);
        text-align: center;
        color: var(--fg-muted);
    }

    .comments-list {
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
</style>
