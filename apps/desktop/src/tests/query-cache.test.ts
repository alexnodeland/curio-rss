/**
 * Query-cache behavior: keyed caching, stale-while-revalidate refetch, and
 * — the load-bearing part — event-driven invalidation through the generated
 * specta event bindings, scoped exactly as the Rust emitters promise.
 */
import { events } from '$lib/bindings';
import {
    type CommandResult,
    ensureQuery,
    hasQuery,
    invalidatePrefix,
    landCarried,
    queryKeys,
    resetQueryCache,
    wireInvalidation,
} from '$lib/state/query-cache.svelte';
import { landRowDetails } from '$lib/state/row-details';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
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

    it('whenLoaded on a brand-new entry resolves with its data in one fetch', async () => {
        const fetcher = countingFetcher(() => ['vault']);
        const query = ensureQuery('destinations', fetcher.fetch);
        // No flush: the creation refetch is still only a queued microtask.
        await query.whenLoaded();
        expect(query.loaded).toBe(true);
        expect(query.data).toEqual(['vault']);
        // The queued creation load reuses the fetch whenLoaded started.
        await flushIpc();
        expect(fetcher.calls()).toBe(1);
    });

    it('whenLoaded keeps waiting when a newer refetch supersedes the one it awaits', async () => {
        const pending: ((value: CommandResult<string>) => void)[] = [];
        const query = ensureQuery(
            'feeds',
            () => new Promise<CommandResult<string>>((resolve) => pending.push(resolve)),
        );
        await flushIpc(); // the creation refetch is now in flight (pending[0])

        let settled = false;
        // whenLoaded starts awaiting pending[0] before anything supersedes it.
        const waiting = query.whenLoaded().then(() => {
            settled = true;
        });
        void query.refetch(); // supersedes it (pending[1])
        expect(pending).toHaveLength(2);

        pending[0]({ status: 'ok', data: 'stale' });
        await flushIpc();
        expect(settled).toBe(false); // the superseded result landed nothing
        expect(query.loaded).toBe(false);

        pending[1]({ status: 'ok', data: 'fresh' });
        await waiting;
        expect(settled).toBe(true);
        expect(query.data).toBe('fresh');
    });

    it('whenLoaded is immediate (no refetch) once an entry has loaded', async () => {
        const fetcher = countingFetcher(() => 'feeds');
        const query = ensureQuery('feeds', fetcher.fetch);
        await flushIpc();
        await query.whenLoaded();
        expect(fetcher.calls()).toBe(1);
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

    it('a landed entry is loaded without a fetch and refetches on invalidation', async () => {
        const fetcher = countingFetcher(() => 'fetched');
        landCarried([{ key: 'article-state:7', fetcher: fetcher.fetch, data: 'landed' }]);
        const query = ensureQuery('article-state:7', fetcher.fetch);
        await query.whenLoaded();
        await flushIpc();
        expect(query.data).toBe('landed');
        expect(query.loaded).toBe(true);
        expect(fetcher.calls()).toBe(0);

        invalidatePrefix('article-state:');
        await flushIpc();
        expect(query.data).toBe('fetched');
        expect(fetcher.calls()).toBe(1);
    });

    it('landing overwrites an existing entry and supersedes its in-flight fetch', async () => {
        const pending: ((value: CommandResult<string>) => void)[] = [];
        const fetcher = () =>
            new Promise<CommandResult<string>>((resolve) => pending.push(resolve));
        const query = ensureQuery('article-tags:7', fetcher);
        await flushIpc();
        expect(pending).toHaveLength(1);

        landCarried([{ key: 'article-tags:7', fetcher, data: 'landed' }]);
        expect(ensureQuery('article-tags:7', fetcher)).toBe(query);
        expect(query.data).toBe('landed');
        expect(query.loading).toBe(false);

        pending[0]({ status: 'ok', data: 'older' });
        await flushIpc();
        expect(query.data).toBe('landed'); // the superseded fetch lands nothing
    });

    it('carries lands each landed result, never a superseded one', async () => {
        let payload = 'first';
        let release: () => void = () => undefined;
        const gate = new Promise<void>((resolve) => {
            release = resolve;
        });
        let calls = 0;
        const detail = countingFetcher(() => 'fetched');
        const query = ensureQuery(
            'articles:x',
            async (): Promise<CommandResult<string>> => {
                calls += 1;
                const data = payload;
                if (calls === 1) {
                    await gate; // the first fetch is overtaken
                }
                return { status: 'ok', data };
            },
            {},
            (data) => [{ key: 'article-state:1', fetcher: detail.fetch, data }],
        );
        await Promise.resolve();
        payload = 'second';
        await query.refetch();
        expect(ensureQuery('article-state:1', detail.fetch).data).toBe('second');
        release();
        await flushIpc();
        expect(query.data).toBe('second');
        expect(ensureQuery('article-state:1', detail.fetch).data).toBe('second');
        expect(detail.calls()).toBe(0);
    });

    it('list rows land their state and tags entries', async () => {
        harness = installIpcHarness({});
        const state = articleStateFixture({ starred: true });
        landRowDetails([articleSummaryFixture({ id: 9, state, tags: ['a'] })]);
        const { commands } = await import('$lib/bindings');
        const stateQuery = ensureQuery(queryKeys.articleState(9), () =>
            commands.getArticleState(9),
        );
        const tagsQuery = ensureQuery(queryKeys.articleTags(9), () => commands.getArticleTags(9));
        await flushIpc();
        expect(stateQuery.data).toEqual(state);
        expect(tagsQuery.data).toEqual(['a']);
        expect(harness.callsFor('get_article_state')).toEqual([]);
        expect(harness.callsFor('get_article_tags')).toEqual([]);
    });

    describe('ArticlesChanged over carried detail entries', () => {
        /** A list of `ids` whose rows carry `article-state:<id>` = `flag`. */
        function carryingList(
            initial: number[],
            flag: () => string,
            detail: ReturnType<typeof countingFetcher<string>>,
            fail: () => boolean = () => false,
        ) {
            let ids = initial;
            const list = countingFetcher(() => ids);
            const query = ensureQuery<number[]>(
                queryKeys.articleList('x'),
                () =>
                    fail()
                        ? Promise.resolve({
                              status: 'error' as const,
                              error: commandErrorFixture(),
                          })
                        : list.fetch(),
                { feedId: null },
                (rows) =>
                    rows.map((id) => ({
                        key: queryKeys.articleState(id),
                        fetcher: detail.fetch,
                        data: `${flag()} ${id}`,
                    })),
            );
            return {
                query,
                list,
                setIds: (next: number[]) => {
                    ids = next;
                },
            };
        }

        it('a refetching list refreshes the rows it carries with no per-row call', async () => {
            harness = installIpcHarness({});
            let flag = 'unread';
            const detail = countingFetcher(() => 'fetched');
            const { list } = carryingList([1, 2, 3], () => flag, detail);
            const unwire = await wireInvalidation();
            await flushIpc();
            const row = ensureQuery(queryKeys.articleState(2), detail.fetch);
            expect(row.data).toBe('unread 2');

            flag = 'read';
            await events.articlesChanged.emit({ feed_id: 1 });
            await flushIpc();

            expect(list.calls()).toBe(2);
            expect(detail.calls()).toBe(0);
            expect(row.data).toBe('read 2');
            unwire();
        });

        it('a carried row the refetch drops is refetched when read, dropped when not', async () => {
            harness = installIpcHarness({});
            const detail = countingFetcher(() => 'fetched');
            const { setIds } = carryingList([1, 2, 3], () => 'unread', detail);
            const unwire = await wireInvalidation();
            await flushIpc();
            const shown = ensureQuery(queryKeys.articleState(2), detail.fetch);

            setIds([1]); // 2 and 3 left the window (read in an unread-only view)
            await events.articlesChanged.emit({ feed_id: null });
            await flushIpc();

            expect(detail.calls()).toBe(1); // only the row something reads
            expect(shown.data).toBe('fetched');
            expect(hasQuery(queryKeys.articleState(3))).toBe(false);
            expect(hasQuery(queryKeys.articleState(1))).toBe(true);
            unwire();
        });

        it('a failed list refetch settles every row it was left', async () => {
            harness = installIpcHarness({});
            let fail = false;
            const detail = countingFetcher(() => 'fetched');
            carryingList(
                [1, 2],
                () => 'unread',
                detail,
                () => fail,
            );
            const unwire = await wireInvalidation();
            await flushIpc();
            const shown = ensureQuery(queryKeys.articleState(1), detail.fetch);

            fail = true;
            await events.articlesChanged.emit({ feed_id: null });
            await flushIpc();

            expect(detail.calls()).toBe(1);
            expect(shown.data).toBe('fetched');
            expect(hasQuery(queryKeys.articleState(2))).toBe(false);
            unwire();
        });

        it('an entry no list carries refetches when read and is dropped when not', async () => {
            harness = installIpcHarness({});
            const detail = countingFetcher(() => 'fetched');
            landCarried([
                { key: queryKeys.articleState(8), fetcher: detail.fetch, data: 'landed 8' },
                { key: queryKeys.articleState(9), fetcher: detail.fetch, data: 'landed 9' },
            ]);
            const shown = ensureQuery(queryKeys.articleState(8), detail.fetch);
            const unwire = await wireInvalidation();
            await flushIpc();

            await events.articlesChanged.emit({ feed_id: null });
            await flushIpc();

            expect(detail.calls()).toBe(1);
            expect(shown.data).toBe('fetched');
            expect(hasQuery(queryKeys.articleState(9))).toBe(false);
            unwire();
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
