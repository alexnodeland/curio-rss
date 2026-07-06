/**
 * Query-cache behavior: keyed caching, stale-while-revalidate refetch, and
 * — the load-bearing part — event-driven invalidation through the generated
 * specta event bindings, scoped exactly as the Rust emitters promise.
 */
import { events } from '$lib/bindings';
import {
    type CommandResult,
    ensureQuery,
    invalidatePrefix,
    queryKeys,
    resetQueryCache,
    wireInvalidation,
} from '$lib/state/query-cache.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';

function countingFetcher<T>(value: () => T): {
    calls: () => number;
    fetch: () => Promise<CommandResult<T>>;
} {
    let count = 0;
    return {
        calls: () => count,
        fetch: () => {
            count += 1;
            return Promise.resolve({ status: 'ok' as const, data: value() });
        },
    };
}

describe('query cache', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        harness?.teardown();
        harness = null;
    });

    it('returns the same entry for the same key and fetches once', async () => {
        const fetcher = countingFetcher(() => [1, 2, 3]);
        const first = ensureQuery('feeds', fetcher.fetch);
        const second = ensureQuery('feeds', fetcher.fetch);
        await flushIpc();

        expect(second).toBe(first);
        expect(fetcher.calls()).toBe(1);
        expect(first.data).toEqual([1, 2, 3]);
        expect(first.loaded).toBe(true);
    });

    it('keeps stale data while a refetch is in flight and swaps on settle', async () => {
        let payload = 'one';
        const query = ensureQuery('feeds', () =>
            Promise.resolve({ status: 'ok' as const, data: payload }),
        );
        await flushIpc();
        expect(query.data).toBe('one');

        payload = 'two';
        const refetching = query.refetch();
        expect(query.loading).toBe(true);
        expect(query.data).toBe('one'); // stale-while-revalidate
        await refetching;
        expect(query.data).toBe('two');
        expect(query.loading).toBe(false);
    });

    it('surfaces a CommandError rejection without dropping stale data', async () => {
        let fail = false;
        const error = commandErrorFixture({ kind: 'internal', code: 'STORAGE' });
        const query = ensureQuery('feeds', () =>
            fail
                ? Promise.resolve({ status: 'error' as const, error })
                : Promise.resolve({ status: 'ok' as const, data: 'good' }),
        );
        await flushIpc();
        expect(query.data).toBe('good');

        fail = true;
        await query.refetch();
        expect(query.error).toEqual(error);
        expect(query.data).toBe('good');
    });

    it('folds a thrown Error (no Tauri runtime) into an internal error state', async () => {
        const query = ensureQuery('feeds', () => Promise.reject(new Error('no runtime')));
        await flushIpc();
        expect(query.error?.kind).toBe('internal');
        expect(query.error?.message).toBe('no runtime');
        expect(query.loaded).toBe(true);
    });

    it('invalidatePrefix refetches only matching keys', async () => {
        const feeds = countingFetcher(() => 'feeds');
        const tags = countingFetcher(() => 'tags');
        ensureQuery(queryKeys.feeds, feeds.fetch);
        ensureQuery(queryKeys.tags, tags.fetch);
        await flushIpc();

        invalidatePrefix(queryKeys.feeds);
        await flushIpc();
        expect(feeds.calls()).toBe(2);
        expect(tags.calls()).toBe(1);
    });

    describe('event-driven invalidation (generated event bindings)', () => {
        it('FeedsChanged refetches the feeds namespace only', async () => {
            harness = installIpcHarness({});
            const feeds = countingFetcher(() => 'feeds');
            const counts = countingFetcher(() => 'counts');
            ensureQuery(queryKeys.feeds, feeds.fetch);
            ensureQuery(queryKeys.unreadCounts, counts.fetch);
            const unwire = await wireInvalidation();
            await flushIpc();

            await events.feedsChanged.emit();
            await flushIpc();

            expect(feeds.calls()).toBe(2);
            expect(counts.calls()).toBe(1);
            unwire();
        });

        it('ArticlesChanged scopes list invalidation by feed and always hits counts', async () => {
            harness = installIpcHarness({});
            const listFeed1 = countingFetcher(() => 'feed-1 list');
            const listFeed2 = countingFetcher(() => 'feed-2 list');
            const listAll = countingFetcher(() => 'all list');
            const counts = countingFetcher(() => 'counts');
            const detail = countingFetcher(() => 'article 7');

            ensureQuery(queryKeys.articleList('feed=1'), listFeed1.fetch, { feedId: 1 });
            ensureQuery(queryKeys.articleList('feed=2'), listFeed2.fetch, { feedId: 2 });
            ensureQuery(queryKeys.articleList('feed=-'), listAll.fetch, { feedId: null });
            ensureQuery(queryKeys.unreadCounts, counts.fetch);
            ensureQuery(queryKeys.article(7), detail.fetch);
            const unwire = await wireInvalidation();
            await flushIpc();

            await events.articlesChanged.emit({ feed_id: 1 });
            await flushIpc();

            expect(listFeed1.calls()).toBe(2); // scoped to the changed feed
            expect(listFeed2.calls()).toBe(1); // other feed untouched
            expect(listAll.calls()).toBe(2); // unscoped lists include feed 1
            expect(counts.calls()).toBe(2); // read-state flips move badges
            expect(detail.calls()).toBe(2); // detail namespaces refresh

            await events.articlesChanged.emit({ feed_id: null });
            await flushIpc();

            expect(listFeed1.calls()).toBe(3); // broad event hits every list
            expect(listFeed2.calls()).toBe(2);
            expect(listAll.calls()).toBe(3);
            unwire();
        });

        it('unsubscribing stops invalidation', async () => {
            harness = installIpcHarness({});
            const feeds = countingFetcher(() => 'feeds');
            ensureQuery(queryKeys.feeds, feeds.fetch);
            const unwire = await wireInvalidation();
            await flushIpc();

            unwire();
            await events.feedsChanged.emit();
            await flushIpc();

            expect(feeds.calls()).toBe(1);
        });
    });

    it('rejectWith produces the wire error shape end to end', async () => {
        const error = commandErrorFixture({ code: 'STORAGE', kind: 'internal' });
        harness = installIpcHarness({ list_tags: rejectWith(error) });
        const { commands } = await import('$lib/bindings');
        const query = ensureQuery(queryKeys.tags, commands.listTags);
        await flushIpc();
        expect(query.error).toEqual(error);
    });
});
