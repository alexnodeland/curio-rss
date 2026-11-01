/**
 * Article-list state: keyset `before`-cursor pagination, backend-owned
 * filters passed through verbatim (never re-filtered or re-sorted
 * client-side), and invalidation reloading the loaded window.
 */
import type { ArticleSummaryDto, ListArticlesDto } from '$lib/bindings';
import { events } from '$lib/bindings';
import {
    ALL_ARTICLES,
    ArticleListState,
    DEFAULT_PAGE_SIZE,
    articlesStore,
    filterKey,
} from '$lib/state/articles.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, articleSummaryFixture, flushIpc, installIpcHarness } from './ipc-harness';

/** `count` summaries descending from `fromId` — backend `id DESC` order. */
function page(fromId: number, count: number): ArticleSummaryDto[] {
    return Array.from({ length: count }, (_, i) =>
        articleSummaryFixture({ id: fromId - i, title: `Article ${fromId - i}` }),
    );
}

/** A list_articles responder serving a fixed descending id space. */
function backend(totalTopId: number, totalCount: number) {
    const all = page(totalTopId, totalCount);
    return (args: Record<string, unknown>) => {
        const params = args.params as ListArticlesDto;
        const below = params.before;
        const window = below === null ? all : all.filter((a) => a.id < below);
        return window.slice(0, params.limit);
    };
}

describe('article list state', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        articlesStore.reset();
        resetQueryCache();
        harness?.teardown();
        harness = null;
    });

    it('loads the first page with a null cursor and the exact filter DTO', async () => {
        harness = installIpcHarness({ list_articles: backend(500, 10) });
        const filters = { ...ALL_ARTICLES, feedId: 3, read: false, tag: 'rust' };
        const list = new ArticleListState(filters);
        await flushIpc();

        expect(list.items).toHaveLength(10);
        const sent = harness.callsFor('list_articles')[0].params as ListArticlesDto;
        expect(sent).toEqual({
            feed_id: 3,
            before: null,
            limit: DEFAULT_PAGE_SIZE,
            read: false,
            starred: null,
            read_later: null,
            // The default hides archived (archiving = out of the flow).
            archived: false,
            tag: 'rust',
            feed_tag: null,
        });
    });

    it('preserves backend order verbatim — no client re-sort', async () => {
        // Deliberately non-monotonic ids: whatever the backend says, we render.
        const rows = [
            articleSummaryFixture({ id: 9 }),
            articleSummaryFixture({ id: 40 }),
            articleSummaryFixture({ id: 2 }),
        ];
        harness = installIpcHarness({ list_articles: rows });
        const list = new ArticleListState(ALL_ARTICLES);
        await flushIpc();

        expect(list.items.map((a) => a.id)).toEqual([9, 40, 2]);
        expect(list.exhausted).toBe(true); // 3 < page size
    });

    it('advances the keyset cursor on loadMore and appends', async () => {
        harness = installIpcHarness({ list_articles: backend(1000, 120) });
        const list = new ArticleListState(ALL_ARTICLES);
        await flushIpc();
        expect(list.items).toHaveLength(DEFAULT_PAGE_SIZE);

        await list.loadMore();
        expect(list.items).toHaveLength(2 * DEFAULT_PAGE_SIZE);
        expect(list.exhausted).toBe(false);

        const calls = harness.callsFor('list_articles');
        expect((calls[1].params as ListArticlesDto).before).toBe(1000 - DEFAULT_PAGE_SIZE + 1);

        await list.loadMore();
        expect(list.items).toHaveLength(120);
        expect(list.exhausted).toBe(true); // short page = end of the id space

        await list.loadMore(); // no-op once exhausted
        expect(harness.callsFor('list_articles')).toHaveLength(3);
    });

    it('reloads the whole loaded window on ArticlesChanged', async () => {
        harness = installIpcHarness({ list_articles: backend(1000, 120) });
        const list = new ArticleListState(ALL_ARTICLES);
        const unwire = await wireInvalidation();
        await flushIpc();
        await list.loadMore();
        expect(list.items).toHaveLength(100);
        const callsBefore = harness.callsFor('list_articles').length;

        await events.articlesChanged.emit({ feed_id: null });
        await flushIpc();

        // The refetch walked the loaded range page by page (2 pages).
        expect(harness.callsFor('list_articles')).toHaveLength(callsBefore + 2);
        expect(list.items).toHaveLength(100);
        unwire();
    });

    it('scopes invalidation: another feed leaves a feed-scoped list alone', async () => {
        harness = installIpcHarness({ list_articles: backend(100, 5) });
        const list = new ArticleListState({ ...ALL_ARTICLES, feedId: 7 });
        const unwire = await wireInvalidation();
        await flushIpc();
        expect(list.loaded).toBe(true);
        const callsBefore = harness.callsFor('list_articles').length;

        await events.articlesChanged.emit({ feed_id: 3 });
        await flushIpc();
        expect(harness.callsFor('list_articles')).toHaveLength(callsBefore);

        await events.articlesChanged.emit({ feed_id: 7 });
        await flushIpc();
        expect(harness.callsFor('list_articles')).toHaveLength(callsBefore + 1);
        unwire();
    });

    it('keeps the current window when a loadMore page errors', async () => {
        harness = installIpcHarness({ list_articles: backend(1000, 120) });
        const list = new ArticleListState(ALL_ARTICLES);
        await flushIpc();

        harness.respond('list_articles', () =>
            Promise.reject({
                kind: 'internal',
                code: 'STORAGE',
                message: 'writer gone',
                recoverable: true,
            }),
        );
        await list.loadMore();
        expect(list.items).toHaveLength(DEFAULT_PAGE_SIZE);
        expect(list.exhausted).toBe(false);
        expect(list.appending).toBe(false);
    });
});

