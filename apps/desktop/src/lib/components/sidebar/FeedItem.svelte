<script lang="ts">
/**
 * One subscription row: a select button (title, URL fallback) carrying the
 * backend-owned unread badge and a lifecycle badge, plus a health control
 * that opens the feed-health panel (recent fetches + pause/resume/revive).
 * The row is a container, not a button, so the health control can be its
 * own focusable target.
 */
import type { FeedDto } from '$lib/bindings';
import { t } from '$lib/i18n';
import FeedHealthBadge from './FeedHealthBadge.svelte';

let {
    feed,
    unread,
    selected,
    onselect,
    onhealth,
}: {
    feed: FeedDto;
    unread: number;
    selected: boolean;
    onselect: (feedId: number) => void;
    onhealth: (feedId: number) => void;
} = $props();
</script>

<div class="feed-item" class:active={selected}>
    <button
        class="feed-select"
        aria-current={selected ? 'true' : undefined}
        onclick={() => onselect(feed.id)}
    >
        <span class="feed-title truncate">{feed.title ?? feed.url}</span>
    </button>
    <FeedHealthBadge status={feed.status} />
    {#if unread > 0}
        <span class="unread-badge" aria-hidden="true">{unread}</span>
        <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
    {/if}
    <button
        class="feed-health"
        type="button"
        aria-label={t('feedHealth.open', { name: feed.title ?? feed.url })}
        onclick={() => onhealth(feed.id)}
    >
        <span class="health-dot health-{feed.status}" aria-hidden="true"></span>
    </button>
</div>

<style>
    .feed-item {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding-right: var(--space-2);
        border-radius: var(--radius-md);
    }

    .feed-item:hover {
        background: var(--bg-hover);
    }

    .feed-item.active {
        background: var(--accent-muted);
    }

    .feed-select {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        align-items: center;
        padding: var(--space-2) var(--space-3);
        background: transparent;
        color: var(--fg);
        text-align: left;
        font-size: 0.875rem;
    }

    .feed-item.active .feed-select {
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

    .feed-health {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: center;
        width: 1.25rem;
        height: 1.25rem;
        border-radius: var(--radius-sm);
        background: transparent;
    }

    .feed-health:hover {
        background: var(--bg-tertiary);
    }

    .health-dot {
        width: 0.5rem;
        height: 0.5rem;
        border-radius: 50%;
        background: var(--fg-subtle);
    }

    .health-active {
        background: var(--success, #37b24d);
    }

    .health-paused {
        background: var(--warning, #f08c00);
    }

    .health-dead {
        background: var(--error);
    }
</style>
