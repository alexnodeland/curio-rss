/**
 * The query cache under state flips (#135): a flag change patches cached
 * rows in place instead of refetching the cache, invalidations coalesce into
 * one batch, entries nothing reads any more go stale instead of refetching,
 * and the registry stays bounded however long the session runs. Lives in a
 * `.svelte.test.ts` so it can observe entries the way components do, from an
 * `$effect.root`.
 */
import {
    events,
    type ArticleStateDto,
    type ArticleSummaryDto,
    type ArticlesChanged,
    type FlagField,
    type ListArticlesDto,
    commands,
} from '$lib/bindings';
import { markReadOnOpen, toggleStar } from '$lib/state/actions';
import { ALL_ARTICLES, type ArticleFilters, articlesStore } from '$lib/state/articles.svelte';
import { articlePrefetcher, neighbourIds } from '$lib/state/prefetch';
import {
    type CommandResult,
    IDLE_CAPS,
    type Query,
    type QueryScope,
    applyArticlesChanged,
    cachedArticleFlag,
    ensureQuery,
    landCarried,
    patchArticleFlag,
    peekQuery,
    pinArticleFlag,
    queryCount,
    queryKeys,
    resetQueryCache,
    unpinArticleFlag,
    wireInvalidation,
} from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { flushSync } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