describe('articles store', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        articlesStore.reset();
        resetQueryCache();
        harness?.teardown();
        harness = null;
    });

    it('caches one list state per filter combination', async () => {
        harness = installIpcHarness({ list_articles: [] });
        const a = articlesStore.list(ALL_ARTICLES);
        const b = articlesStore.list({ ...ALL_ARTICLES });
        const c = articlesStore.list({ ...ALL_ARTICLES, starred: true });
        await flushIpc();

        expect(b).toBe(a);
        expect(c).not.toBe(a);
        expect(filterKey(ALL_ARTICLES)).not.toBe(filterKey({ ...ALL_ARTICLES, starred: true }));
    });

    it('current follows the reactive filter set', async () => {
        harness = installIpcHarness({ list_articles: [] });
        const before = articlesStore.current;
        articlesStore.filters = { ...ALL_ARTICLES, readLater: true };
        const after = articlesStore.current;
        await flushIpc();

        expect(after).not.toBe(before);
        const sent = harness.callsFor('list_articles').map((c) => c.params as ListArticlesDto);
        expect(sent.some((p) => p.read_later === true)).toBe(true);
    });

    it('toggleUnreadOnly flips read between false and null, preserving other filters', async () => {
        harness = installIpcHarness({ list_articles: [] });
        articlesStore.filters = { ...ALL_ARTICLES, feedId: 5, starred: true };

        expect(articlesStore.unreadOnly).toBe(false);
        articlesStore.toggleUnreadOnly();
        expect(articlesStore.unreadOnly).toBe(true);
        expect(articlesStore.filters).toEqual({
            ...ALL_ARTICLES,
            feedId: 5,
            starred: true,
            read: false,
        });

        articlesStore.toggleUnreadOnly();
        expect(articlesStore.unreadOnly).toBe(false);
        expect(articlesStore.filters.read).toBeNull();
        // The other dimensions never move.
        expect(articlesStore.filters.feedId).toBe(5);
        expect(articlesStore.filters.starred).toBe(true);
    });

    it('the unread-only window is its own cached list, sending read:false to the backend', async () => {
        harness = installIpcHarness({ list_articles: [] });
        articlesStore.toggleUnreadOnly();
        void articlesStore.current;
        await flushIpc();

        const sent = harness.callsFor('list_articles').map((c) => c.params as ListArticlesDto);
        expect(sent.some((p) => p.read === false)).toBe(true);
    });

    it('passes state mutations through and returns the changed flag', async () => {
        harness = installIpcHarness({ mark_read: true, set_starred: false });
        const marked = await articlesStore.markRead(9, true);
        const starred = await articlesStore.setStarred(9, true);

        expect(marked).toEqual({ status: 'ok', data: true });
        expect(starred).toEqual({ status: 'ok', data: false }); // idempotent no-op
        expect(harness.callsFor('mark_read')[0]).toEqual({ articleId: 9, read: true });
    });

    describe('session read-hold', () => {
        /** An unread-only backend over ids 105..101; `read` ids drop out. */
        function unreadBackend(read: Set<number>) {
            return (args: Record<string, unknown>) => {
                const params = args.params as ListArticlesDto;
                return page(105, 5)
                    .filter((a) => params.read !== false || !read.has(a.id))
                    .filter((a) => params.before === null || a.id < params.before)
                    .slice(0, params.limit);
            };
        }

        const UNREAD = { ...ALL_ARTICLES, read: false };

        it('keeps a row read this session in its slot when the unread window drops it', async () => {
            const read = new Set<number>();
            harness = installIpcHarness({ list_articles: unreadBackend(read) });
            articlesStore.filters = UNREAD;
            const list = articlesStore.current;
            await flushIpc();

            articlesStore.holdRead(103);
            read.add(103);
            await list.query.refetch();

            expect(list.query.data?.map((a) => a.id)).toEqual([105, 104, 102, 101]);
            expect(list.items.map((a) => a.id)).toEqual([105, 104, 103, 102, 101]);
            expect(list.isHeld(103)).toBe(true);
        });

        it('holding a row that is not displayed, or twice, changes nothing', async () => {
            harness = installIpcHarness({ list_articles: unreadBackend(new Set()) });
            articlesStore.filters = UNREAD;
            const list = articlesStore.current;
            await flushIpc();

            articlesStore.holdRead(999);
            expect(list.isHeld(999)).toBe(false);
            articlesStore.holdRead(104);
            articlesStore.holdRead(104);
            expect(list.isHeld(104)).toBe(true);
        });

        it('a view switch ends the outgoing session: returning shows the backend window', async () => {
            const read = new Set<number>();
            harness = installIpcHarness({ list_articles: unreadBackend(read) });
            articlesStore.filters = UNREAD;
            const list = articlesStore.current;
            await flushIpc();
            articlesStore.holdRead(103);
            read.add(103);
            await list.query.refetch();

            // Re-assigning the same filter set is not a view change.
            articlesStore.filters = { ...UNREAD };
            expect(list.isHeld(103)).toBe(true);

            articlesStore.filters = ALL_ARTICLES;
            articlesStore.filters = UNREAD;
            expect(articlesStore.current).toBe(list);
            expect(list.items.map((a) => a.id)).toEqual([105, 104, 102, 101]);
        });

        it('releaseHeld lets one row go; releaseHolds ends the whole hold', async () => {
            const read = new Set<number>();
            harness = installIpcHarness({ list_articles: unreadBackend(read) });
            articlesStore.filters = UNREAD;
            const list = articlesStore.current;
            await flushIpc();
            articlesStore.holdRead(104);
            articlesStore.holdRead(103);
            read.add(104);
            read.add(103);
            await list.query.refetch();
            expect(list.items).toHaveLength(5);

            articlesStore.releaseHeld(104);
            articlesStore.releaseHeld(104); // already released: a no-op
            expect(list.items.map((a) => a.id)).toEqual([105, 103, 102, 101]);

            articlesStore.releaseHolds();
            articlesStore.releaseHolds(); // nothing held: a no-op
            expect(list.items.map((a) => a.id)).toEqual([105, 102, 101]);
        });

        it('pages from the backend window, never from a held row', async () => {
            const read = new Set<number>();
            harness = installIpcHarness({ list_articles: backend(500, 10) });
            const list = new ArticleListState(UNREAD, 3);
            await flushIpc();
            expect(list.items.map((a) => a.id)).toEqual([500, 499, 498]);
            list.hold(498);
            // The backend now leaves 498 out of the window.
            read.add(498);
            harness.respond('list_articles', (args) => {
                const params = args.params as ListArticlesDto;
                return page(500, 10)
                    .filter((a) => !read.has(a.id))
                    .filter((a) => params.before === null || a.id < params.before)
                    .slice(0, params.limit);
            });
            await list.query.refetch();
            expect(list.query.data?.map((a) => a.id)).toEqual([500, 499, 497]);
            expect(list.items.map((a) => a.id)).toEqual([500, 499, 498, 497]);

            await list.loadMore();
            const cursors = harness
                .callsFor('list_articles')
                .map((call) => (call.params as ListArticlesDto).before);
            expect(cursors[cursors.length - 1]).toBe(497);
        });

        it('a hold reaches no list while nothing is shown for the filters', () => {
            harness = installIpcHarness({ list_articles: [] });
            // No list was ever created for the current filters: nothing to hold.
            articlesStore.holdRead(1);
            articlesStore.releaseHeld(1);
            articlesStore.releaseHolds();
            expect(harness.callsFor('list_articles')).toHaveLength(0);
        });
    });
});
