<script lang="ts">
/**
 * The middle pane while a search is active: the FTS result window (a fixed,
 * backend-ordered list — no keyset paging) through the same virtualizer and
 * row as the filter list. Selection reads `activeItems`, which is the search
 * results during a search, so j/k and the reader work unchanged.
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { handleShortcut } from '$lib/state/actions';
import { menuStore } from '$lib/state/menu.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import EmptyState from '$components/common/EmptyState.svelte';
import { ROW_HEIGHT } from './ArticleList.svelte';
import ArticleRow, { articleOptionId, buildArticleMenu } from './ArticleRow.svelte';
import VirtualList from './VirtualList.svelte';

const resultsError = $derived(
    searchStore.error === null ? '' : commandErrorMessage(searchStore.error),
);

/** The listbox's active option: the selected result, when one is selected. */
const activeDescendantId = $derived(
    selectionStore.selectedArticleId === null
        ? undefined
        : articleOptionId(selectionStore.selectedArticleId),
);

function selectArticle(articleId: number): void {
    selectionStore.selectedArticleId = articleId;
}

// Keep the last-present index current (parity with ArticleList) so j/k resume
// correctly if the selected result leaves the window.
$effect(() => {
    const index = selectionStore.selectedIndex;
    if (index >= 0) {
        selectionStore.rememberIndex(index);
    }
});

/** Arrow/Page navigation while the results listbox holds focus (same paths as
 *  the filter list; search has no keyset paging, so first/last use the results). */
function moveSelection(to: 'next' | 'previous' | 'first' | 'last' | 'pageDown' | 'pageUp'): void {
    const results = searchStore.results;
    if (to === 'next') {
        handleShortcut('nav.nextArticle');
    } else if (to === 'previous') {
        handleShortcut('nav.previousArticle');
    } else if (to === 'pageDown') {
        selectionStore.selectPageDown();
    } else if (to === 'pageUp') {
        selectionStore.selectPageUp();
    } else if (to === 'first' && results.length > 0) {
        selectionStore.selectedArticleId = results[0].id;
        selectionStore.rememberIndex(0);
    } else if (to === 'last' && results.length > 0) {
        selectionStore.selectedArticleId = results[results.length - 1].id;
        selectionStore.rememberIndex(results.length - 1);
    }
}

/** Opens the context menu for the selected result on the keyboard menu key. */
function openSelectedMenu(): void {
    const id = selectionStore.selectedArticleId;
    if (id === null) {
        return;
    }
    const article = searchStore.results.find((item) => item.id === id);
    const el = document.getElementById(articleOptionId(id));
    if (article === undefined || el === null) {
        return;
    }
    const state = ensureQuery(queryKeys.articleState(id), () => commands.getArticleState(id)).data;
    const rect = el.getBoundingClientRect();
    menuStore.openAt(
        buildArticleMenu(article, state),
        rect.left + 8,
        rect.bottom - 4,
        el,
        article.title,
    );
}
</script>

<div class="search-results">
    {#if searchStore.error !== null}
        <p class="status error" role="alert">{resultsError}</p>
    {:else if searchStore.results.length === 0 && !searchStore.settled}
        <!-- Debounce/in-flight gap: the query hasn't resolved, so show
             "Searching…" rather than flashing "No matches". -->
        <p class="status">{t('search.loading')}</p>
    {:else if searchStore.results.length === 0}
        <EmptyState icon="search" title={t('search.empty')} />
    {:else}
        <VirtualList
            items={searchStore.results}
            rowHeight={ROW_HEIGHT}
            key={(article) => article.id}
            selectedIndex={selectionStore.selectedIndex}
            label={t('search.label')}
            {activeDescendantId}
            focusNonce={selectionStore.listFocusNonce}
            onmove={moveSelection}
            onmenukey={openSelectedMenu}
            onactivate={() => handleShortcut('article.open')}
        >
            {#snippet row(article, index)}
                <ArticleRow
                    {article}
                    {index}
                    rowHeight={ROW_HEIGHT}
                    compact={false}
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
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

    .status.error {
        color: var(--error-text);
    }
</style>
