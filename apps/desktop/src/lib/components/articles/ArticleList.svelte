<script lang="ts" module>
import type { ReadingDensity } from '$lib/state/ui.svelte';

/**
 * Fixed virtualizer row height in px per density. The row applies the same
 * number as an inline height, so the CSS and the windowing math share one
 * source and cannot drift.
 */
export const ROW_HEIGHTS: Record<ReadingDensity, number> = {
    comfortable: 84,
    compact: 56,
};

/** The comfortable height — the fixed row height where density isn't varied (e.g. search). */
export const ROW_HEIGHT = ROW_HEIGHTS.comfortable;
</script>

<script lang="ts">
/**
 * The middle pane: the current filter combination's loaded window through
 * the virtualizer, with keyset `before`-cursor infinite scroll (the store
 * owns the cursor; nearing the bottom of the window asks it for the next
 * page). Selection lives in `selectionStore`; the reader reacts to it.
 */
import { commands } from '$lib/bindings';
import { t } from '$lib/i18n';
import { handleShortcut, markReadOnScroll } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { ensureQuery, queryKeys } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import Skeleton from '$components/common/Skeleton.svelte';
import { untrack } from 'svelte';
import ArticleRow, { articleOptionId, buildArticleMenu } from './ArticleRow.svelte';
import VirtualList from './VirtualList.svelte';

/** How many placeholder rows the loading skeleton sketches. */
const SKELETON_ROWS = Array.from({ length: 8 }, (_, index) => index);

// Rows resolve feed titles — prime the feed queries outside the template.
untrack(() => feedsStore.prime());

/**
 * The current filter combination's list state. Created through a $derived
 * — never inline in a template expression — because svelte excludes state
 * created inside a reaction from that reaction's dependencies: a template
 * that both created and read the query would go blind to its updates.
 */
const list = $derived(articlesStore.current);

const listError = $derived(list.error === null ? '' : commandErrorMessage(list.error));

/** Row density → the fixed row height and the compact-layout flag. */
const rowHeight = $derived(ROW_HEIGHTS[uiStore.readingDensity]);
const compact = $derived(uiStore.readingDensity === 'compact');

/** The listbox's active option: the selected row, when one is selected. */
const activeDescendantId = $derived(
    selectionStore.selectedArticleId === null
        ? undefined
        : articleOptionId(selectionStore.selectedArticleId),
);

function selectArticle(articleId: number): void {
    selectionStore.selectedArticleId = articleId;
}

/**
 * Opens the context menu for the selected row when the keyboard menu key fires
 * on the listbox (rows are `tabindex="-1"`, so their own menu-key never fires).
 * Anchors to the selected row's element; a no-op if nothing is selected or the
 * row has scrolled out of the virtualized window.
 */
function openSelectedMenu(): void {
    const id = selectionStore.selectedArticleId;
    if (id === null) {
        return;
    }
    const article = list.items.find((item) => item.id === id);
    const el = document.getElementById(articleOptionId(id));
    if (article === undefined || el === null) {
        return;
    }
    const state = ensureQuery(queryKeys.articleState(id), () => commands.getArticleState(id)).data;
    const rect = el.getBoundingClientRect();
    menuStore.openAt(buildArticleMenu(article, state), rect.left + 8, rect.bottom - 4, el, article.title);
}

/**
 * Arrow-key navigation while the listbox holds focus. Down/Up reuse the exact
 * `j`/`k` path (so paging-in stays wired); Home/End jump to the ends of the
 * loaded window.
 */
function moveSelection(to: 'next' | 'previous' | 'first' | 'last'): void {
    const items = list.items;
    if (to === 'next') {
        handleShortcut('nav.nextArticle');
    } else if (to === 'previous') {
        handleShortcut('nav.previousArticle');
    } else if (to === 'first' && items.length > 0) {
        selectionStore.selectedArticleId = items[0].id;
    } else if (to === 'last' && items.length > 0) {
        selectionStore.selectedArticleId = items[items.length - 1].id;
    }
}

/**
 * Mark-on-scroll high-water: the number of leading rows already marked read
 * this filter-session. It only advances (scrolling back up never re-marks),
 * and resets when the filter changes to a fresh list.
 */
let markedThrough = 0;

$effect(() => {
    // Re-key on the list identity: a new filter is a fresh reading session.
    void list;
    markedThrough = 0;
});

/**
 * When the setting is on, rows that scrolled up past the top mark themselves
 * read. Only the newly passed rows (past the high-water) are touched, so a
 * fast scroll costs one mark per row, once.
 */
function onScrollPast(firstVisibleIndex: number): void {
    if (!uiStore.markOnScroll) {
        return;
    }
    const items = list.items;
    const target = Math.min(firstVisibleIndex, items.length);
    for (let index = markedThrough; index < target; index++) {
        void markReadOnScroll(items[index].id);
    }
    if (target > markedThrough) {
        markedThrough = target;
    }
}
</script>

<div class="article-list">
    {#if list.error !== null}
        <p class="list-status error" role="alert">{listError}</p>
    {:else if !list.loaded}
        <div class="list-skeleton" aria-hidden="true">
            {#each SKELETON_ROWS as index (index)}
                <div class="skeleton-row" style="height: {rowHeight}px">
                    <div class="skeleton-row-main">
                        <Skeleton width="65%" height="0.9rem" />
                        {#if !compact}
                            <Skeleton width="92%" height="0.75rem" />
                        {/if}
                        <Skeleton width="35%" height="0.7rem" />
                    </div>
                    {#if !compact}
                        <Skeleton width="52px" height="52px" radius="var(--radius-md)" />
                    {/if}
                </div>
            {/each}
        </div>
        <span class="sr-only">{t('list.loading')}</span>
    {:else if list.items.length === 0}
        <p class="list-status">{t('list.empty')}</p>
    {:else}
        <VirtualList
            items={list.items}
            {rowHeight}
            key={(article) => article.id}
            selectedIndex={selectionStore.selectedIndex}
            label={t('list.label')}
            {activeDescendantId}
            onnearend={() => void list.loadMore()}
            onscrollpast={onScrollPast}
            onmove={moveSelection}
            onmenukey={openSelectedMenu}
        >
            {#snippet row(article, index)}
                <ArticleRow
                    {article}
                    {index}
                    {rowHeight}
                    {compact}
                    setsize={list.items.length}
                    selected={article.id === selectionStore.selectedArticleId}
                    onselect={selectArticle}
                />
            {/snippet}
        </VirtualList>
    {/if}
</div>

<style>
    .article-list {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-height: 0;
        background: var(--bg);
    }

    .list-status {
        padding: var(--space-4);
        font-size: 0.875rem;
        color: var(--fg-muted);
    }

    .list-status.error {
        color: var(--error-text);
    }

    .list-skeleton {
        display: flex;
        flex-direction: column;
    }

    .skeleton-row {
        display: flex;
        align-items: center;
        gap: var(--space-3);
        padding: 0 var(--space-4);
        margin: 0 var(--space-2);
    }

    .skeleton-row-main {
        flex: 1 1 auto;
        min-width: 0;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
</style>
