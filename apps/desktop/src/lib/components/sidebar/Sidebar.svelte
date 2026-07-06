<script lang="ts">
/**
 * The left pane: built-in views (all / starred / read later) and the
 * subscription list, with backend-owned unread counts (`get_unread_counts`
 * — never client badge math). Clicks re-scope the backend-owned filters;
 * the article list reacts through the stores.
 */
import { type MessageKey, t } from '$lib/i18n';
import { activeView, refreshAll, selectView, type ViewId } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import FeedItem from './FeedItem.svelte';

const VIEWS: readonly { id: ViewId; label: MessageKey }[] = [
    { id: 'all', label: 'view.all' },
    { id: 'starred', label: 'view.starred' },
    { id: 'readLater', label: 'view.readLater' },
];

// Prime the feed + unread-count queries during init, NOT from the template:
// state created inside the render reaction is excluded from that reaction's
// dependencies, so a fetch kicked off by a template read would never
// re-render on completion (the error/loaded transition would be lost).
feedsStore.prime();

function currentView(): ViewId | null {
    if (selectionStore.selectedFeedId !== null) {
        return null;
    }
    return activeView(articlesStore.filters);
}

function feedsError(): string {
    const failure = feedsStore.error;
    return failure === null ? '' : commandErrorMessage(failure);
}
</script>

<nav class="sidebar" aria-label={t('sidebar.label')}>
    <div class="sidebar-toolbar" role="toolbar" aria-label={t('sidebar.label')}>
        <button
            class="toolbar-button"
            type="button"
            aria-label={t('toolbar.addFeed')}
            title={t('toolbar.addFeed')}
            onclick={() => uiStore.openModal('add-feed')}>＋</button
        >
        <button
            class="toolbar-button"
            type="button"
            aria-label={t('toolbar.refreshAll')}
            title={t('toolbar.refreshAll')}
            aria-busy={feedsStore.refreshing}
            disabled={feedsStore.refreshing}
            onclick={() => refreshAll()}>⟳</button
        >
        <button
            class="toolbar-button"
            type="button"
            aria-label={t('toolbar.settings')}
            title={t('toolbar.settings')}
            onclick={() => uiStore.openModal('settings')}>⚙</button
        >
    </div>

    <section class="sidebar-group">
        <h2 class="sidebar-heading">{t('sidebar.views')}</h2>
        <ul class="sidebar-list">
            {#each VIEWS as view (view.id)}
                <li>
                    <button
                        class="view-item"
                        class:active={currentView() === view.id}
                        aria-current={currentView() === view.id ? 'true' : undefined}
                        onclick={() => selectView(view.id)}
                    >
                        <span class="truncate">{t(view.label)}</span>
                        {#if view.id === 'all' && feedsStore.unreadTotal > 0}
                            <span class="unread-badge" aria-hidden="true"
                                >{feedsStore.unreadTotal}</span
                            >
                            <span class="sr-only"
                                >{t('shell.unread.count', { count: feedsStore.unreadTotal })}</span
                            >
                        {/if}
                    </button>
                </li>
            {/each}
        </ul>
    </section>

    <section class="sidebar-group sidebar-feeds">
        <h2 class="sidebar-heading">{t('sidebar.feeds')}</h2>
        {#if feedsStore.error !== null}
            <p class="sidebar-status error" role="alert">{feedsError()}</p>
        {:else if !feedsStore.loaded}
            <p class="sidebar-status">{t('shell.feeds.loading')}</p>
        {:else if feedsStore.feeds.length === 0}
            <p class="sidebar-status">{t('shell.feeds.empty')}</p>
        {:else}
            <ul class="sidebar-list">
                {#each feedsStore.feeds as feed (feed.id)}
                    <li>
                        <FeedItem
                            {feed}
                            unread={feedsStore.unreadFor(feed.id)}
                            selected={selectionStore.selectedFeedId === feed.id}
                            onselect={(feedId) => selectionStore.selectFeed(feedId)}
                            onhealth={(feedId) => uiStore.openHealth(feedId)}
                        />
                    </li>
                {/each}
            </ul>
        {/if}
    </section>
</nav>

<style>
    .sidebar {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        gap: var(--space-4);
        min-height: 0;
        padding: var(--space-3);
        overflow-y: auto;
        background: var(--bg-secondary);
    }

    .sidebar-toolbar {
        display: flex;
        gap: var(--space-1);
        padding: 0 var(--space-2) var(--space-2);
        border-bottom: 1px solid var(--border-subtle);
    }

    .toolbar-button {
        display: flex;
        align-items: center;
        justify-content: center;
        width: 2rem;
        height: 2rem;
        border-radius: var(--radius-md);
        color: var(--fg-muted);
        background: transparent;
        font-size: 1rem;
        line-height: 1;
    }

    .toolbar-button:hover:not(:disabled) {
        background: var(--bg-hover);
        color: var(--fg);
    }

    .sidebar-group {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
    }

    .sidebar-heading {
        padding: 0 var(--space-3);
        font-size: 0.6875rem;
        font-weight: 600;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--fg-subtle);
    }

    .sidebar-list {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .sidebar-status {
        padding: var(--space-2) var(--space-3);
        font-size: 0.8125rem;
        color: var(--fg-muted);
    }

    .sidebar-status.error {
        color: var(--error);
    }

    .view-item {
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

    .view-item:hover {
        background: var(--bg-hover);
    }

    .view-item.active {
        background: var(--accent-muted);
        color: var(--accent);
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

    .view-item.active .unread-badge {
        background: var(--accent);
        color: var(--accent-fg);
    }
</style>
