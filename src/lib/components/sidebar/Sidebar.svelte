<script lang="ts">
import {
    selectFeed,
    clearFilter,
    setFilter,
    refreshAllFeeds,
    isRefreshing,
    selectedFeedId,
    selectedFolderId,
    totalUnreadCount,
    folderTree,
} from '$lib/stores/feeds';
import { sidebarCollapsed, toggleSidebar, openModal } from '$lib/stores/ui';
import FolderItem from './FolderItem.svelte';

function handleAllItems() {
    clearFilter();
    selectFeed(null);
}

function handleUnread() {
    setFilter({ unread_only: true });
    selectFeed(null);
}

function handleStarred() {
    setFilter({ starred_only: true });
    selectFeed(null);
}

function handleReadLater() {
    setFilter({ read_later_only: true });
    selectFeed(null);
}
</script>

<aside class="sidebar" class:collapsed={$sidebarCollapsed}>
    <header class="sidebar-header">
        <h1 class="logo">Curio</h1>
        <div class="header-actions">
            <button on:click={() => refreshAllFeeds()} disabled={$isRefreshing} title="Refresh all feeds">
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M21 12a9 9 0 11-6.219-8.56"/>
                    <path d="M21 3v9h-9"/>
                </svg>
            </button>
            <button on:click={toggleSidebar} title="Toggle sidebar">
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <rect x="3" y="3" width="18" height="18" rx="2"/>
                    <line x1="9" y1="3" x2="9" y2="21"/>
                </svg>
            </button>
        </div>
    </header>

    <nav class="sidebar-nav">
        <section class="quick-access">
            <button class="nav-item" class:active={!$selectedFeedId && !$selectedFolderId} on:click={handleAllItems}>
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M21 15a2 2 0 01-2 2H7l-4 4V5a2 2 0 012-2h14a2 2 0 012 2z"/>
                </svg>
                <span>All Items</span>
                {#if $totalUnreadCount > 0}
                    <span class="badge">{$totalUnreadCount}</span>
                {/if}
            </button>

            <button class="nav-item" on:click={handleUnread}>
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="12" cy="12" r="10"/>
                </svg>
                <span>Unread</span>
            </button>

            <button class="nav-item" on:click={handleStarred}>
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"/>
                </svg>
                <span>Starred</span>
            </button>

            <button class="nav-item" on:click={handleReadLater}>
                <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M19 21l-7-5-7 5V5a2 2 0 012-2h10a2 2 0 012 2z"/>
                </svg>
                <span>Read Later</span>
            </button>
        </section>

        <section class="folders">
            <h2 class="section-title">Feeds</h2>
            {#each $folderTree as node}
                <FolderItem {node} />
            {/each}
        </section>
    </nav>

    <footer class="sidebar-footer">
        <button class="add-feed-btn" on:click={() => openModal('add_feed')}>
            <svg xmlns="http://www.w3.org/2000/svg" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <line x1="12" y1="5" x2="12" y2="19"/>
                <line x1="5" y1="12" x2="19" y2="12"/>
            </svg>
            Add Feed
        </button>
    </footer>
</aside>

<style>
    .sidebar {
        width: var(--sidebar-width);
        height: 100vh;
        background: var(--bg-secondary);
        border-right: 1px solid var(--border);
        display: flex;
        flex-direction: column;
        overflow: hidden;
        transition: width 0.2s ease;
    }

    .sidebar.collapsed {
        width: 0;
        border-right: none;
    }

    .sidebar-header {
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: var(--space-3) var(--space-4);
        border-bottom: 1px solid var(--border);
    }

    .logo {
        font-size: 18px;
        font-weight: 600;
        color: var(--accent);
    }

    .header-actions {
        display: flex;
        gap: var(--space-2);
    }

    .header-actions button {
        padding: var(--space-1);
        color: var(--fg-muted);
        border-radius: var(--radius-sm);
    }

    .header-actions button:hover {
        background: var(--bg-tertiary);
        color: var(--fg);
    }

    .sidebar-nav {
        flex: 1;
        overflow-y: auto;
        padding: var(--space-2);
    }

    .quick-access {
        display: flex;
        flex-direction: column;
        gap: var(--space-1);
        margin-bottom: var(--space-4);
    }

    .nav-item {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: var(--space-2) var(--space-3);
        width: 100%;
        text-align: left;
        color: var(--fg);
        border-radius: var(--radius-md);
        font-size: 14px;
    }

    .nav-item:hover {
        background: var(--bg-tertiary);
    }

    .nav-item.active {
        background: var(--accent);
        color: var(--accent-fg);
    }

    .nav-item svg {
        flex-shrink: 0;
    }

    .nav-item span {
        flex: 1;
    }

    .badge {
        background: var(--accent);
        color: var(--accent-fg);
        font-size: 12px;
        padding: 2px 6px;
        border-radius: 10px;
        min-width: 20px;
        text-align: center;
    }

    .nav-item.active .badge {
        background: var(--accent-fg);
        color: var(--accent);
    }

    .section-title {
        font-size: 11px;
        font-weight: 600;
        text-transform: uppercase;
        color: var(--fg-subtle);
        padding: var(--space-2) var(--space-3);
        margin-bottom: var(--space-1);
    }

    .folders {
        display: flex;
        flex-direction: column;
    }

    .sidebar-footer {
        padding: var(--space-3) var(--space-4);
        border-top: 1px solid var(--border);
    }

    .add-feed-btn {
        display: flex;
        align-items: center;
        justify-content: center;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-3);
        background: var(--accent);
        color: var(--accent-fg);
        border-radius: var(--radius-md);
        font-size: 14px;
        font-weight: 500;
    }

    .add-feed-btn:hover {
        background: var(--accent-hover);
    }
</style>
