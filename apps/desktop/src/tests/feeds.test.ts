/**
 * Feeds store: backend-owned subscription list + unread counts (no client
 * badge math) and refresh progress driven by the generated events.
 */
import { events } from '$lib/bindings';
import { FeedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    feedFixture,
    flushIpc,
    installIpcHarness,
    refreshOutcomeFixture,
    unreadCountsFixture,
} from './ipc-harness';

describe('feeds store', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        harness?.teardown();
        harness = null;
    });

    it('loads feeds lazily and exposes backend unread counts', async () => {
        harness = installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1 }),
                feedFixture({ id: 2, url: 'https://b.example/f' }),
            ],
            get_unread_counts: unreadCountsFixture({
                total: 7,
                by_feed: [
                    [1, 4],
                    [2, 3],
                ],
            }),
        });
        const store = new FeedsStore();

        expect(store.feeds).toEqual([]); // first touch kicks the fetch off
        expect(store.unreadTotal).toBe(0); // same for the counts query
        await flushIpc();

        expect(store.feeds).toHaveLength(2);
        expect(store.loaded).toBe(true);
        expect(store.unreadTotal).toBe(7);
        expect(store.unreadFor(1)).toBe(4);
        expect(store.unreadFor(2)).toBe(3);
        expect(store.unreadFor(99)).toBe(0); // unknown feed: zero, not NaN
    });

    it('accumulates refresh progress from events and settles on finished', async () => {
        harness = installIpcHarness({});
        const store = new FeedsStore();
        const unwire = await store.wireRefreshEvents();

        const first = refreshOutcomeFixture({ feed_id: 1, new_articles: 2 });
        const second = refreshOutcomeFixture({ feed_id: 2, status: 'not_modified' });

        await events.refreshProgress.emit({
            sweep_id: 1,
            done: 1,
            total: 2,
            pages_done: 0,
            pages_total: 0,
            feed_id: 1,
            outcome: first,
        });
        expect(store.refreshing).toBe(true);
        expect(store.refreshOutcomes).toEqual([first]);

        await events.refreshProgress.emit({
            sweep_id: 1,
            done: 2,
            total: 2,
            pages_done: 0,
            pages_total: 0,
            feed_id: 2,
            outcome: second,
        });
        expect(store.refreshOutcomes).toEqual([first, second]);

        await events.refreshFinished.emit({
            sweep_id: 1,
            cancelled: false,
            outcomes: [first, second],
        });
        expect(store.refreshing).toBe(false);
        expect(store.refreshOutcomes).toEqual([first, second]);
        unwire();
    });

    it('tracks the sweep count and starts afresh on a new sweep', async () => {
        harness = installIpcHarness({});
        const store = new FeedsStore();
        const unwire = await store.wireRefreshEvents();
        // The last sweep's outcomes, still shown in its summary.
        store.refreshOutcomes = [refreshOutcomeFixture({ feed_id: 9 })];

        const first = refreshOutcomeFixture({ feed_id: 1 });
        await events.refreshProgress.emit({
            sweep_id: 4,
            done: 12,
            total: 300,
            pages_done: 0,
            pages_total: 0,
            feed_id: 1,
            outcome: first,
        });
        expect(store.refreshProgress).toEqual({
            sweepId: 4,
            done: 12,
            total: 300,
            pagesDone: 0,
            pagesTotal: 0,
        });
        // A sweep the scheduler started never adds to the last one's.
        expect(store.refreshOutcomes).toEqual([first]);

        await events.refreshFinished.emit({ sweep_id: 4, cancelled: false, outcomes: [first] });
        expect(store.refreshProgress).toBeNull();
        expect(store.refreshCancelled).toBe(false);
        unwire();
    });

    it('cancels the sweep in flight and settles on its cancelled finish', async () => {
        harness = installIpcHarness({ cancel_refresh: true });
        const store = new FeedsStore();
        const unwire = await store.wireRefreshEvents();
        const outcome = refreshOutcomeFixture({ feed_id: 1 });
        await events.refreshProgress.emit({
            sweep_id: 1,
            done: 1,
            total: 3,
            pages_done: 0,
            pages_total: 0,
            feed_id: 1,
            outcome,
        });

        const result = await store.cancelRefresh();
        expect(result).toEqual({ status: 'ok', data: true });
        expect(harness.callsFor('cancel_refresh')).toHaveLength(1);
        expect(store.cancellingRefresh).toBe(true);
        expect(store.refreshing).toBe(true);

        await events.refreshFinished.emit({ sweep_id: 1, cancelled: true, outcomes: [outcome] });
        expect(store.refreshing).toBe(false);
        expect(store.cancellingRefresh).toBe(false);
        expect(store.refreshCancelled).toBe(true);
        expect(store.refreshOutcomes).toEqual([outcome]);
        unwire();
    });

    it('counts full-text pages past the last feed and stays alive while they load', async () => {
        vi.useFakeTimers();
        try {
            harness = installIpcHarness({});
            const store = new FeedsStore();
            const unwire = await store.wireRefreshEvents();
            const outcome = refreshOutcomeFixture({ feed_id: 1, new_articles: 3 });
            await events.refreshProgress.emit({
                sweep_id: 3,
                done: 1,
                total: 1,
                pages_done: 0,
                pages_total: 3,
                feed_id: 1,
                outcome,
            });
            // Each page re-arms the watchdog: a long run of pages, one
            // every 100 s, never trips its 120 s deadline.
            for (const pagesDone of [1, 2]) {
                await vi.advanceTimersByTimeAsync(100_000);
                await events.refreshProgress.emit({
                    sweep_id: 3,
                    done: 1,
                    total: 1,
                    pages_done: pagesDone,
                    pages_total: 3,
                    feed_id: null,
                    outcome: null,
                });
            }
            await vi.advanceTimersByTimeAsync(100_000);
            expect(store.refreshing).toBe(true);
            expect(store.refreshProgress).toEqual({
                sweepId: 3,
                done: 1,
                total: 1,
                pagesDone: 2,
                pagesTotal: 3,
            });
            // A page carries no outcome of its own.
            expect(store.refreshOutcomes).toEqual([outcome]);
            unwire();
        } finally {
            vi.useRealTimers();
        }
    });

    it('a late event of a sweep the watchdog gave up on keeps its outcomes', async () => {
        vi.useFakeTimers();
        try {
            harness = installIpcHarness({});
            const store = new FeedsStore();
            const unwire = await store.wireRefreshEvents();
            const first = refreshOutcomeFixture({ feed_id: 1 });
            const second = refreshOutcomeFixture({ feed_id: 2 });
            await events.refreshProgress.emit({
                sweep_id: 5,
                done: 1,
                total: 2,
                pages_done: 0,
                pages_total: 0,
                feed_id: 1,
                outcome: first,
            });
            await vi.advanceTimersByTimeAsync(120_000);
            expect(store.refreshing).toBe(false);
            expect(store.refreshProgress).toBeNull();

            await events.refreshProgress.emit({
                sweep_id: 5,
                done: 2,
                total: 2,
                pages_done: 0,
                pages_total: 0,
                feed_id: 2,
                outcome: second,
            });
            expect(store.refreshing).toBe(true);
            expect(store.refreshOutcomes).toEqual([first, second]);
            unwire();
        } finally {
            vi.useRealTimers();
        }
    });

    it('drops the stopping state when no sweep was running to cancel', async () => {
        harness = installIpcHarness({ cancel_refresh: false });
        const store = new FeedsStore();
        await store.cancelRefresh();
        expect(store.cancellingRefresh).toBe(false);
    });

    it('a refresh request during a sweep joins it without wiping its progress', async () => {
        harness = installIpcHarness({ refresh_all: [] });
        const store = new FeedsStore();
        const unwire = await store.wireRefreshEvents();
        const outcome = refreshOutcomeFixture({ feed_id: 1 });
        await events.refreshProgress.emit({
            sweep_id: 2,
            done: 1,
            total: 5,
            pages_done: 0,
            pages_total: 0,
            feed_id: 1,
            outcome,
        });

        await store.refreshAll();
        expect(store.refreshOutcomes).toEqual([outcome]);
        expect(store.refreshProgress).toEqual({
            sweepId: 2,
            done: 1,
            total: 5,
            pagesDone: 0,
            pagesTotal: 0,
        });
        unwire();
    });

    it('lastErrored flags only feeds whose last refresh errored', () => {
        harness = installIpcHarness({});
        const store = new FeedsStore();
        store.refreshOutcomes = [
            refreshOutcomeFixture({ feed_id: 1, status: 'error' }),
            refreshOutcomeFixture({ feed_id: 2, status: 'ok' }),
        ];
        expect(store.lastErrored(1)).toBe(true);
        expect(store.lastErrored(2)).toBe(false);
        expect(store.lastErrored(99)).toBe(false); // no outcome yet
    });

    it('mutes and unmutes a feed for notifications, persisting the id set', async () => {
        harness = installIpcHarness({ set_setting: null });
        const store = new FeedsStore();

        expect(store.isNotifyMuted(5)).toBe(false);
        store.setNotifyMuted(5, true);
        expect(store.isNotifyMuted(5)).toBe(true);
        await flushIpc();
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.notify.muted-feeds', value: '["5"]' },
        ]);

        store.setNotifyMuted(5, false);
        expect(store.isNotifyMuted(5)).toBe(false);
        await flushIpc();
        expect(harness.callsFor('set_setting')[1]).toEqual({
            key: 'ui.notify.muted-feeds',
            value: '[]',
        });
    });

    it('clears the refreshing flag when refresh_all itself errors', async () => {
        harness = installIpcHarness({
            refresh_all: () =>
                Promise.reject({
                    kind: 'internal',
                    code: 'STORAGE',
                    message: 'writer gone',
                    recoverable: true,
                }),
        });
        const store = new FeedsStore();
        const result = await store.refreshAll();

        expect(result.status).toBe('error');
        expect(store.refreshing).toBe(false);
    });

    it('passes feed mutations through the generated wrappers', async () => {
        harness = installIpcHarness({
            add_feed: feedFixture({ id: 5 }),
            remove_feed: null,
            set_feed_status: null,
        });
        const store = new FeedsStore();

        const added = await store.addFeed({
            url: 'https://example.org/feed.xml',
            title: null,
            tags: [],
            validate: true,
        });
        expect(added.status).toBe('ok');

        await store.removeFeed(5);
        await store.setFeedStatus(5, 'paused');

        expect(harness.callsFor('remove_feed')[0]).toEqual({ feedId: 5 });
        expect(harness.callsFor('set_feed_status')[0]).toEqual({ feedId: 5, status: 'paused' });
    });
});
