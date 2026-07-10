/**
 * The sidebar over mockIPC: backend-owned unread counts (never client badge
 * math), feed selection re-scoping the backend filters, and the built-in
 * views (all / starred / read later) over those same filters.
 */
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { feedDnd } from '$lib/state/feed-dnd.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { sidebarTreeStore } from '$lib/state/sidebar-tree.svelte';
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
        sidebarTreeStore.reset(); // the cursor now persists across blur (UX), so isolate it here
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
        sidebarTreeStore.reset(); // the cursor now persists across blur (UX), so isolate it here
        uiStore.reset();
        feedsStore.reset();
        menuStore.reset();
        feedDnd.clear();
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
            set_feed_tags: null,
            set_setting: null,
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

    it('Right on a leaf feed drills into the list (selects it + focuses the list)', async () => {
        harness = taggedHarness();
        const { getByRole } = render(Sidebar);
        await flushIpc();

        selectionStore.focusSidebar();
        await tick();
        const tree = getByRole('tree');
        // Walk down to a leaf feed (Tech → Databases → SQLite feed).
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed:Tech/Databases:2');

        // Right at a tree dead-end crosses rightward into the article list.
        await fireEvent.keyDown(tree, { key: 'ArrowRight' });
        expect(selectionStore.selectedFeedId).toBe(2);
        expect(selectionStore.focus).toBe('list');
    });

    it('keeps the cursor across blur so returning to the tree resumes there', async () => {
        harness = taggedHarness();
        const { getByRole } = render(Sidebar);
        await flushIpc();

        selectionStore.focusSidebar();
        await tick();
        const tree = getByRole('tree');
        // The browser fires `focus` when the seat effect focuses the tree
        // (jsdom does not dispatch it on programmatic .focus()).
        await fireEvent.focus(tree);
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed:Tech/Databases:2');
        expect(sidebarTreeStore.focused).toBe(true);

        // Focus leaves the tree: the ring stops (focused=false) but the cursor
        // position is kept, so it isn't forgotten.
        await fireEvent.blur(tree);
        expect(sidebarTreeStore.focused).toBe(false);
        expect(sidebarTreeStore.activeKey).toBe('feed:Tech/Databases:2');

        // Returning to the tree resumes on the same row, not the top.
        selectionStore.focusSidebar();
        await tick();
        await fireEvent.focus(tree);
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed:Tech/Databases:2');
        expect(sidebarTreeStore.focused).toBe(true);
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

    /** The ids of a feed row's "Move to folder" submenu, as built at open time. */
    function moveSubmenuIds(label: string, getByLabelText: (l: string) => HTMLElement): string[] {
        fireEvent.contextMenu(getByLabelText(label));
        const move = menuStore.current?.items.find((item) => item.id === 'move');
        return (move?.children ?? []).map((child) => child.id);
    }

    it('the move-to-folder menu omits the feed’s own folder and offers "New folder…"', async () => {
        harness = taggedHarness();
        const { getByLabelText } = render(Sidebar);
        await flushIpc();

        // Rust Blog sits in Tech: its own folder is not offered, deeper Tech/Databases is,
        // "New folder…" is always present, and Ungroup shows (it is grouped).
        const grouped = moveSubmenuIds('Rust Blog', getByLabelText);
        expect(grouped).not.toContain('move:Tech');
        expect(grouped).toContain('move:Tech/Databases');
        expect(grouped).toContain('new-folder');
        expect(grouped).toContain('ungroup');
    });

    it('an ungrouped feed’s move menu lists every folder and hides the no-op Ungroup', async () => {
        harness = taggedHarness();
        const { getByLabelText } = render(Sidebar);
        await flushIpc();

        // Loose has no folder: every path is a valid target, and Ungroup is a no-op so it's gone.
        const loose = moveSubmenuIds('Loose', getByLabelText);
        expect(loose).toEqual(['move:Tech', 'move:Tech/Databases', 'new-folder']);
    });

    it('Alt+↓ reorders the feed under the cursor within its group', async () => {
        harness = installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1, title: 'Alpha' }),
                feedFixture({ id: 2, title: 'Beta', url: 'https://beta.example/feed' }),
            ],
            get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
            reorder_feeds: null,
        });
        const { getByRole } = render(Sidebar);
        await flushIpc();

        selectionStore.focusSidebar();
        await tick();
        const tree = getByRole('tree');
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed::1');

        // Alt+↓ moves Alpha past Beta — a within-group reorder, not a cursor move.
        await fireEvent.keyDown(tree, { key: 'ArrowDown', altKey: true });
        expect(harness.callsFor('reorder_feeds').at(-1)?.feedIds).toEqual([2, 1]);

        // Move the cursor to the last row; Alt+↓ there falls off the end — a no-op.
        // (The mock emits no FeedsChanged, so the rendered order is still Alpha, Beta.)
        await fireEvent.keyDown(tree, { key: 'ArrowDown' });
        expect(tree.getAttribute('aria-activedescendant')).toBe('feed::2');
        await fireEvent.keyDown(tree, { key: 'ArrowDown', altKey: true });
        expect(harness.callsFor('reorder_feeds')).toHaveLength(1);
    });

    it('dragging a folder-feed onto the ungroup zone removes it from its folder', async () => {
        harness = taggedHarness();
        const { getByText, queryByText } = render(Sidebar);
        await flushIpc();

        // No drag in progress ⇒ the ungroup zone is hidden.
        expect(queryByText('Remove from folder')).toBeNull();

        // Start dragging Rust Blog (in Tech): the pointer-only ungroup target appears.
        feedDnd.start(1);
        await tick();
        const zone = getByText('Remove from folder');

        await fireEvent.drop(zone);
        // ungroupFeed(1) rewrites the feed's tags to none (dropping the Tech path tag).
        expect(harness.callsFor('set_feed_tags').at(-1)).toEqual({ feedId: 1, tags: [] });
    });

    it('does not offer the ungroup zone for an already-ungrouped feed', async () => {
        harness = taggedHarness();
        const { queryByText } = render(Sidebar);
        await flushIpc();

        // Loose has no folder — dragging it out would be a no-op, so no zone.
        feedDnd.start(3);
        await tick();
        expect(queryByText('Remove from folder')).toBeNull();
    });
});