/** A fetcher that counts its calls and serves `value()` each time. */
function counted<T>(value: () => T): {
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

/** A pure flag flip, as the Rust state commands emit it. */
function flip(field: FlagField, ids: number[], value: boolean): ArticlesChanged {
    return { feed_id: null, flags: { article_ids: ids, field, value } };
}

/** A list query the way `ArticleListState` registers one. */
function listQuery(
    name: string,
    rows: number[],
    flagFilter: QueryScope['flagFilter'],
): { query: Query<{ id: number }[]>; calls: () => number } {
    const fetcher = counted(() => rows.map((id) => ({ id })));
    const query = ensureQuery(queryKeys.articleList(name), fetcher.fetch, {
        feedId: null,
        flagFilter,
    });
    return { query, calls: fetcher.calls };
}

/** Reads `read()` reactively, like a mounted component; returns the unmount. */
function observe(read: () => unknown): () => void {
    const destroy = $effect.root(() => {
        $effect(() => {
            read();
        });
    });
    flushSync();
    return destroy;
}

/** Unmounts and lets the cache notice nothing reads the entries any more. */
async function unmount(destroy: () => void): Promise<void> {
    destroy();
    await flushIpc();
}

describe('query cache — patch in place (#135)', () => {
    afterEach(() => {
        resetQueryCache();
    });

    it('a flag flip patches the cached state and refetches nothing else', async () => {
        const state = counted(() => articleStateFixture());
        const body = counted(() => articleFixture({ id: 7 }));
        const tags = counted(() => ['rust']);
        const counts = counted(() => unreadCountsFixture());
        const stateQuery = ensureQuery<ArticleStateDto>(queryKeys.articleState(7), state.fetch);
        ensureQuery(queryKeys.article(7), body.fetch);
        ensureQuery(queryKeys.articleTags(7), tags.fetch);
        ensureQuery(queryKeys.unreadCounts, counts.fetch);
        await flushIpc();

        applyArticlesChanged(flip('starred', [7], true));
        await flushIpc();

        expect(stateQuery.data?.starred).toBe(true);
        expect([state.calls(), body.calls(), tags.calls(), counts.calls()]).toEqual([1, 1, 1, 1]);
    });

    it('read and archive flips refresh the unread counts; star and read-later do not', async () => {
        const counts = counted(() => unreadCountsFixture());
        ensureQuery(queryKeys.unreadCounts, counts.fetch);
        await flushIpc();

        applyArticlesChanged(flip('starred', [7], true));
        applyArticlesChanged(flip('read_later', [7], true));
        await flushIpc();
        expect(counts.calls()).toBe(1);

        applyArticlesChanged(flip('read', [7], true));
        applyArticlesChanged(flip('archived', [8], true));
        await flushIpc();
        expect(counts.calls()).toBe(2); // both in one coalesced refetch
    });

    it('a list filtering on the other value drops the rows in place, one on this value refetches', async () => {
        const unread = listQuery('unread', [7, 8], { read: false });
        const readOnly = listQuery('read', [9], { read: true });
        const any = listQuery('any', [7, 8, 9], { read: null });
        await flushIpc();

        applyArticlesChanged(flip('read', [7], true));
        await flushIpc();

        expect(unread.query.data).toEqual([{ id: 8 }]);
        expect(unread.calls()).toBe(1); // patched, not refetched
        expect(readOnly.calls()).toBe(2); // 7 may now belong in it
        expect(any.calls()).toBe(1); // ignores the flag
    });

    it('a list on the new value that already holds every flipped row is left alone', async () => {
        const starred = listQuery('starred', [7, 8], { starred: true });
        await flushIpc();

        applyArticlesChanged(flip('starred', [7], true));
        await flushIpc();
        expect(starred.calls()).toBe(1);
    });

    it('a flip scoped to one feed leaves lists of other feeds alone', async () => {
        const fetcher = counted(() => [{ id: 1 }]);
        ensureQuery(queryKeys.articleList('feed2-read'), fetcher.fetch, {
            feedId: 2,
            flagFilter: { read: true },
        });
        await flushIpc();

        applyArticlesChanged({
            feed_id: 1,
            flags: { article_ids: [7], field: 'read', value: true },
        });
        await flushIpc();
        expect(fetcher.calls()).toBe(1);
    });

    it('a flip landing while a state read is in flight refetches it, so an older read cannot win', async () => {
        let resolveFirst: (value: CommandResult<ArticleStateDto>) => void = () => undefined;
        let calls = 0;
        const query = ensureQuery<ArticleStateDto>(queryKeys.articleState(7), () => {
            calls += 1;
            if (calls === 1) {
                return new Promise((resolve) => {
                    resolveFirst = resolve;
                });
            }
            return Promise.resolve({
                status: 'ok' as const,
                data: articleStateFixture({ read: true }),
            });
        });
        await flushIpc(); // the first read is in flight, nothing loaded yet

        applyArticlesChanged(flip('read', [7], true));
        await flushIpc();
        resolveFirst({ status: 'ok', data: articleStateFixture({ read: false }) });
        await flushIpc();

        expect(calls).toBe(2);
        expect(query.data?.read).toBe(true);
    });

    it('a list fetch that started before a flip does not land its older flags over the patch', async () => {
        const state = counted(() => articleStateFixture());
        const stateQuery = ensureQuery<ArticleStateDto>(queryKeys.articleState(7), state.fetch);
        await flushIpc();
        let finish: () => void = () => undefined;
        const list = ensureQuery<number[]>(
            queryKeys.articleList('any'),
            () =>
                new Promise((resolve) => {
                    finish = () => resolve({ status: 'ok', data: [7] });
                }),
            { feedId: null, flagFilter: {} },
            (ids) =>
                ids.map((id) => ({
                    key: queryKeys.articleState(id),
                    fetcher: state.fetch,
                    data: articleStateFixture(), // read before the star committed
                })),
        );
        await flushIpc(); // the list's read is in flight

        applyArticlesChanged(flip('starred', [7], true));
        finish();
        await flushIpc();

        expect(list.data).toEqual([7]);
        expect(stateQuery.data?.starred).toBe(true);
        expect(state.calls()).toBe(1);
    });

    it('a pinned flag keeps the pressed value over older events and reads until unpinned', async () => {
        // The backend still holds the first press's value; a second press is pending.
        const state = counted(() => articleStateFixture({ starred: true }));
        const query = ensureQuery<ArticleStateDto>(queryKeys.articleState(7), state.fetch);
        await flushIpc();
        pinArticleFlag(7, 'starred', false);
        expect(query.data?.starred).toBe(false);

        // The first press's event, a read that predates the pending write, and
        // a list landing its rows: none paints the old value back.
        applyArticlesChanged(flip('starred', [7], true));
        await flushIpc();
        expect(query.data?.starred).toBe(false);
        await query.refetch();
        expect(query.data?.starred).toBe(false);
        landCarried([
            {
                key: queryKeys.articleState(7),
                fetcher: state.fetch,
                data: articleStateFixture({ starred: true }),
            },
        ]);
        expect(query.data?.starred).toBe(false);

        // Once the writes settle, the cache follows events again.
        unpinArticleFlag(7, 'starred');
        applyArticlesChanged(flip('starred', [7], true));
        await flushIpc();
        expect(query.data?.starred).toBe(true);
    });
});

describe('query cache — coalescing and idle entries (#135)', () => {
    afterEach(() => {
        resetQueryCache();
    });

    it('events in one tick refetch each entry once', async () => {
        const list = counted(() => [{ id: 1 }]);
        const counts = counted(() => unreadCountsFixture());
        ensureQuery(queryKeys.articleList('all'), list.fetch, { feedId: null });
        ensureQuery(queryKeys.unreadCounts, counts.fetch);
        await flushIpc();

        applyArticlesChanged({ feed_id: null, flags: null });
        applyArticlesChanged({ feed_id: 1, flags: null });
        applyArticlesChanged(flip('read', [1], true));
        await flushIpc();

        expect(list.calls()).toBe(2);
        expect(counts.calls()).toBe(2);
    });

    it('events in separate tasks while a refetch is in flight cost one follow-up, not one each', async () => {
        const settles: (() => void)[] = [];
        let calls = 0;
        ensureQuery(queryKeys.unreadCounts, () => {
            calls += 1;
            return new Promise<CommandResult<unknown>>((resolve) => {
                settles.push(() => resolve({ status: 'ok', data: unreadCountsFixture() }));
            });
        });
        await flushIpc();
        settles.shift()?.();
        await flushIpc();
        expect(calls).toBe(1);

        // Three read flips, each delivered in its own task (as Tauri delivers
        // events): the first starts a refetch, the rest arrive while it runs.
        for (const id of [1, 2, 3]) {
            applyArticlesChanged(flip('read', [id], true));
            await flushIpc();
        }
        expect(calls).toBe(2);

        settles.shift()?.();
        await flushIpc();
        expect(calls).toBe(3); // one more, after the running one landed
        settles.shift()?.();
        await flushIpc();
        expect(calls).toBe(3);
    });

    it('a stale flag reads as uncached, and whenLoaded refetches it', async () => {
        let starred = false;
        const state = counted(() => articleStateFixture({ starred }));
        const query = ensureQuery<ArticleStateDto>(queryKeys.articleState(7), state.fetch);
        const shown = observe(() => query.data);
        await flushIpc();
        await unmount(shown);

        // Changed by a write whose event names no flag (e.g. saving a URL that
        // already exists sets read-later): the idle entry only goes stale.
        starred = true;
        applyArticlesChanged({ feed_id: null, flags: null });
        await flushIpc();
        expect(query.stale).toBe(true);
        expect(cachedArticleFlag(7, 'starred')).toBeUndefined();
        expect(patchArticleFlag(7, 'starred', false)).toBeNull();

        await query.whenLoaded();
        expect(state.calls()).toBe(2);
        expect(query.data?.starred).toBe(true);
        expect(cachedArticleFlag(7, 'starred')).toBe(true);
    });

    it('an entry nothing reads any more goes stale instead of refetching, and refetches once read again', async () => {
        const body = counted(() => articleFixture({ id: 7 }));
        const query = ensureQuery(queryKeys.article(7), body.fetch);
        const shown = observe(() => query.data);
        await flushIpc();
        expect(query.observed).toBe(true);

        applyArticlesChanged({ feed_id: null, flags: null });
        await flushIpc();
        expect(body.calls()).toBe(2); // observed: refetched

        await unmount(shown);
        expect(query.idle).toBe(true);
        applyArticlesChanged({ feed_id: null, flags: null });
        applyArticlesChanged({ feed_id: null, flags: null });
        await flushIpc();
        expect(body.calls()).toBe(2); // idle: only marked stale
        expect(query.stale).toBe(true);

        const again = observe(() => query.data);
        await flushIpc();
        expect(body.calls()).toBe(3); // read again: one catch-up refetch
        expect(query.stale).toBe(false);
        await unmount(again);
    });

    it('an idle list covers none of its rows: a shown row it carries refetches on its own', async () => {
        const detail = counted(() => articleStateFixture({ read: true }));
        const list = ensureQuery<number[]>(
            queryKeys.articleList('idle'),
            () => Promise.resolve({ status: 'ok', data: [1] }),
            { feedId: null },
            (ids) =>
                ids.map((id) => ({
                    key: queryKeys.articleState(id),
                    fetcher: detail.fetch,
                    data: articleStateFixture(),
                })),
        );
        const listShown = observe(() => list.data);
        await flushIpc();
        await unmount(listShown);
        const row = ensureQuery<ArticleStateDto>(queryKeys.articleState(1), detail.fetch);
        const rowShown = observe(() => row.data);
        await flushIpc();
        expect(detail.calls()).toBe(0); // landed by the list

        applyArticlesChanged({ feed_id: null, flags: null });
        await flushIpc();

        // The idle list only went stale — its landing is not coming — so the
        // row the reader shows was refetched rather than left to it.
        expect(list.stale).toBe(true);
        expect(detail.calls()).toBe(1);
        expect(row.data?.read).toBe(true);
        await unmount(rowShown);
    });

    it('a list view nothing shows any more goes stale too', async () => {
        const harness = installIpcHarness({ list_articles: [articleSummaryFixture({ id: 1 })] });
        try {
            const view = articlesStore.list({ ...ALL_ARTICLES, starred: true });
            const shown = observe(() => view.items);
            await flushIpc();
            await unmount(shown);

            applyArticlesChanged({ feed_id: null, flags: null });
            await flushIpc();
            expect(harness.callsFor('list_articles')).toHaveLength(1);
            expect(view.query.stale).toBe(true);
        } finally {
            articlesStore.reset();
            harness.teardown();
        }
    });
});

describe('query cache — bounded registry (#135)', () => {
    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
    });

    it('stays bounded over a long session of opened articles', async () => {
        const ok =
            <T>(data: T) =>
            () =>
                Promise.resolve({ status: 'ok' as const, data });
        let shown = observe(() => undefined);
        for (let id = 1; id <= 1000; id += 1) {
            await unmount(shown);
            // The reader for article `id`: its body, flags and tags.
            const body = ensureQuery(queryKeys.article(id), ok(articleFixture({ id })));
            const state = ensureQuery(queryKeys.articleState(id), ok(articleStateFixture()));
            const tags = ensureQuery(queryKeys.articleTags(id), ok<string[]>([]));
            // A neighbour prefetched but never shown counts toward the cap too.
            ensureQuery(queryKeys.articleState(id + 5000), ok(articleStateFixture()));
            shown = observe(() => [body.data, state.data, tags.data]);
        }
        await flushIpc();

        expect(queryCount('article:')).toBeLessThanOrEqual(IDLE_CAPS.article + 1);
        expect(queryCount('article-state:')).toBeLessThanOrEqual(IDLE_CAPS.articleState + 1);
        expect(queryCount('article-tags:')).toBeLessThanOrEqual(IDLE_CAPS.articleTags + 1);
        // The open article is read, so it is never the one evicted.
        expect(queryCount(queryKeys.article(1000))).toBe(1);
        await unmount(shown);
    });

    it('keeps the list registry bounded across many visited views', async () => {
        const harness = installIpcHarness({ list_articles: [] });
        try {
            let shown = observe(() => undefined);
            for (let feedId = 1; feedId <= 60; feedId += 1) {
                await unmount(shown);
                const view = articlesStore.list({ ...ALL_ARTICLES, feedId });
                shown = observe(() => view.items);
            }
            await flushIpc();
            // One more visit sweeps the list states whose windows were evicted.
            articlesStore.list(ALL_ARTICLES);

            expect(queryCount('articles:')).toBeLessThanOrEqual(IDLE_CAPS.articles + 2);
            expect(articlesStore.listCount).toBeLessThanOrEqual(IDLE_CAPS.articles + 2);
            await unmount(shown);
        } finally {
            harness.teardown();
        }
    });

    it('a view whose window was evicted comes back as a fresh, loading list', async () => {
        const harness = installIpcHarness({ list_articles: [articleSummaryFixture({ id: 3 })] });
        try {
            const first = articlesStore.list({ ...ALL_ARTICLES, feedId: 1 });
            const shown = observe(() => first.items);
            await flushIpc();
            await unmount(shown);
            for (let feedId = 2; feedId <= IDLE_CAPS.articles + 2; feedId += 1) {
                articlesStore.list({ ...ALL_ARTICLES, feedId });
            }
            await flushIpc();

            const back = articlesStore.list({ ...ALL_ARTICLES, feedId: 1 });
            expect(back).not.toBe(first);
            await flushIpc();
            expect(back.items.map((row) => row.id)).toEqual([3]);
        } finally {
            harness.teardown();
        }
    });
});

