/**
 * RefreshStatus announces the refresh sweep for screen readers: a polite
 * live region shows "Refreshing feeds" while in flight and a completion
 * summary (new articles across N feeds) when the feeds store settles.
 */
import RefreshStatus from '$components/common/RefreshStatus.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { cleanup, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { refreshOutcomeFixture } from './ipc-harness';

describe('RefreshStatus', () => {
    afterEach(() => {
        cleanup();
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [];
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
        const { getByRole } = render(RefreshStatus);
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [
            refreshOutcomeFixture({ new_articles: 3 }),
            refreshOutcomeFixture({ new_articles: 2 }),
        ];
        await tick();
        expect(getByRole('status').textContent).toContain(
            'Refresh complete: 5 new articles across 2 feeds',
        );
    });
});
