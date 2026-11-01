import { ArticlePrefetcher, PREFETCH_RADIUS, neighbourIds } from '$lib/state/prefetch';
/**
 * The reader's neighbour prefetch: which ids it picks around the selection,
 * and that it evicts only the body queries it created once they fall out of
 * range — never one the reader (or anyone else) created.
 */
import { ensureQuery, hasQuery, queryKeys, resetQueryCache } from '$lib/state/query-cache.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleFixture,
    articleStateFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';

const ITEMS = [10, 20, 30, 40, 50, 60].map((id) => ({ id }));

describe('neighbourIds', () => {
    it('orders the window nearest first, forward before backward', () => {
        expect(neighbourIds(ITEMS, 2, 30)).toEqual([40, 20, 50, 10]);
        expect(PREFETCH_RADIUS).toBe(2);
    });

    it('clips at both ends of the window', () => {
        expect(neighbourIds(ITEMS, 0, 10)).toEqual([20, 30]);
        expect(neighbourIds(ITEMS, 5, 60)).toEqual([50, 40]);
    });

    it('includes the row that shifted into a dropped selection’s slot', () => {
        // The selected row (99) left the window; 30 now sits at its old index.
        expect(neighbourIds(ITEMS, 2, 99)).toEqual([30, 40, 20, 50, 10]);
    });

    it('honours a custom radius and degenerate inputs', () => {
        expect(neighbourIds(ITEMS, 2, 30, 1)).toEqual([40, 20]);
        expect(neighbourIds([], 0, null)).toEqual([]);
        expect(neighbourIds(ITEMS, -1, null)).toEqual([]);
    });
});

describe('ArticlePrefetcher', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        harness?.teardown();
        harness = null;
    });

    function install(): IpcHarness {
        harness = installIpcHarness({
            get_article: (args) => articleFixture({ id: args.articleId as number }),
            get_article_state: articleStateFixture(),
        });
        return harness;
    }

    it('loads bodies and flags for the given ids', async () => {
        const ipc = install();
        const prefetcher = new ArticlePrefetcher();
        prefetcher.prefetch([1, 2], 3);
        await flushIpc();

        expect(ipc.callsFor('get_article')).toEqual([{ articleId: 1 }, { articleId: 2 }]);
        expect(ipc.callsFor('get_article_state')).toEqual([{ articleId: 1 }, { articleId: 2 }]);
        expect(prefetcher.owned).toEqual([1, 2]);
    });

    it('drops its own bodies once out of range, but keeps the selected one', () => {
        install();
        const prefetcher = new ArticlePrefetcher();
        prefetcher.prefetch([1, 2], 3);
        prefetcher.prefetch([4], 2); // 2 is now selected; 1 left the range

        expect(hasQuery(queryKeys.article(1))).toBe(false);
        expect(hasQuery(queryKeys.article(2))).toBe(true);
        expect([...prefetcher.owned].sort((a, b) => a - b)).toEqual([2, 4]);

        prefetcher.prefetch([], null); // selection cleared: evict everything owned
        expect(prefetcher.owned).toEqual([]);
        expect(hasQuery(queryKeys.article(2))).toBe(false);
        expect(hasQuery(queryKeys.article(4))).toBe(false);
    });

    it('retain drops out-of-range bodies without fetching anything', async () => {
        const ipc = install();
        const prefetcher = new ArticlePrefetcher();
        prefetcher.prefetch([1, 2, 3], null);
        await flushIpc();
        const fetched = ipc.callsFor('get_article').length;

        prefetcher.retain([2, 9], 3); // 1 left the range, 3 is selected, 9 is new
        await flushIpc();

        expect(ipc.callsFor('get_article')).toHaveLength(fetched);
        expect(hasQuery(queryKeys.article(1))).toBe(false);
        expect(hasQuery(queryKeys.article(9))).toBe(false);
        expect([...prefetcher.owned].sort((a, b) => a - b)).toEqual([2, 3]);
    });

    it('never adopts or drops a body query someone else created', () => {
        install();
        ensureQuery(queryKeys.article(7), () => Promise.resolve({ status: 'ok', data: null }));
        const prefetcher = new ArticlePrefetcher();
        prefetcher.prefetch([7], 1);
        prefetcher.prefetch([], null);

        expect(prefetcher.owned).toEqual([]);
        expect(hasQuery(queryKeys.article(7))).toBe(true);
    });

    it('leaves a successor alone when its own entry was replaced meanwhile', () => {
        install();
        const prefetcher = new ArticlePrefetcher();
        prefetcher.prefetch([8], null);
        resetQueryCache(); // the owned instance is gone…
        const successor = ensureQuery(queryKeys.article(8), () =>
            Promise.resolve({ status: 'ok', data: null }),
        ); // …and someone else created a new one under the same key
        prefetcher.prefetch([], null);

        expect(prefetcher.owned).toEqual([]);
        expect(hasQuery(queryKeys.article(8))).toBe(true);
        expect(
            ensureQuery(queryKeys.article(8), () => Promise.resolve({ status: 'ok', data: null })),
        ).toBe(successor);
    });

    it('reset forgets ownership without touching the cache', () => {
        install();
        const prefetcher = new ArticlePrefetcher();
        prefetcher.prefetch([5], null);
        prefetcher.reset();
        expect(prefetcher.owned).toEqual([]);
        expect(hasQuery(queryKeys.article(5))).toBe(true);
    });
});