describe('acceptance — one toggleStar after a session (#135)', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        articlePrefetcher.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    /** 40 articles over two feeds, with backend-side flags the commands flip. */
    function sessionHarness(): IpcHarness {
        const rows: ArticleSummaryDto[] = Array.from({ length: 40 }, (_, index) =>
            articleSummaryFixture({ id: 40 - index, feed_id: index % 2 === 0 ? 1 : 2 }),
        );
        // Articles 1–8 start starred and 9–16 queued, so those views have rows to open.
        const flags = new Map<number, ArticleStateDto>(
            rows.map((row) => [
                row.id,
                articleStateFixture({
                    starred: row.id <= 8,
                    read_later: row.id > 8 && row.id <= 16,
                }),
            ]),
        );
        const matches = (params: ListArticlesDto, row: ArticleSummaryDto): boolean => {
            const state = flags.get(row.id) ?? articleStateFixture();
            return (
                (params.feed_id === null || row.feed_id === params.feed_id) &&
                (params.starred === null || state.starred === params.starred) &&
                (params.read_later === null || state.read_later === params.read_later) &&
                (params.before === null || row.id < params.before)
            );
        };
        const set = (field: FlagField, id: number, value: boolean): boolean => {
            const state = flags.get(id) ?? articleStateFixture();
            const changed = state[field] !== value;
            flags.set(id, { ...state, [field]: value });
            if (changed) {
                // What the Rust command emits after a real change.
                void events.articlesChanged.emit(flip(field, [id], value));
            }
            return changed;
        };
        return installIpcHarness({
            list_articles: (args) => {
                const params = args.params as ListArticlesDto;
                // Each row carries its flags, as `list_articles` rows do.
                return rows
                    .filter((row) => matches(params, row))
                    .slice(0, params.limit)
                    .map((row) => ({ ...row, state: flags.get(row.id) ?? articleStateFixture() }));
            },
            get_article: (args) => articleFixture({ id: args.articleId as number }),
            get_article_state: (args) => flags.get(args.articleId as number),
            get_unread_counts: unreadCountsFixture(),
            mark_read: (args) => set('read', args.articleId as number, args.read as boolean),
            set_starred: (args) =>
                set('starred', args.articleId as number, args.starred as boolean),
        });
    }

    const VIEWS: ArticleFilters[] = [
        ALL_ARTICLES,
        { ...ALL_ARTICLES, starred: true },
        { ...ALL_ARTICLES, readLater: true },
        { ...ALL_ARTICLES, feedId: 1 },
        { ...ALL_ARTICLES, feedId: 2 },
    ];

    it('opening 20 articles across 5 views leaves one toggleStar at most 3 IPC calls', async () => {
        harness = sessionHarness();
        const unwire = await wireInvalidation();
        // The sidebar's unread badges are on screen the whole session.
        const sidebar = observe(
            () => ensureQuery(queryKeys.unreadCounts, () => commands.getUnreadCounts()).data,
        );
        let screen = observe(() => undefined);
        let lastOpened = 0;
        for (const filters of VIEWS) {
            await unmount(screen);
            articlesStore.filters = filters;
            const list = articlesStore.current;
            await list.query.whenLoaded();
            const opened = list.items.slice(0, 4).map((row) => row.id);
            for (const [index, id] of opened.entries()) {
                await unmount(screen);
                selectionStore.selectedArticleId = id;
                // The reader's body + flags, and the rows on screen.
                const body = ensureQuery(queryKeys.article(id), () => commands.getArticle(id));
                const state = ensureQuery(queryKeys.articleState(id), () =>
                    commands.getArticleState(id),
                );
                screen = observe(() => [list.items, body.data, state.data]);
                articlePrefetcher.prefetch(neighbourIds(list.items, index, id), id);
                await markReadOnOpen(id);
                await flushIpc();
                lastOpened = id;
            }
        }
        await flushIpc();
        expect(harness.callsFor('mark_read')).toHaveLength(20);

        const before = harness.calls.length;
        await toggleStar(lastOpened);
        await flushIpc();
        await flushIpc();
        const toggleCalls = harness.calls.slice(before).map((call) => call.cmd);

        expect(toggleCalls.length).toBeLessThanOrEqual(3);
        // In fact one: the write. The flag is patched in place, and the idle
        // Starred view only goes stale until it is shown again.
        expect(toggleCalls).toEqual(['set_starred']);
        expect(articlesStore.list(VIEWS[1]).query.stale).toBe(true);
        expect(peekQuery<ArticleStateDto>(queryKeys.articleState(lastOpened))?.data?.starred).toBe(
            true,
        );

        await unmount(screen);
        await unmount(sidebar);
        unwire();
    });
});
