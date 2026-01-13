<script lang="ts">
import type { RedditComment } from '$lib/types';
import { formatNumber, formatRelativeTime } from '$lib/utils/format';

export let comment: RedditComment;
export const collapsed = false;

let isCollapsed = collapsed || comment.is_collapsed;

function toggleCollapse() {
    isCollapsed = !isCollapsed;
}

function getRelativeTime(): string {
    const date = new Date(comment.created_utc * 1000);
    return formatRelativeTime(date.toISOString());
}

function getScoreDisplay(): string {
    if (comment.score === 0) return '0';
    return formatNumber(comment.score);
}

// Depth-based indentation with max
const MAX_DEPTH = 10;
const effectiveDepth = Math.min(comment.depth, MAX_DEPTH);
</script>

<div
    class="reddit-comment"
    class:collapsed={isCollapsed}
    style="--depth: {effectiveDepth}"
>
    <div class="comment-collapse-bar" on:click={toggleCollapse} role="button" tabindex="0" on:keypress={toggleCollapse} />

    <div class="comment-content">
        <header class="comment-header">
            <button class="collapse-toggle" on:click={toggleCollapse} aria-expanded={!isCollapsed}>
                {isCollapsed ? '[+]' : '[-]'}
            </button>

            <span class="comment-author" class:op={comment.author === '[deleted]'}>
                {comment.author}
            </span>

            {#if comment.author_flair}
                <span class="author-flair">{comment.author_flair}</span>
            {/if}

            <span class="comment-score" class:positive={comment.score > 0} class:negative={comment.score < 0}>
                {getScoreDisplay()} points
            </span>

            <span class="comment-time">
                {getRelativeTime()}
            </span>

            {#if isCollapsed && comment.replies.length > 0}
                <span class="collapsed-info">
                    ({comment.replies.length} {comment.replies.length === 1 ? 'child' : 'children'})
                </span>
            {/if}
        </header>

        {#if !isCollapsed}
            <div class="comment-body">
                {@html comment.body_html}
            </div>

            <div class="comment-actions">
                <button class="action-link">permalink</button>
                <button class="action-link">reply</button>
            </div>

            {#if comment.replies.length > 0}
                <div class="comment-replies">
                    {#each comment.replies as reply (reply.reddit_id)}
                        <svelte:self comment={reply} />
                    {/each}
                </div>
            {/if}
        {/if}
    </div>
</div>

<style>
    .reddit-comment {
        display: flex;
        gap: var(--space-2);
        font-size: 13px;
        margin-left: calc(var(--depth) * 16px);
    }

    .comment-collapse-bar {
        width: 2px;
        min-height: 100%;
        background: var(--border);
        cursor: pointer;
        flex-shrink: 0;
    }

    .comment-collapse-bar:hover {
        background: var(--accent);
    }

    .comment-content {
        flex: 1;
        min-width: 0;
        padding: var(--space-2) 0;
    }

    .comment-header {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        font-size: 12px;
        color: var(--fg-muted);
        margin-bottom: var(--space-2);
    }

    .collapse-toggle {
        font-family: var(--font-mono);
        font-size: 10px;
        color: var(--fg-muted);
        padding: 0;
    }

    .collapse-toggle:hover {
        color: var(--accent);
    }

    .comment-author {
        font-weight: 600;
        color: var(--accent);
    }

    .comment-author.op {
        color: var(--fg-muted);
        font-style: italic;
    }

    .author-flair {
        padding: 0 var(--space-1);
        background: var(--bg-tertiary);
        border-radius: var(--radius-sm);
        font-size: 11px;
    }

    .comment-score {
        color: var(--fg-subtle);
    }

    .comment-score.positive {
        color: var(--success);
    }

    .comment-score.negative {
        color: var(--error);
    }

    .comment-time {
        color: var(--fg-subtle);
    }

    .collapsed-info {
        color: var(--fg-subtle);
        font-style: italic;
    }

    .comment-body {
        color: var(--fg);
        line-height: 1.5;
    }

    .comment-body :global(p) {
        margin-bottom: 0.75em;
    }

    .comment-body :global(p:last-child) {
        margin-bottom: 0;
    }

    .comment-body :global(a) {
        color: var(--link);
    }

    .comment-body :global(blockquote) {
        border-left: 3px solid var(--border);
        margin: var(--space-2) 0;
        padding-left: var(--space-3);
        color: var(--fg-muted);
    }

    .comment-body :global(code) {
        background: var(--bg-secondary);
        padding: 0.1em 0.3em;
        border-radius: var(--radius-sm);
        font-family: var(--font-mono);
        font-size: 0.9em;
    }

    .comment-body :global(pre) {
        background: var(--bg-secondary);
        padding: var(--space-2);
        border-radius: var(--radius-sm);
        overflow-x: auto;
    }

    .comment-actions {
        display: flex;
        gap: var(--space-3);
        margin-top: var(--space-2);
    }

    .action-link {
        font-size: 11px;
        color: var(--fg-muted);
        padding: 0;
    }

    .action-link:hover {
        color: var(--accent);
        text-decoration: underline;
    }

    .comment-replies {
        margin-top: var(--space-2);
    }

    .reddit-comment.collapsed .comment-body,
    .reddit-comment.collapsed .comment-actions,
    .reddit-comment.collapsed .comment-replies {
        display: none;
    }

    .reddit-comment.collapsed .comment-collapse-bar {
        background: var(--fg-subtle);
    }
</style>
