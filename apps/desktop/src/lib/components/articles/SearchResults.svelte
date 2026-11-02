<script lang="ts">
/**
 * The middle pane while a search is active: a scope bar (hit count, and a
 * switch between the current view and everywhere) over the FTS result window
 * (a fixed, backend-ranked list — no keyset paging) through the same
 * virtualizer and row as the filter list. Selection reads `activeItems`,
 * which is the search results during a search, so j/k and the reader work
 * unchanged.
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import type { ArticleFilters } from '$lib/state/articles.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import ErrorState from '$components/common/ErrorState.svelte';
import EmptyState from '$components/common/EmptyState.svelte';
import { ROW_HEIGHT } from './ArticleList.svelte';
import ArticleRow, { articleOptionId, buildArticleMenu } from './ArticleRow.svelte';
import VirtualList from './VirtualList.svelte';

const resultsError = $derived(
    searchStore.error === null ? '' : commandErrorMessage(searchStore.error),
);

/** The name of the view a scoped search runs in (most specific dimension first). */
function viewName(filters: ArticleFilters): string {
    if (filters.feedId !== null) {
        const feed = feedsStore.feeds.find((candidate) => candidate.id === filters.feedId);
        return feed === undefined ? t('view.feeds') : (feed.title ?? feed.url);
    }
    if (filters.feedTag !== null) {
        return filters.feedTag;
    }
    if (filters.tag !== null) {
        return `#${filters.tag}`;
    }
    if (filters.starred === true) {
        return t('view.starred');
    }
    if (filters.readLater === true) {
        return t('view.readLater');
    }
    if (filters.archived === true) {
        return t('view.archived');
    }
    return t('view.all');
}

/** The view the "this view" half of the scope switch names. */
const scopeName = $derived(viewName(articlesStore.filters));

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

/** Jump navigation while the results listbox holds focus (row-by-row movement
 *  is `j`/`k`; search has no keyset paging, so first/last use the results). */
function moveSelection(to: 'first' | 'last' | 'pageDown' | 'pageUp'): void {
    const results = searchStore.results;
    if (to === 'pageDown') {
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
    <div class="scope-bar">
        <span class="scope-count">
            {#if searchStore.settled && searchStore.error === null && searchStore.results.length > 0}
                {t(searchStore.capped ? 'search.resultsCapped' : 'search.results', {
                    count: searchStore.results.length,
                })}
            {/if}
        </span>
        <div class="scope-switch" role="group" aria-label={t('search.scope.label')}>
            <button
                class="scope-option truncate"
                type="button"
                title={scopeName}
                aria-pressed={!searchStore.everywhere}
                onclick={() => searchStore.setEverywhere(false)}>{scopeName}</button
            >
            <button
                class="scope-option"
                type="button"
                aria-pressed={searchStore.everywhere}
                onclick={() => searchStore.setEverywhere(true)}
                >{t('search.scope.everywhere')}</button
            >
        </div>
    </div>
    {#if searchStore.error !== null}
        <ErrorState message={resultsError} busy={searchStore.loading} onretry={() => searchStore.retry()} />
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
                    highlight={searchStore.query}
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

    .scope-bar {
        flex: 0 0 auto;
        display: flex;
        align-items: center;
        justify-content: space-between;
        gap: var(--space-2);
        min-height: 34px;
        padding: var(--space-1) var(--space-3) var(--space-1) var(--space-4);
        border-bottom: 1px solid var(--hairline);
        background: var(--surface-list);
        font-size: var(--text-xs);
    }

    .scope-count {
        flex: 0 0 auto;
        color: var(--fg-muted);
        font-weight: var(--weight-medium);
        font-variant-numeric: tabular-nums;
    }

    /* A two-way segmented switch: the current view, or the whole library. */
    .scope-switch {
        flex: 0 1 auto;
        display: flex;
        min-width: 0;
        padding: 2px;
        gap: 2px;
        border-radius: var(--radius-sm);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
    }

    .scope-option {
        flex: 0 0 auto;
        padding: 1px var(--space-2);
        border-radius: calc(var(--radius-sm) - 2px);
        background: transparent;
        color: var(--fg-muted);
        font-size: var(--text-xs);
        font-weight: var(--weight-medium);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .scope-option.truncate {
        flex: 0 1 auto;
        min-width: 0;
        max-width: 12rem;
    }

    .scope-option:hover {
        color: var(--fg);
    }

    .scope-option[aria-pressed='true'] {
        background: var(--accent-muted);
        color: var(--accent);
    }

    .status {
        padding: var(--space-4);
        font-size: var(--text-md);
        color: var(--fg-muted);
    }

</style>
