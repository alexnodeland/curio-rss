/**
 * The list-pane header over mockIPC: the unread-only toggle flips the
 * backend-owned `read` filter (server-side, never a client re-filter) and
 * reflects its state via aria-pressed / label swap.
 */
import ListPane from '$components/layout/ListPane.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness, unreadCountsFixture } from './ipc-harness';

function emptyHarness(): IpcHarness {
    return installIpcHarness({
        list_articles: [],
        list_feeds: [],
        search_articles: [],
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
    });
}

describe('ListPane — unread-only toggle', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        searchStore.reset();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('toggles the backend read filter and reflects state on the control', async () => {
        harness = emptyHarness();
        const { getByLabelText, queryByLabelText } = render(ListPane);
        await flushIpc();

        const toggle = getByLabelText('Show unread only');
        expect(toggle.getAttribute('aria-pressed')).toBe('false');
        expect(articlesStore.filters.read).toBeNull();

        await fireEvent.click(toggle);
        expect(articlesStore.filters.read).toBe(false);
        // The label/affordance flips to the "off" action.
        const pressed = getByLabelText('Show all articles');
        expect(pressed.getAttribute('aria-pressed')).toBe('true');
        expect(queryByLabelText('Show unread only')).toBeNull();

        await fireEvent.click(pressed);
        expect(articlesStore.filters.read).toBeNull();
        expect(getByLabelText('Show unread only').getAttribute('aria-pressed')).toBe('false');
    });

    it('is hidden while a search is active (search owns the header)', async () => {
        harness = emptyHarness();
        const { queryByLabelText } = render(ListPane);
        await flushIpc();
        expect(queryByLabelText('Show unread only')).not.toBeNull();

        searchStore.setQuery('rust', 0);
        await flushIpc();
        expect(queryByLabelText('Show unread only')).toBeNull();
        expect(queryByLabelText('Show all articles')).toBeNull();
    });
});
