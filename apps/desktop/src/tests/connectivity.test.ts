/**
 * The "Offline" badge (#123): the Rust scheduler's `ConnectivityChanged`
 * event raises and clears it, and `connectivity_online` seeds it at startup
 * so a badge raised before the webview (re)loaded isn't lost.
 */
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { events } from '$lib/bindings';
import { ConnectivityStore, connectivityStore } from '$lib/state/connectivity.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness, unreadCountsFixture } from './ipc-harness';
import { resetUiStores } from './ui-stores';

describe('connectivity', () => {
    let harness: IpcHarness | null = null;
    const teardowns: Array<() => void> = [];

    afterEach(() => {
        cleanup();
        for (const teardown of teardowns.splice(0)) teardown();
        resetQueryCache();
        resetUiStores();
        feedsStore.reset();
        connectivityStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('follows the scheduler’s events: offline raises it, online clears it', async () => {
        harness = installIpcHarness({ connectivity_online: true });
        const store = new ConnectivityStore();
        teardowns.push(await store.wire());
        expect(store.offline).toBe(false);

        await events.connectivityChanged.emit({ online: false });
        expect(store.offline).toBe(true);

        await events.connectivityChanged.emit({ online: true });
        expect(store.offline).toBe(false);
    });

    it('seeds from the backend, so a badge raised before a reload survives it', async () => {
        harness = installIpcHarness({ connectivity_online: false });
        const store = new ConnectivityStore();
        teardowns.push(await store.wire());
        expect(store.offline).toBe(true);
        expect(harness.callsFor('connectivity_online')).toHaveLength(1);
    });

    it('never lets a slower seed answer overwrite a newer event', async () => {
        let answer: (online: boolean) => void = () => {};
        harness = installIpcHarness({
            connectivity_online: () =>
                new Promise<boolean>((resolve) => {
                    answer = resolve;
                }),
        });
        const store = new ConnectivityStore();
        const wired = store.wire();
        await flushIpc();

        await events.connectivityChanged.emit({ online: false });
        answer(true); // read before the event: stale
        teardowns.push(await wired);
        expect(store.offline).toBe(true);
    });

    it('keeps the current verdict when the seed query fails', async () => {
        harness = installIpcHarness({
            connectivity_online: () => Promise.reject({ kind: 'internal', code: 'INTERNAL' }),
        });
        const store = new ConnectivityStore();
        teardowns.push(await store.wire());
        expect(store.offline).toBe(false);
    });

    it('shows the sidebar badge while offline and removes it once back online', async () => {
        harness = installIpcHarness({
            connectivity_online: true,
            list_feeds: [],
            get_unread_counts: unreadCountsFixture(),
        });
        teardowns.push(await connectivityStore.wire());
        const { getByRole, queryByText } = render(Sidebar);
        await flushIpc();
        expect(queryByText('Offline')).toBeNull();

        await events.connectivityChanged.emit({ online: false });
        await tick();
        const region = getByRole('status');
        expect(region.textContent).toContain('Offline');
        expect(region.textContent).toContain('background refresh is paused');

        await events.connectivityChanged.emit({ online: true });
        await tick();
        expect(queryByText('Offline')).toBeNull();
    });
});
