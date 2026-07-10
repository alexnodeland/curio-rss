/**
 * The sidebar over mockIPC: backend-owned unread counts (never client badge
 * math), feed selection re-scoping the backend filters, and the built-in
 * views (all / starred / read later) over those same filters.
 */
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
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
        feedsStore.reset();
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

        await fireEvent.click(getByText('Archived'));
        expect(articlesStore.filters).toEqual({ ...ALL_ARTICLES, archived: true });

        await fireEvent.click(getByText('All articles'));
        expect(articlesStore.filters).toEqual(ALL_ARTICLES);
        expect(articlesStore.filters.archived).toBe(false);
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

describe('Sidebar — folder tree', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        uiStore.reset();
        feedsStore.reset();
        harness?.teardown();
        harness = null;
    });

    function taggedHarness(): IpcHarness {
        return installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1, title: 'Rust Blog', tags: ['Tech'] }),
                feedFixture({
                    id: 2,
                    title: 'SQLite',
                    url: 'https://sqlite.example/feed',
                    tags: ['Tech/Databases'],
                }),
                feedFixture({ id: 3, title: 'Loose', url: 'https://loose.example/feed', tags: [] }),
            ],
            get_unread_counts: unreadCountsFixture({
                total: 15,
                by_feed: [
                    [1, 4],
                    [2, 6],
                    [3, 0],
                ],
            }),
        });
    }

    it('groups tagged feeds into a nested folder tree with roll-up counts', async () => {
        harness = taggedHarness();
        const { getByText } = render(Sidebar);
        await flushIpc();

        expect(getByText('Tech')).toBeTruthy();
        expect(getByText('Databases')).toBeTruthy();
        expect(getByText('Rust Blog')).toBeTruthy();
        expect(getByText('SQLite')).toBeTruthy();
        expect(getByText('Loose')).toBeTruthy();
        // Tech's roll-up = Rust (4) + SQLite (6); All shows the backend total.
        expect(getByText('10')).toBeTruthy();
        expect(getByText('15')).toBeTruthy();
    });

    it('the disclosure collapses a folder, hiding its subtree but not ungrouped feeds', async () => {
        harness = taggedHarness();
        const { getByText, getByLabelText, queryByText } = render(Sidebar);
        await flushIpc();
        expect(getByText('SQLite')).toBeTruthy();

        const disclosure = getByLabelText('Toggle Tech');
        expect(disclosure.getAttribute('aria-expanded')).toBe('true');
        await fireEvent.click(disclosure);

        expect(queryByText('Databases')).toBeNull();
        expect(queryByText('SQLite')).toBeNull();
        expect(queryByText('Rust Blog')).toBeNull();
        // The ungrouped feed and the folder header itself survive.
        expect(getByText('Loose')).toBeTruthy();
        expect(getByLabelText('Toggle Tech').getAttribute('aria-expanded')).toBe('false');

        await fireEvent.click(getByLabelText('Toggle Tech'));
        expect(getByText('SQLite')).toBeTruthy();
    });

    it('clicking a folder name scopes the article list to its feed-tag subtree', async () => {
        harness = taggedHarness();
        const { getByText } = render(Sidebar);
        await flushIpc();

        await fireEvent.click(getByText('Tech'));
        expect(articlesStore.filters.feedTag).toBe('Tech');
        expect(articlesStore.filters.feedId).toBeNull();
        expect(selectionStore.selectedFeedId).toBeNull();
        expect(getByText('Tech').closest('button')?.getAttribute('aria-current')).toBe('true');

        // The nested folder scopes narrower.
        await fireEvent.click(getByText('Databases'));
        expect(articlesStore.filters.feedTag).toBe('Tech/Databases');
    });

    it('g-f focuses the tree; arrows walk the visible rows and Enter selects', async () => {
        harness = taggedHarness();
        const { getByRole } = render(Sidebar);
        await flushIpc();

        // The `view.feeds` shortcut hands the keyboard to the sidebar.
        selectionStore.focusSidebar();
        await tick();

        const tree = getByRole('tree');
        // Cursor seats on the first row (the Tech folder).
        expect(tree.getAttribute('aria-activedescendant')).toBe('folder:Tech');

        // Down walks: Tech → Tech/Databases → SQLite → Rust (DFS render order).
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        expect(tree.getAttribute('aria-activedescendant')).toBe('folder:Tech/Databases');
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed:Tech/Databases:2');
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed:Tech:1');

        // Enter selects the feed under the cursor and hands the keyboard back.
        await fireEvent.keyDown(tree, { key: 'Enter' });
        expect(selectionStore.selectedFeedId).toBe(1);
        expect(selectionStore.focus).toBe('list');
    });

    it('Left collapses the folder under the cursor', async () => {
        harness = taggedHarness();
        const { getByRole, getByLabelText, queryByText } = render(Sidebar);
        await flushIpc();

        selectionStore.focusSidebar();
        await tick();
        const tree = getByRole('tree');
        expect(tree.getAttribute('aria-activedescendant')).toBe('folder:Tech');

        await fireEvent.keyDown(tree, { key: 'ArrowLeft' });
        expect(getByLabelText('Toggle Tech').getAttribute('aria-expanded')).toBe('false');
        expect(queryByText('SQLite')).toBeNull();
    });
});
