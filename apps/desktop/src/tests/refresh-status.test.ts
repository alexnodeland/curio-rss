/**
 * RefreshStatus announces the refresh sweep for screen readers: a polite
 * live region says "Refreshing feeds" while in flight and a completion
 * summary (new articles across N feeds) when the feeds store settles. The
 * visible pill carries the sweep's running count and a Stop button that
 * cancels it.
 */
import RefreshStatus from '$components/common/RefreshStatus.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness, refreshOutcomeFixture } from './ipc-harness';

describe('RefreshStatus', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        harness?.teardown();
        harness = null;
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [];
        feedsStore.refreshProgress = null;
        feedsStore.refreshCancelled = false;
        feedsStore.cancellingRefresh = false;
    });

    it('announces an in-flight refresh in a polite live region', async () => {
        feedsStore.refreshing = true;
        const { getByRole } = render(RefreshStatus);
        await tick();
        const region = getByRole('status');
        expect(region.getAttribute('aria-live')).toBe('polite');
        expect(region.textContent).toContain('Refreshing feeds');
    });

    it('announces the completion summary when the sweep settles', async () => {
        const { getByRole, queryByRole } = render(RefreshStatus);
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [
            refreshOutcomeFixture({ new_articles: 3 }),
            refreshOutcomeFixture({ new_articles: 2 }),
        ];
        await tick();
        expect(getByRole('status').textContent).toContain(
            'Refresh complete: 5 new articles across 2 feeds',
        );
        expect(queryByRole('button')).toBeNull();
    });

    it('shows the running count once the first feed is in, without announcing each one', async () => {
        feedsStore.refreshing = true;
        feedsStore.refreshProgress = { sweepId: 1, done: 12, total: 300 };
        const { container, getByRole } = render(RefreshStatus);
        await tick();
        expect(container.querySelector('.message')?.textContent).toBe('Refreshing… 12 of 300');
        expect(getByRole('status').textContent).toBe('Refreshing feeds');
        const bar = container.querySelector<HTMLElement>('.bar');
        expect(bar?.style.transform).toBe('scaleX(0.04)');
    });

    it('stops the sweep from its Stop button', async () => {
        harness = installIpcHarness({ cancel_refresh: true });
        feedsStore.refreshing = true;
        feedsStore.refreshProgress = { sweepId: 1, done: 3, total: 10 };
        const { container, getByRole } = render(RefreshStatus);
        await tick();

        const stop = getByRole('button', { name: 'Stop refreshing' });
        expect(stop.textContent).toBe('Stop');
        await fireEvent.click(stop);
        await flushIpc();
        await tick();

        expect(harness.callsFor('cancel_refresh')).toHaveLength(1);
        expect((stop as HTMLButtonElement).disabled).toBe(true);
        expect(container.querySelector('.message')?.textContent).toBe('Stopping…');
        expect(getByRole('status').textContent).toBe('Stopping…');
    });

    it('summarises a stopped sweep as stopped', async () => {
        const { getByRole } = render(RefreshStatus);
        feedsStore.refreshCancelled = true;
        feedsStore.refreshOutcomes = [refreshOutcomeFixture({ new_articles: 1 })];
        await tick();
        expect(getByRole('status').textContent).toBe('Refresh stopped: 1 new article from 1 feed');
    });
});
