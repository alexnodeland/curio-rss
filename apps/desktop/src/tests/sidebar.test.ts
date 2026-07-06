/**
 * The sidebar over mockIPC: backend-owned unread counts (never client badge
 * math), feed selection re-scoping the backend filters, and the built-in
 * views (all / starred / read later) over those same filters.
 */
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
    unreadCountsFixture,
} from './ipc-harness';

function twoFeedHarness(): IpcHarness {
    return installIpcHarness({
        list_feeds: [
            feedFixture({ id: 1, title: 'Alpha' }),
            feedFixture({ id: 2, title: 'Beta', url: 'https://beta.example/feed' }),
        ],
        get_unread_counts: unreadCountsFixture({
            total: 7,
            by_feed: [
                [1, 4],
                [2, 3],
            ],
        }),
    });
}

describe('Sidebar', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        // Unmount first: a mounted tree re-creates queries on cache reset.
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('renders feeds with backend unread counts and the total on All articles', async () => {
        harness = twoFeedHarness();
        const { getByText } = render(Sidebar);
        await flushIpc();

        expect(getByText('Alpha')).toBeTruthy();
        expect(getByText('Beta')).toBeTruthy();
        expect(getByText('4')).toBeTruthy(); // Alpha's unread badge
        expect(getByText('3')).toBeTruthy(); // Beta's unread badge
        expect(getByText('7')).toBeTruthy(); // backend total on All articles
    });

    it('clicking a feed scopes the backend filters to it (no client re-filter)', async () => {
        harness = twoFeedHarness();
        const { getByText } = render(Sidebar);
        await flushIpc();

        await fireEvent.click(getByText('Alpha'));
        expect(selectionStore.selectedFeedId).toBe(1);
        expect(articlesStore.filters.feedId).toBe(1);
        expect(selectionStore.selectedArticleId).toBeNull();
        const button = getByText('Alpha').closest('button');
        expect(button?.getAttribute('aria-current')).toBe('true');
    });

    it('built-in views swap the backend filter set', async () => {
        harness = twoFeedHarness();
        const { getByText } = render(Sidebar);
        await flushIpc();

        await fireEvent.click(getByText('Starred'));
        expect(articlesStore.filters).toEqual({ ...ALL_ARTICLES, starred: true });
        expect(getByText('Starred').closest('button')?.getAttribute('aria-current')).toBe('true');

        await fireEvent.click(getByText('Read later'));
        expect(articlesStore.filters).toEqual({ ...ALL_ARTICLES, readLater: true });

        await fireEvent.click(getByText('All articles'));
        expect(articlesStore.filters).toEqual(ALL_ARTICLES);
        expect(selectionStore.selectedFeedId).toBeNull();
    });

    it('view selection clears a prior feed scope', async () => {
        harness = twoFeedHarness();
        const { getByText } = render(Sidebar);
        await flushIpc();

        await fireEvent.click(getByText('Beta'));
        expect(articlesStore.filters.feedId).toBe(2);
        await fireEvent.click(getByText('Starred'));
        expect(articlesStore.filters.feedId).toBeNull();
        expect(selectionStore.selectedFeedId).toBeNull();
    });

    it('surfaces feed-list failures', async () => {
        harness = installIpcHarness({
            list_feeds: rejectWith(commandErrorFixture({ kind: 'user', message: 'db locked' })),
            get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        });
        const { getByRole } = render(Sidebar);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('db locked');
    });
});
