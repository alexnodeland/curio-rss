<script lang="ts">
/**
 * The middle column: a header (search + destinations entry) over either the
 * filter list or the FTS results, chosen by whether a search is active. The
 * reader reacts to selection regardless of which list is showing.
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import SearchResults from '$components/articles/SearchResults.svelte';
import SearchBar from '$components/common/SearchBar.svelte';
import { t } from '$lib/i18n';
import { searchStore } from '$lib/state/search.svelte';
import { uiStore } from '$lib/state/ui.svelte';
</script>

<div class="list-pane">
    <div class="list-header">
        <div class="search-slot">
            <SearchBar />
        </div>
        <button
            class="destinations-button"
            type="button"
            onclick={() => uiStore.openModal('destinations')}
        >
            {t('destinations.open')}
        </button>
    </div>
    {#if searchStore.active}
        <SearchResults />
    {:else}
        <ArticleList />
    {/if}
</div>

<style>
    .list-pane {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--bg);
    }

    .list-header {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-3);
        border-bottom: 1px solid var(--border-subtle);
        background: var(--bg-secondary);
    }

    .search-slot {
        flex: 1 1 auto;
        min-width: 0;
    }

    .destinations-button {
        flex: 0 0 auto;
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--border-subtle);
        font-size: 0.75rem;
        white-space: nowrap;
    }

    .destinations-button:hover {
        background: var(--bg-hover);
        color: var(--fg);
    }
</style>
