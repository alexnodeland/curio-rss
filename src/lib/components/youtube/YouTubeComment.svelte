<script lang="ts">
import type { YouTubeComment as YTComment } from '$lib/types';
import { formatRelativeTime, formatNumber } from '$lib/utils/format';

export let comment: YTComment;

function getCommentTime(): string {
    if (!comment.timestamp) return '';
    const date = new Date(comment.timestamp * 1000);
    return formatRelativeTime(date.toISOString());
}
</script>

<div class="youtube-comment" class:pinned={comment.is_pinned}>
    <img
        src={comment.author_thumbnail}
        alt={comment.author}
        class="author-avatar"
    />

    <div class="comment-content">
        <header class="comment-header">
            <span class="comment-author" class:uploader={comment.author_is_uploader}>
                {comment.author}
            </span>
            {#if comment.author_is_uploader}
                <span class="uploader-badge">Creator</span>
            {/if}
            {#if comment.is_pinned}
                <span class="pinned-badge">
                    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                        <path d="M16 12V4h1V2H7v2h1v8l-2 2v2h5.2v6h1.6v-6H18v-2l-2-2z"/>
                    </svg>
                    Pinned
                </span>
            {/if}
            {#if comment.is_favorited}
                <span class="favorited-badge">
                    <svg xmlns="http://www.w3.org/2000/svg" width="12" height="12" viewBox="0 0 24 24" fill="currentColor">
                        <path d="M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z"/>
                    </svg>
                </span>
            {/if}
            <span class="comment-time">{getCommentTime()}</span>
        </header>

        <div class="comment-text">
            {comment.text}
        </div>

        <div class="comment-actions">
            {#if comment.like_count !== null && comment.like_count > 0}
                <span class="likes">
                    <svg xmlns="http://www.w3.org/2000/svg" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                        <path d="M14 9V5a3 3 0 0 0-3-3l-4 9v11h11.28a2 2 0 0 0 2-1.7l1.38-9a2 2 0 0 0-2-2.3zM7 22H4a2 2 0 0 1-2-2v-7a2 2 0 0 1 2-2h3"/>
                    </svg>
                    {formatNumber(comment.like_count)}
                </span>
            {/if}
        </div>
    </div>
</div>

<style>
    .youtube-comment {
        display: flex;
        gap: var(--space-3);
        padding: var(--space-3);
        border-radius: var(--radius-md);
    }

    .youtube-comment.pinned {
        background: var(--bg-secondary);
        border: 1px solid var(--accent-muted);
    }

    .author-avatar {
        width: 40px;
        height: 40px;
        border-radius: 50%;
        flex-shrink: 0;
    }

    .comment-content {
        flex: 1;
        min-width: 0;
    }

    .comment-header {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        margin-bottom: var(--space-1);
    }

    .comment-author {
        font-size: 13px;
        font-weight: 600;
        color: var(--fg);
    }

    .comment-author.uploader {
        padding: var(--space-1) var(--space-2);
        background: var(--bg-tertiary);
        border-radius: var(--radius-sm);
    }

    .uploader-badge {
        padding: 2px 6px;
        font-size: 11px;
        font-weight: 500;
        color: var(--accent-fg);
        background: var(--accent);
        border-radius: var(--radius-sm);
    }

    .pinned-badge {
        display: flex;
        align-items: center;
        gap: 4px;
        padding: 2px 6px;
        font-size: 11px;
        color: var(--fg-muted);
        background: var(--bg-tertiary);
        border-radius: var(--radius-sm);
    }

    .favorited-badge {
        color: var(--error);
    }

    .comment-time {
        font-size: 12px;
        color: var(--fg-muted);
    }

    .comment-text {
        font-size: 14px;
        line-height: 1.5;
        color: var(--fg);
        white-space: pre-wrap;
        word-wrap: break-word;
    }

    .comment-actions {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        margin-top: var(--space-2);
    }

    .likes {
        display: flex;
        align-items: center;
        gap: var(--space-1);
        font-size: 12px;
        color: var(--fg-muted);
    }
</style>
