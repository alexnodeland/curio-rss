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
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
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
