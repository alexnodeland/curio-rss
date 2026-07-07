<script lang="ts" module>
/** Fixed virtualizer row height in px — ArticleRow's CSS height matches. */
export const ROW_HEIGHT = 84;
</script>

<script lang="ts">
/**
 * The middle pane: the current filter combination's loaded window through
 * the virtualizer, with keyset `before`-cursor infinite scroll (the store
 * owns the cursor; nearing the bottom of the window asks it for the next
 * page). Selection lives in `selectionStore`; the reader reacts to it.
 */
import { t } from '$lib/i18n';
import { markReadOnScroll } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { commandErrorMessage } from '$lib/utils/errors';
import { untrack } from 'svelte';
import ArticleRow from './ArticleRow.svelte';
import VirtualList from './VirtualList.svelte';

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

function selectArticle(articleId: number): void {
    selectionStore.selectedArticleId = articleId;
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
        <p class="list-status">{t('list.loading')}</p>
    {:else if list.items.length === 0}
        <p class="list-status">{t('list.empty')}</p>
    {:else}
        <VirtualList
            items={list.items}
            rowHeight={ROW_HEIGHT}
            key={(article) => article.id}
            selectedIndex={selectionStore.selectedIndex}
            label={t('list.label')}
            onnearend={() => void list.loadMore()}
            onscrollpast={onScrollPast}
        >
            {#snippet row(article, index)}
                <ArticleRow
                    {article}
                    {index}
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
        color: var(--error);
    }
</style>
