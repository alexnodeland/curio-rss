/**
 * The feed-health panel: a feed's recent fetch log (`recent_fetches`) plus
 * lifecycle controls (`set_feed_status`). Status changes go through the
 * feeds store; the panel resolves the feed from the (primed) feed list.
 */
import FeedHealthPanel from '$components/modals/FeedHealthPanel.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    feedFixture,
    fetchRecordFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';

function harnessFor(status: 'active' | 'paused' | 'dead'): IpcHarness {
    return installIpcHarness({
        list_feeds: [feedFixture({ id: 1, title: 'Alpha', status })],
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        recent_fetches: [
            fetchRecordFixture({ status: 'ok', http_status: 200, articles_new: 3 }),
            fetchRecordFixture({
                status: 'error',
                http_status: 500,
                error: 'server error',
                articles_new: 0,
            }),
        ],
        set_feed_status: null,
    });
}

describe('FeedHealthPanel', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [];
        harness?.teardown();
        harness = null;
    });

    it('renders the feed name and its recent fetches', async () => {
        harness = harnessFor('active');
        const { getByText } = render(FeedHealthPanel, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        expect(getByText('Alpha')).toBeTruthy();
        expect(harness.callsFor('recent_fetches')).toEqual([{ feedId: 1, limit: 20 }]);
        expect(getByText('3 new')).toBeTruthy();
        expect(getByText('server error')).toBeTruthy();
    });

    it('pauses an active feed through set_feed_status', async () => {
        harness = harnessFor('active');
        const { getByText } = render(FeedHealthPanel, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        await fireEvent.click(getByText('Pause'));
        await flushIpc();

        expect(harness.callsFor('set_feed_status')).toEqual([{ feedId: 1, status: 'paused' }]);
    });

    it('offers reactivation for a dead feed', async () => {
        harness = harnessFor('dead');
        const { getByText } = render(FeedHealthPanel, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        await fireEvent.click(getByText('Reactivate'));
        await flushIpc();

        expect(harness.callsFor('set_feed_status')).toEqual([{ feedId: 1, status: 'active' }]);
    });
});
