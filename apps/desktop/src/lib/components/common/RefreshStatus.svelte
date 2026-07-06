<script lang="ts">
/**
 * Refresh progress + screen-reader announcements. A polite live region
 * announces the start of a sweep and its completion summary (new articles
 * across N feeds); a small visible indicator mirrors it for sighted users.
 * Reads the feeds store's event-driven refresh fields — no polling.
 */
import { t } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';

const completed = $derived(feedsStore.refreshOutcomes.length);
const newArticles = $derived(
    feedsStore.refreshOutcomes.reduce((sum, outcome) => sum + outcome.new_articles, 0),
);

const message = $derived(
    feedsStore.refreshing
        ? t('refresh.announce.start')
        : completed > 0
          ? t('refresh.announce.done', { count: newArticles, feeds: completed })
          : '',
);
</script>

<div class="refresh-status" role="status" aria-live="polite">
    {#if message !== ''}
        {#if feedsStore.refreshing}
            <span class="spinner" aria-hidden="true"></span>
        {/if}
        <span class="message">{message}</span>
    {/if}
</div>

<style>
    .refresh-status {
        position: fixed;
        left: var(--space-4);
        bottom: var(--space-4);
        display: flex;
        align-items: center;
        gap: var(--space-2);
        z-index: 60;
        pointer-events: none;
    }

    .message {
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-xl);
        background: var(--bg-tertiary);
        color: var(--fg-muted);
        font-size: 0.75rem;
        box-shadow: var(--shadow-sm);
    }

    .spinner {
        width: 0.75rem;
        height: 0.75rem;
        border: 2px solid var(--border);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @media (prefers-reduced-motion: reduce) {
        .spinner {
            animation: none;
        }
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }
</style>
