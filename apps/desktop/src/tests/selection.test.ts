/**
 * Selection state: j/k walking over the backend-ordered window, load-more
 * margin signaling, and feed selection re-scoping the article list.
 */
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { LOAD_MORE_MARGIN, PAGE_ROWS, SelectionStore } from '$lib/state/selection.svelte';
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

    it('resumes j/k from the last position when the selected row leaves the window', async () => {
        // Reproduces the auto-mark-read case: selecting a row marks it read, an
        // unread view refetches it out, and selectedIndex goes to -1. j must
        // not teleport to the top and k must still step back.
        const selection = await loadedSelection();
        for (let i = 0; i < 4; i += 1) {
            selection.selectNextArticle(); // lands on index 3 (id 97)
        }
        expect(selection.selectedArticleId).toBe(97);

        // Simulate the row dropping out of the window (now absent → index -1).
        selection.selectedArticleId = 999;
        expect(selection.selectedIndex).toBe(-1);

        // j resumes at the captured position (the row that shifted into the gap).
        selection.selectNextArticle();
        expect(selection.selectedArticleId).toBe(97);

        // k from the same orphaned state steps back, not to the top.
        selection.selectedArticleId = 999;
        selection.selectPreviousArticle();
        expect(selection.selectedArticleId).toBe(98);
    });

    it('PageDown / PageUp move a page and clamp at the ends', async () => {
        const selection = await loadedSelection();
        selection.selectNextArticle(); // index 0 (id 100)

        selection.selectPageDown(); // +PAGE_ROWS, clamped to the last loaded row
        expect(PAGE_ROWS).toBeGreaterThanOrEqual(ROWS.length);
        expect(selection.selectedArticleId).toBe(91); // last of the 10 rows

        selection.selectPageUp(); // back to the top
        expect(selection.selectedArticleId).toBe(100);
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
