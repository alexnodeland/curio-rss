<script lang="ts">
/**
 * One node of the sidebar folder tree. The header has two targets: a
 * disclosure chevron that expands/collapses, and the folder name itself,
 * which **scopes the article list to the folder** (every article whose feed
 * carries this `/`-path tag or one nested beneath it — backend-owned via the
 * `feed_tag` filter). Below the header sit the subfolders (recursive) and the
 * feeds that live directly in this folder.
 */
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import { selectFolder } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import type { FeedFolder } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import FeedItem from './FeedItem.svelte';
import FolderNode from './FolderNode.svelte';

let { folder, depth = 0 }: { folder: FeedFolder; depth?: number } = $props();

const collapsed = $derived(feedsStore.isFolderCollapsed(folder.path));
const unread = $derived(feedsStore.folderUnread(folder));
const selected = $derived(
    selectionStore.selectedFeedId === null && articlesStore.filters.feedTag === folder.path,
);
</script>

<li class="folder">
    <div class="folder-header" style:--depth={depth}>
        <button
            class="folder-disclosure"
            type="button"
            aria-expanded={!collapsed}
            aria-label={t('folder.toggle', { name: folder.name })}
            onclick={() => feedsStore.toggleFolder(folder.path)}
        >
            <span class="chevron" class:open={!collapsed}><Icon name="chevron" size={14} /></span>
        </button>
        <button
            class="folder-select"
            class:active={selected}
            aria-current={selected ? 'true' : undefined}
            onclick={() => selectFolder(folder.path)}
        >
            <span class="folder-name truncate">{folder.name}</span>
            {#if unread > 0}
                <span class="folder-count" aria-hidden="true">{unread}</span>
                <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
            {/if}
        </button>
    </div>
    {#if !collapsed}
        <ul class="folder-children">
            {#each folder.subfolders as subfolder (subfolder.path)}
                <FolderNode folder={subfolder} depth={depth + 1} />
            {/each}
            {#each folder.feeds as feed (feed.id)}
                <li class="folder-feed" style:--depth={depth + 1}>
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
</li>

<style>
    .folder {
        list-style: none;
    }

    .folder-header {
        position: relative;
        display: flex;
        align-items: center;
        gap: 2px;
        padding-left: calc(var(--depth) * var(--space-4));
    }

    .folder-disclosure {
        flex: 0 0 auto;
        display: grid;
        place-items: center;
        width: 22px;
        height: 28px;
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--fg-subtle);
    }

    .folder-disclosure:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .chevron {
        display: inline-flex;
        transition: transform var(--dur-fast) var(--ease);
    }

    .chevron.open {
        transform: rotate(90deg);
    }

    .folder-select {
        position: relative;
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-2);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
        font-weight: 500;
        transition: background var(--dur-fast) var(--ease);
    }

    .folder-select:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .folder-select.active {
        background: var(--selected);
        color: var(--fg);
    }

    .folder-select.active::before {
        content: '';
        position: absolute;
        left: calc(-1 * var(--space-1));
        top: 8px;
        bottom: 8px;
        width: 3px;
        border-radius: var(--radius-pill);
        background: var(--accent);
    }

    .folder-name {
        flex: 1 1 auto;
        min-width: 0;
    }

    .folder-count {
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

    .folder-select.active .folder-count {
        background: color-mix(in srgb, var(--accent), transparent 15%);
        color: var(--accent-fg);
    }

    .folder-children {
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: 1px;
    }

    .folder-feed {
        padding-left: calc(var(--depth) * var(--space-4));
    }
</style>
