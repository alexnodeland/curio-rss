<script lang="ts">
/**
 * The middle pane while a search is active: the FTS result window (a fixed,
 * backend-ordered list — no keyset paging) through the same virtualizer and
 * row as the filter list. Selection reads `activeItems`, which is the search
 * results during a search, so j/k and the reader work unchanged.
 */
import { t } from '$lib/i18n';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import { ROW_HEIGHT } from './ArticleList.svelte';
import ArticleRow from './ArticleRow.svelte';
import VirtualList from './VirtualList.svelte';

const resultsError = $derived(
    searchStore.error === null ? '' : commandErrorMessage(searchStore.error),
);

function selectArticle(articleId: number): void {
    selectionStore.selectedArticleId = articleId;
}
</script>

<div class="search-results">
    {#if searchStore.error !== null}
        <p class="status error" role="alert">{resultsError}</p>
    {:else if searchStore.loading && searchStore.results.length === 0}
        <p class="status">{t('search.loading')}</p>
    {:else if searchStore.results.length === 0}
        <p class="status">{t('search.empty')}</p>
    {:else}
        <VirtualList
            items={searchStore.results}
            rowHeight={ROW_HEIGHT}
            key={(article) => article.id}
            selectedIndex={selectionStore.selectedIndex}
            label={t('search.label')}
        >
            {#snippet row(article, index)}
                <ArticleRow
                    {article}
                    {index}
                    setsize={searchStore.results.length}
                    selected={article.id === selectionStore.selectedArticleId}
                    onselect={selectArticle}
                />
            {/snippet}
        </VirtualList>
    {/if}
</div>

<style>
    .search-results {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--bg);
    }

    .status {
        padding: var(--space-4);
        font-size: 0.875rem;
        color: var(--fg-muted);
    }

    .status.error {
        color: var(--error);
    }
</style>
