<script lang="ts">
/**
 * The middle column: a header (search + destinations entry) over either the
 * filter list or the FTS results, chosen by whether a search is active. The
 * reader reacts to selection regardless of which list is showing.
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import SearchResults from '$components/articles/SearchResults.svelte';
import Icon from '$components/common/Icon.svelte';
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
            title={t('destinations.open')}
            aria-label={t('destinations.open')}
            onclick={() => uiStore.openModal('destinations')}
        >
            <Icon name="folder" />
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
        background: var(--surface-list);
        border-right: 1px solid var(--hairline);
    }

    .list-header {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        gap: var(--space-2);
        height: var(--header-height);
        padding: 0 var(--space-3);
        border-bottom: 1px solid var(--hairline);
    }

    .search-slot {
        flex: 1 1 auto;
        min-width: 0;
    }

    .destinations-button {
        flex: 0 0 auto;
        display: inline-grid;
        place-items: center;
        width: 34px;
        height: 34px;
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid transparent;
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .destinations-button:hover {
        background: var(--hover);
        color: var(--fg);
    }
</style>
