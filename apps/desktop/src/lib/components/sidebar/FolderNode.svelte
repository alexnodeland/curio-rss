<script lang="ts">
/**
 * One node of the sidebar folder tree: a collapsible header (disclosure
 * chevron, folder name, roll-up unread count) over its subfolders (rendered
 * recursively) and the feeds that live directly in it. Folders are a *view*
 * of `/`-path tags, so the header only expands/collapses — there is no
 * backend folder-scoped article filter; selecting a feed still scopes the
 * list through the same path as the flat sidebar.
 */
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import type { FeedFolder } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import FeedItem from './FeedItem.svelte';
import FolderNode from './FolderNode.svelte';

let { folder, depth = 0 }: { folder: FeedFolder; depth?: number } = $props();

const collapsed = $derived(feedsStore.isFolderCollapsed(folder.path));
const unread = $derived(feedsStore.folderUnread(folder));
</script>

<li class="folder">
    <button
        class="folder-header"
        type="button"
        aria-expanded={!collapsed}
        style:--depth={depth}
        onclick={() => feedsStore.toggleFolder(folder.path)}
    >
        <span class="chevron" class:open={!collapsed}><Icon name="chevron" size={14} /></span>
        <span class="folder-name truncate">{folder.name}</span>
        {#if unread > 0}
            <span class="folder-count" aria-hidden="true">{unread}</span>
            <span class="sr-only">{t('shell.unread.count', { count: unread })}</span>
        {/if}
    </button>
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
        display: flex;
        align-items: center;
        gap: var(--space-1);
        width: 100%;
        padding: var(--space-2) var(--space-2);
        padding-left: calc(var(--space-2) + var(--depth) * var(--space-4));
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        text-align: left;
        font-size: var(--text-md);
        font-weight: 500;
        transition: background var(--dur-fast) var(--ease);
    }

    .folder-header:hover {
        background: var(--hover);
        color: var(--fg);
    }

    .chevron {
        flex: 0 0 auto;
        display: inline-flex;
        color: var(--fg-subtle);
        transition: transform var(--dur-fast) var(--ease);
    }

    .chevron.open {
        transform: rotate(90deg);
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
