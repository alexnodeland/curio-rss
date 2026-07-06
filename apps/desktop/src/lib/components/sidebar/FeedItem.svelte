<script lang="ts">
/**
 * One subscription row: title (URL fallback) plus the backend-owned unread
 * badge. Paused/dead affordances arrive with the feed-health surface (WP5).
 */
import type { FeedDto } from '$lib/bindings';
import { t } from '$lib/i18n';

let {
    feed,
    unread,
    selected,
    onselect,
}: {
    feed: FeedDto;
    unread: number;
    selected: boolean;
    onselect: (feedId: number) => void;
} = $props();
</script>

<button
    class="feed-item"
    class:active={selected}
    aria-current={selected ? 'true' : undefined}
    onclick={() => onselect(feed.id)}
>
    <span class="feed-title truncate">{feed.title ?? feed.url}</span>
    {#if unread > 0}
        <span class="unread-badge" aria-hidden="true">{unread}</span>
        <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
    {/if}
</button>

<style>
    .feed-item {
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg);
        text-align: left;
        font-size: 0.875rem;
    }

    .feed-item:hover {
        background: var(--bg-hover);
    }

    .feed-item.active {
        background: var(--accent-muted);
        color: var(--accent);
    }

    .feed-title {
        flex: 1 1 auto;
        min-width: 0;
    }

    .unread-badge {
        flex: 0 0 auto;
        min-width: 1.5rem;
        padding: 0 var(--space-1);
        border-radius: var(--radius-xl);
        background: var(--bg-tertiary);
        color: var(--fg-muted);
        font-size: 0.75rem;
        font-variant-numeric: tabular-nums;
        text-align: center;
        line-height: 1.5;
    }

    .feed-item.active .unread-badge {
        background: var(--accent);
        color: var(--accent-fg);
    }
</style>
