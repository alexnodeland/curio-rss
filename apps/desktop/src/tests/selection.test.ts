/**
 * Selection state: j/k walking over the backend-ordered window, load-more
 * margin signaling, and feed selection re-scoping the article list.
 */
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { LOAD_MORE_MARGIN, SelectionStore } from '$lib/state/selection.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, articleSummaryFixture, flushIpc, installIpcHarness } from './ipc-harness';

const ROWS = Array.from({ length: 10 }, (_, i) =>
    articleSummaryFixture({ id: 100 - i, title: `Article ${100 - i}` }),
);

describe('selection store', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        articlesStore.reset();
        resetQueryCache();
        harness?.teardown();
        harness = null;
    });

    async function loadedSelection(): Promise<SelectionStore> {
        harness = installIpcHarness({ list_articles: ROWS });
        articlesStore.filters = ALL_ARTICLES;
        void articlesStore.current; // touch to start the fetch
        await flushIpc();
        return new SelectionStore();
    }

    it('selects the first row when nothing is selected, then advances', async () => {
        const selection = await loadedSelection();

        selection.selectNextArticle();
        expect(selection.selectedArticleId).toBe(100);

        selection.selectNextArticle();
        expect(selection.selectedArticleId).toBe(99);
        expect(selection.selectedArticle?.title).toBe('Article 99');
    });

    it('clamps at both ends of the window', async () => {
        const selection = await loadedSelection();

        selection.selectPreviousArticle();
        expect(selection.selectedArticleId).toBe(100); // k from nothing: first row

        for (let i = 0; i < 20; i += 1) {
            selection.selectNextArticle();
        }
        expect(selection.selectedArticleId).toBe(91); // last loaded row

        selection.selectPreviousArticle();
        expect(selection.selectedArticleId).toBe(92);
    });

    it('signals when selection enters the load-more margin', async () => {
        const selection = await loadedSelection();

        let wantsMore = selection.selectNextArticle(); // index 0 of 10
        expect(wantsMore).toBe(false);

        // Walk to index 5: exactly LOAD_MORE_MARGIN rows remain below it.
        for (let i = 0; i < 10 - LOAD_MORE_MARGIN; i += 1) {
            wantsMore = selection.selectNextArticle();
        }
        expect(wantsMore).toBe(true);
    });

    it('selectFeed re-scopes the list filters and clears the selection', async () => {
        const selection = await loadedSelection();
        selection.selectNextArticle();
        expect(selection.selectedArticleId).not.toBeNull();

        selection.selectFeed(7);

        expect(selection.selectedFeedId).toBe(7);
        expect(selection.selectedArticleId).toBeNull();
        expect(articlesStore.filters.feedId).toBe(7);

        selection.selectFeed(null);
        expect(articlesStore.filters.feedId).toBeNull();
    });

    it('does nothing on an empty window', async () => {
        harness = installIpcHarness({ list_articles: [] });
        articlesStore.filters = ALL_ARTICLES;
        void articlesStore.current;
        await flushIpc();
        const selection = new SelectionStore();

        expect(selection.selectNextArticle()).toBe(false);
        selection.selectPreviousArticle();
        expect(selection.selectedArticleId).toBeNull();
        expect(selection.selectedArticle).toBeNull();
    });
});
