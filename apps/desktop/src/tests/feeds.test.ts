/**
 * Feeds store: backend-owned subscription list + unread counts (no client
 * badge math) and refresh progress driven by the generated events.
 */
import { events } from '$lib/bindings';
import { FeedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { afterEach, describe, expect, it } from 'vitest';
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

        await events.refreshProgress.emit({ feed_id: 1, outcome: first });
        expect(store.refreshing).toBe(true);
        expect(store.refreshOutcomes).toEqual([first]);

        await events.refreshProgress.emit({ feed_id: 2, outcome: second });
        expect(store.refreshOutcomes).toEqual([first, second]);

        await events.refreshFinished.emit({ outcomes: [first, second] });
        expect(store.refreshing).toBe(false);
        expect(store.refreshOutcomes).toEqual([first, second]);
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
        });
        expect(added.status).toBe('ok');

        await store.removeFeed(5);
        await store.setFeedStatus(5, 'paused');

        expect(harness.callsFor('remove_feed')[0]).toEqual({ feedId: 5 });
        expect(harness.callsFor('set_feed_status')[0]).toEqual({ feedId: 5, status: 'paused' });
    });
});
