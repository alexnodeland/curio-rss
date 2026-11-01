/**
 * RefreshStatus announces the refresh sweep for screen readers: a polite
 * live region says "Refreshing feeds" while in flight and a completion
 * summary (new articles across N feeds) when the feeds store settles. The
 * visible pill carries the sweep's running count and a Stop button that
 * cancels it.
 */
import RefreshStatus from '$components/common/RefreshStatus.svelte';
import { REFRESH_CONTROL_ID, type RefreshSweepProgress, feedsStore } from '$lib/state/feeds.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness, refreshOutcomeFixture } from './ipc-harness';

/** A sweep's running count, no full-text pages unless given. */
function progressOf(fields: Partial<RefreshSweepProgress>): RefreshSweepProgress {
    return { sweepId: 1, done: 0, total: 0, pagesDone: 0, pagesTotal: 0, ...fields };
}

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
        feedsStore.refreshProgress = progressOf({ done: 12, total: 300 });
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
        feedsStore.refreshProgress = progressOf({ done: 3, total: 10 });
        const { container, getByRole } = render(RefreshStatus);
        await tick();

        const stop = getByRole('button', { name: 'Stop refreshing' });
        expect(stop.textContent).toBe('Stop');
        await fireEvent.click(stop);
        await flushIpc();
        await tick();

        expect(harness.callsFor('cancel_refresh')).toHaveLength(1);
        // Marked, not disabled: a disabled button would drop keyboard focus.
        expect(stop.getAttribute('aria-disabled')).toBe('true');
        expect(container.querySelector('.message')?.textContent).toBe('Stopping…');
        expect(getByRole('status').textContent).toBe('Stopping…');
        // A second press while the stop lands asks for nothing more.
        await fireEvent.click(stop);
        await flushIpc();
        expect(harness.callsFor('cancel_refresh')).toHaveLength(1);
    });

    it('counts the full-text pages once every feed is in', async () => {
        feedsStore.refreshing = true;
        feedsStore.refreshProgress = progressOf({
            done: 300,
            total: 300,
            pagesDone: 3,
            pagesTotal: 12,
        });
        const { container } = render(RefreshStatus);
        await tick();
        expect(container.querySelector('.message')?.textContent).toBe('Loading full text… 3 of 12');
        const bar = container.querySelector<HTMLElement>('.bar');
        expect(bar?.style.transform).toBe('scaleX(0.25)');

        // Feeds still refreshing: the feed count stands, pages or not.
        feedsStore.refreshProgress = progressOf({
            done: 299,
            total: 300,
            pagesDone: 3,
            pagesTotal: 12,
        });
        await tick();
        expect(container.querySelector('.message')?.textContent).toBe('Refreshing… 299 of 300');
    });

    it('hands focus back to the refresh control when a stopped sweep settles', async () => {
        harness = installIpcHarness({ cancel_refresh: true });
        const control = document.createElement('button');
        control.id = REFRESH_CONTROL_ID;
        document.body.append(control);
        try {
            feedsStore.refreshing = true;
            feedsStore.refreshProgress = progressOf({ done: 3, total: 10 });
            const { getByRole, queryByRole } = render(RefreshStatus);
            await tick();
            const stop = getByRole('button', { name: 'Stop refreshing' });
            stop.focus();
            await fireEvent.click(stop);
            await flushIpc();
            expect(document.activeElement).toBe(stop);

            feedsStore.refreshing = false;
            feedsStore.cancellingRefresh = false;
            feedsStore.refreshCancelled = true;
            await tick();
            await tick();
            expect(queryByRole('button', { name: 'Stop refreshing' })).toBeNull();
            expect(document.activeElement).toBe(control);
        } finally {
            control.remove();
        }
    });

    it('leaves focus alone when the Stop button did not hold it', async () => {
        const control = document.createElement('button');
        control.id = REFRESH_CONTROL_ID;
        document.body.append(control);
        try {
            feedsStore.refreshing = true;
            feedsStore.refreshProgress = progressOf({ done: 3, total: 10 });
            render(RefreshStatus);
            await tick();
            feedsStore.refreshing = false;
            await tick();
            await tick();
            expect(document.activeElement).not.toBe(control);
        } finally {
            control.remove();
        }
    });

    it('summarises a stopped sweep as stopped', async () => {
        const { getByRole } = render(RefreshStatus);
        feedsStore.refreshCancelled = true;
        feedsStore.refreshOutcomes = [refreshOutcomeFixture({ new_articles: 1 })];
        await tick();
        expect(getByRole('status').textContent).toBe('Refresh stopped: 1 new article from 1 feed');
    });
});
