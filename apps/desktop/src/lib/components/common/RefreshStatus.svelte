<script lang="ts">
/**
 * Refresh progress + screen-reader announcements. A polite live region
 * announces the start of a sweep, a stop request, and its completion summary
 * (new articles across N feeds, or how far a stopped sweep got). A small
 * visible pill mirrors it for sighted users with the sweep's running count
 * ("Refreshing… 12 of 300"), a thin progress bar, and a Stop button that
 * cancels the sweep. Reads the feeds store's event-driven refresh fields —
 * no polling.
 */
import { tooltip } from '$lib/actions/tooltip';
import { t } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';

/** How long the completion summary lingers before the pill self-dismisses. */
const REFRESH_DISMISS_MS = 5000;

const completed = $derived(feedsStore.refreshOutcomes.length);
const newArticles = $derived(
    feedsStore.refreshOutcomes.reduce((sum, outcome) => sum + outcome.new_articles, 0),
);
const progress = $derived(feedsStore.refreshing ? feedsStore.refreshProgress : null);

// The completion summary used to linger forever (refreshOutcomes persists).
// A fresh sweep revives the pill; its summary then auto-dismisses.
let dismissed = $state(false);
$effect(() => {
    if (feedsStore.refreshing) {
        dismissed = false;
        return;
    }
    if ((completed === 0 && !feedsStore.refreshCancelled) || dismissed) {
        return;
    }
    const timer = setTimeout(() => {
        dismissed = true;
    }, REFRESH_DISMISS_MS);
    return () => clearTimeout(timer);
});

/** The summary of a finished sweep, while it lingers. */
const summary = $derived(
    feedsStore.refreshing || dismissed || (completed === 0 && !feedsStore.refreshCancelled)
        ? ''
        : t(feedsStore.refreshCancelled ? 'refresh.announce.cancelled' : 'refresh.announce.done', {
              count: newArticles,
              feeds: completed,
          }),
);

/** What the live region announces: the start, a stop, the summary — never each feed. */
const announcement = $derived(
    feedsStore.refreshing
        ? feedsStore.cancellingRefresh
            ? t('refresh.cancelling')
            : t('refresh.announce.start')
        : summary,
);

/** The pill's text: the running count once the first feed is in. */
const label = $derived(
    !feedsStore.refreshing
        ? summary
        : feedsStore.cancellingRefresh
          ? t('refresh.cancelling')
          : progress !== null
            ? t('refresh.inProgress', { done: progress.done, total: progress.total })
            : t('refresh.announce.start'),
);

const fraction = $derived(
    progress !== null && progress.total > 0 ? Math.min(progress.done / progress.total, 1) : 0,
);

function stop(): void {
    void feedsStore.cancelRefresh();
}
</script>

<div class="refresh-status">
    <span class="sr-only" role="status" aria-live="polite">{announcement}</span>
    {#if label !== ''}
        {#if feedsStore.refreshing}
            <span class="spinner" aria-hidden="true"></span>
        {/if}
        <div class="pill" class:running={feedsStore.refreshing}>
            <span class="message">{label}</span>
            {#if feedsStore.refreshing}
                <button
                    class="stop"
                    type="button"
                    aria-label={t('refresh.cancel.label')}
                    use:tooltip={t('refresh.cancel.label')}
                    disabled={feedsStore.cancellingRefresh}
                    onclick={stop}>{t('refresh.cancel')}</button
                >
                <span
                    class="bar"
                    aria-hidden="true"
                    style:transform={`scaleX(${fraction})`}
                ></span>
            {/if}
        </div>
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

    .pill {
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-pill);
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        box-shadow: var(--shadow-md);
        overflow: hidden;
    }

    .pill.running {
        padding-right: var(--space-1);
    }

    .message {
        color: var(--fg-muted);
        font-size: var(--text-xs);
        font-variant-numeric: tabular-nums;
        white-space: nowrap;
    }

    .stop {
        pointer-events: auto;
        padding: 0 var(--space-2);
        border: none;
        border-left: 1px solid var(--hairline);
        border-radius: 0 var(--radius-pill) var(--radius-pill) 0;
        background: transparent;
        color: var(--fg-muted);
        font: inherit;
        font-size: var(--text-xs);
        line-height: inherit;
        cursor: pointer;
    }

    .stop:hover:not(:disabled) {
        color: var(--fg);
    }

    .stop:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: 1px;
    }

    .stop:disabled {
        cursor: default;
        opacity: 0.5;
    }

    .bar {
        /* Inset to the pill's straight run, clear of its rounded ends. */
        position: absolute;
        left: var(--space-3);
        right: var(--space-3);
        bottom: 0;
        height: 2px;
        border-radius: 1px;
        background: var(--accent);
        transform-origin: left center;
        transition: transform 0.2s ease-out;
    }

    .spinner {
        width: 0.75rem;
        height: 0.75rem;
        border: 2px solid var(--hairline-strong);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: spin 0.8s linear infinite;
    }

    @media (prefers-reduced-motion: reduce) {
        .spinner {
            animation: none;
        }

        .bar {
            transition: none;
        }
    }

    @keyframes spin {
        to {
            transform: rotate(360deg);
        }
    }
</style>
