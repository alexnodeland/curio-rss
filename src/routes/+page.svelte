<script lang="ts">
import { refreshAllFeeds } from '$lib/stores/feeds';
import { closeModal, openModal, sidebarCollapsed } from '$lib/stores/ui';

function handleKeydown(e: KeyboardEvent) {
    // Global keyboard shortcuts
    if (e.key === '?' && !e.ctrlKey && !e.metaKey) {
        // Show keyboard shortcuts
    } else if (e.key === 't' && !e.ctrlKey && !e.metaKey) {
        sidebarCollapsed.update((v) => !v);
    } else if (e.key === 'r' && e.shiftKey) {
        refreshAllFeeds();
    } else if (e.key === 'Escape') {
        closeModal();
    } else if (e.key === 'a' && e.ctrlKey) {
        e.preventDefault();
        openModal('add_feed');
    }
}
</script>

<svelte:window on:keydown={handleKeydown} />

<div class="app" class:sidebar-collapsed={$sidebarCollapsed}>
    <Sidebar />

    <main class="main-content">
        <ArticleList />
        {#if $selectedArticle}
            <ReaderPane article={$selectedArticle} />
        {:else}
            <div class="empty-state">
                <p>Select an article to read</p>
            </div>
        {/if}
    </main>

    {#if $isRefreshing}
        <div class="refresh-indicator">Refreshing feeds...</div>
    {/if}
</div>

{#if $activeModal === 'add_feed'}
    <AddFeedModal on:close={closeModal} />
{/if}

<style>
    .app {
        display: flex;
        height: 100vh;
        overflow: hidden;
    }

    .main-content {
        flex: 1;
        display: flex;
        overflow: hidden;
    }

    .empty-state {
        flex: 1;
        display: flex;
        align-items: center;
        justify-content: center;
        color: var(--fg-muted);
        background: var(--bg);
    }

    .refresh-indicator {
        position: fixed;
        bottom: var(--space-4);
        right: var(--space-4);
        background: var(--accent);
        color: var(--accent-fg);
        padding: var(--space-2) var(--space-4);
        border-radius: var(--radius-md);
        font-size: 14px;
        box-shadow: var(--shadow-md);
    }

    .sidebar-collapsed .main-content {
        margin-left: 0;
    }
</style>
