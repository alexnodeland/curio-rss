<script lang="ts">
    import type { FolderNode } from '$lib/types';
    import { selectedFeedId, selectFeed, unreadCounts } from '$lib/stores/feeds';

    export let node: FolderNode;

    let expanded = true;

    function toggleExpand() {
        expanded = !expanded;
    }

    function getTotalUnread(node: FolderNode): number {
        let count = node.unread_count;
        for (const feed of node.feeds) {
            count += $unreadCounts.get(feed.id) ?? 0;
        }
        for (const child of node.children) {
            count += getTotalUnread(child);
        }
        return count;
    }

    $: totalUnread = getTotalUnread(node);
</script>

<div class="folder-item">
    {#if node.children.length > 0 || node.feeds.length > 0}
        <button class="folder-header" on:click={toggleExpand}>
            <svg
                class="chevron"
                class:expanded
                xmlns="http://www.w3.org/2000/svg"
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
            >
                <polyline points="9 18 15 12 9 6" />
            </svg>
            <span class="folder-name">{node.folder.name}</span>
            {#if totalUnread > 0}
                <span class="badge">{totalUnread}</span>
            {/if}
        </button>
    {/if}

    {#if expanded}
        <div class="folder-content">
            {#each node.children as child}
                <svelte:self node={child} />
            {/each}

            {#each node.feeds as feed}
                <button
                    class="feed-item"
                    class:active={$selectedFeedId === feed.id}
                    on:click={() => selectFeed(feed.id)}
                >
                    {#if feed.icon_url}
                        <img src={feed.icon_url} alt="" class="feed-icon" />
                    {:else}
                        <svg
                            class="feed-icon-default"
                            xmlns="http://www.w3.org/2000/svg"
                            width="16"
                            height="16"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            stroke-width="2"
                            stroke-linecap="round"
                            stroke-linejoin="round"
                        >
                            <path d="M4 11a9 9 0 019 9" />
                            <path d="M4 4a16 16 0 0116 16" />
                            <circle cx="5" cy="19" r="1" />
                        </svg>
                    {/if}
                    <span class="feed-title">{feed.title}</span>
                    {#if ($unreadCounts.get(feed.id) ?? 0) > 0}
                        <span class="badge">{$unreadCounts.get(feed.id)}</span>
                    {/if}
                </button>
            {/each}
        </div>
    {/if}
</div>

<style>
    .folder-item {
        margin-bottom: var(--space-1);
    }

    .folder-header {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-3);
        color: var(--fg);
        border-radius: var(--radius-md);
        font-size: 14px;
        font-weight: 500;
    }

    .folder-header:hover {
        background: var(--bg-tertiary);
    }

    .chevron {
        flex-shrink: 0;
        transition: transform 0.2s ease;
    }

    .chevron.expanded {
        transform: rotate(90deg);
    }

    .folder-name {
        flex: 1;
        text-align: left;
    }

    .folder-content {
        padding-left: var(--space-4);
    }

    .feed-item {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-3);
        color: var(--fg-muted);
        border-radius: var(--radius-md);
        font-size: 13px;
    }

    .feed-item:hover {
        background: var(--bg-tertiary);
        color: var(--fg);
    }

    .feed-item.active {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .feed-icon {
        width: 16px;
        height: 16px;
        border-radius: var(--radius-sm);
        object-fit: cover;
    }

    .feed-icon-default {
        flex-shrink: 0;
    }

    .feed-title {
        flex: 1;
        text-align: left;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
    }

    .badge {
        background: var(--accent);
        color: var(--accent-fg);
        font-size: 11px;
        padding: 1px 5px;
        border-radius: 8px;
        min-width: 18px;
        text-align: center;
    }

    .feed-item.active .badge {
        background: var(--accent-fg);
        color: var(--accent);
    }
</style>
