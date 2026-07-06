<script lang="ts">
/**
 * One subscription row: a select button (monogram, title) carrying the
 * backend-owned unread count, plus a health control that opens the
 * feed-health panel. The row is a container, not a button, so the health
 * control can be its own focusable target.
 */
import type { FeedDto } from '$lib/bindings';
import { t } from '$lib/i18n';

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

const label = $derived(feed.title ?? feed.url);

/** A stable per-feed hue for the monogram, derived from the title. */
function hue(text: string): number {
    let hash = 0;
    for (let i = 0; i < text.length; i += 1) {
        hash = (hash * 31 + text.charCodeAt(i)) % 360;
    }
    return hash;
}
</script>

<div class="feed-item" class:active={selected} class:unhealthy={feed.status !== 'active'}>
    <button
        class="feed-select"
        aria-current={selected ? 'true' : undefined}
        onclick={() => onselect(feed.id)}
    >
        <span class="feed-mono" style:--mono-hue={hue(label)} aria-hidden="true"
            >{label.slice(0, 1).toUpperCase()}</span
        >
        <span class="feed-title truncate">{label}</span>
    </button>
    {#if unread > 0}
        <span class="unread-badge" aria-hidden="true">{unread}</span>
        <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
    {/if}
    <button
        class="feed-health"
        type="button"
        aria-label={t('feedHealth.open', { name: label })}
        onclick={() => onhealth(feed.id)}
    >
        <span class="health-dot health-{feed.status}" aria-hidden="true"></span>
    </button>
</div>

<style>
    .feed-item {
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-1);
        padding-right: var(--space-1);
        border-radius: var(--radius-md);
        transition: background var(--dur-fast) var(--ease);
    }

    .feed-item:hover {
        background: var(--hover);
    }

    .feed-item.active {
        background: var(--selected);
    }

    .feed-item.active::before {
        content: '';
        position: absolute;
        left: 0;
        top: 8px;
        bottom: 8px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
    }

    .feed-select {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-2);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
    }

    .feed-item.active .feed-select {
        color: var(--fg);
    }

    .feed-mono {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 20px;
        height: 20px;
        border-radius: var(--radius-sm);
        font-size: 0.68rem;
        font-weight: 700;
        color: hsl(var(--mono-hue) 60% 80%);
        background: hsl(var(--mono-hue) 45% 34% / 0.5);
        box-shadow: inset 0 0 0 1px hsl(var(--mono-hue) 50% 58% / 0.3);
    }

    .feed-title {
        flex: 1 1 auto;
        min-width: 0;
    }

    .feed-item.unhealthy .feed-title {
        color: var(--fg-subtle);
    }

    .unread-badge {
        flex: 0 0 auto;
        min-width: 1.35rem;
        padding: 1px var(--space-1);
        border-radius: var(--radius-pill);
        background: var(--surface-raised);
        color: var(--fg-muted);
        font-size: 0.7rem;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        text-align: center;
        line-height: 1.5;
    }

    .feed-item.active .unread-badge {
        background: color-mix(in srgb, var(--accent), transparent 15%);
        color: var(--accent-fg);
    }

    .feed-health {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 1.25rem;
        height: 1.25rem;
        border-radius: var(--radius-sm);
        background: transparent;
        opacity: 0;
        transition: opacity var(--dur-fast) var(--ease);
    }

    /* Healthy feeds hide the dot until hover; unhealthy ones always show it. */
    .feed-item:hover .feed-health,
    .feed-item.unhealthy .feed-health {
        opacity: 1;
    }

    .feed-health:hover {
        background: var(--active);
    }

    .health-dot {
        width: 0.5rem;
        height: 0.5rem;
        border-radius: 50%;
        background: var(--fg-subtle);
    }

    .health-active {
        background: var(--success);
    }

    .health-paused {
        background: var(--warning);
    }

    .health-dead {
        background: var(--error);
    }
</style>
