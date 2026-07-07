<script lang="ts">
/**
 * The left pane: a brand/action header, the built-in views (all / starred /
 * read later), and the subscription list, with backend-owned unread counts
 * (`get_unread_counts` — never client badge math). Clicks re-scope the
 * backend-owned filters; the article list reacts through the stores.
 */
import Icon, { type IconName } from '$components/common/Icon.svelte';
import { type MessageKey, t } from '$lib/i18n';
import { activeView, refreshAll, selectView, type ViewId } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { buildFeedTree } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import FeedItem from './FeedItem.svelte';
import FolderNode from './FolderNode.svelte';

const VIEWS: readonly { id: ViewId; label: MessageKey; icon: IconName }[] = [
    { id: 'all', label: 'view.all', icon: 'inbox' },
    { id: 'starred', label: 'view.starred', icon: 'star' },
    { id: 'readLater', label: 'view.readLater', icon: 'bookmark' },
    { id: 'archived', label: 'view.archived', icon: 'archive' },
];

// Prime the feed + unread-count queries during init, NOT from the template:
// state created inside the render reaction is excluded from that reaction's
// dependencies, so a fetch kicked off by a template read would never
// re-render on completion (the error/loaded transition would be lost).
feedsStore.prime();

// The folder tree derived from each feed's `/`-path tags; feeds with no tags
// fall into `ungrouped` and render flat below the folders.
const feedTree = $derived(buildFeedTree(feedsStore.feeds));

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
    <div class="sidebar-brand">
        <div class="brand">
            <span class="brand-mark" aria-hidden="true"><Icon name="sparkle" size={16} /></span>
            <span class="brand-name">Curio</span>
        </div>
        <div class="sidebar-toolbar" role="toolbar" aria-label={t('sidebar.label')}>
            <button
                class="chrome-button"
                type="button"
                aria-label={t('toolbar.addFeed')}
                title={t('toolbar.addFeed')}
                onclick={() => uiStore.openModal('add-feed')}><Icon name="plus" /></button
            >
            <button
                class="chrome-button"
                type="button"
                aria-label={t('toolbar.refreshAll')}
                title={t('toolbar.refreshAll')}
                aria-busy={feedsStore.refreshing}
                disabled={feedsStore.refreshing}
                onclick={() => refreshAll()}><Icon name="refresh" /></button
            >
            <button
                class="chrome-button"
                type="button"
                aria-label={t('toolbar.settings')}
                title={t('toolbar.settings')}
                onclick={() => uiStore.openModal('settings')}><Icon name="settings" /></button
            >
        </div>
    </div>

    <div class="sidebar-scroll">
        <section class="sidebar-group">
            <ul class="sidebar-list">
                {#each VIEWS as view (view.id)}
                    <li>
                        <button
                            class="view-item"
                            class:active={currentView() === view.id}
                            aria-current={currentView() === view.id ? 'true' : undefined}
                            onclick={() => selectView(view.id)}
                        >
                            <Icon name={view.icon} size={17} />
                            <span class="truncate">{t(view.label)}</span>
                            {#if view.id === 'all' && feedsStore.unreadTotal > 0}
                                <span class="view-count" aria-hidden="true"
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
                    {#each feedTree.folders as folder (folder.path)}
                        <FolderNode {folder} depth={0} />
                    {/each}
                    {#each feedTree.ungrouped as feed (feed.id)}
                        <li>
                            <FeedItem
                                {feed}
                                unread={feedsStore.unreadFor(feed.id)}
                                selected={selectionStore.selectedFeedId === feed.id}
                                siblings={feedTree.ungrouped.map((candidate) => candidate.id)}
                                onselect={(feedId) => selectionStore.selectFeed(feedId)}
                                onhealth={(feedId) => uiStore.openHealth(feedId)}
                            />
                        </li>
                    {/each}
                </ul>
            {/if}
        </section>
    </div>
</nav>

<style>
    .sidebar {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--surface-chrome);
        border-right: 1px solid var(--hairline);
    }

    .sidebar-brand {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-2);
        height: var(--header-height);
        padding: 0 var(--space-2) 0 var(--space-4);
        border-bottom: 1px solid var(--hairline);
    }

    .brand {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        min-width: 0;
    }

    .brand-mark {
        display: grid;
        place-items: center;
        width: 26px;
        height: 26px;
        border-radius: var(--radius-md);
        color: var(--accent);
        background: var(--selected);
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 72%);
    }

    .brand-name {
        font-size: var(--text-lg);
        font-weight: 660;
        letter-spacing: var(--tracking-tight);
        color: var(--fg);
    }

    .sidebar-toolbar {
        display: flex;
        align-items: center;
        gap: 2px;
    }

    .chrome-button {
        display: inline-grid;
        place-items: center;
        width: 30px;
        height: 30px;
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .chrome-button:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .sidebar-scroll {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        padding: var(--space-3) var(--space-2) var(--space-4);
        display: flex;
        flex-direction: column;
        gap: var(--space-5);
    }

    .sidebar-group {
        display: flex;
        flex-direction: column;
        gap: 2px;
    }

    .sidebar-heading {
        padding: 0 var(--space-3);
        margin-bottom: var(--space-1);
        font-size: 0.6875rem;
        font-weight: 650;
        letter-spacing: var(--tracking-caps);
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
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

    .sidebar-status.error {
        color: var(--error);
    }

    .view-item {
        position: relative;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-2);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
        font-weight: 500;
        transition: background var(--dur-fast) var(--ease);
    }

    .view-item :global(svg) {
        flex: 0 0 auto;
        color: var(--fg-subtle);
    }

    .view-item:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .view-item.active {
        background: var(--selected);
        color: var(--fg);
    }

    .view-item.active :global(svg) {
        color: var(--accent);
    }

    .view-item.active::before {
        content: '';
        position: absolute;
        left: 0;
        top: 8px;
        bottom: 8px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
    }

    .view-count {
        margin-left: auto;
        min-width: 1.35rem;
        padding: 1px var(--space-1);
        border-radius: var(--radius-pill);
        background: var(--accent);
        color: var(--accent-fg);
        font-size: 0.7rem;
        font-weight: 600;
        font-variant-numeric: tabular-nums;
        text-align: center;
        line-height: 1.5;
    }
</style>
