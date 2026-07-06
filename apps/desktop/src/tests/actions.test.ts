/**
 * The action layer over mockIPC: toggles read the authoritative backend
 * flag before writing the flip, failures land in toasts (user tier
 * verbatim, internal generic), view selection swaps the backend filter
 * set, and unwired registry ids are deliberate no-ops.
 */
import {
    activeView,
    handleShortcut,
    openInBrowser,
    selectView,
    toggleArchived,
    toggleStar,
} from '$lib/state/actions';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    commandErrorFixture,
    installIpcHarness,
    refreshOutcomeFixture,
    rejectWith,
} from './ipc-harness';

describe('actions — toggles', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('toggleStar writes the negation of the backend flag', async () => {
        harness = installIpcHarness({
            get_article_state: articleStateFixture({ starred: true }),
            set_starred: true,
        });
        await toggleStar(7);
        expect(harness.callsFor('set_starred')).toEqual([{ articleId: 7, starred: false }]);
    });

    it('toggleArchived writes the negation of the backend flag', async () => {
        harness = installIpcHarness({
            get_article_state: articleStateFixture({ archived: false }),
            set_archived: true,
        });
        await toggleArchived(7);
        expect(harness.callsFor('set_archived')).toEqual([{ articleId: 7, archived: true }]);
    });

    it('a user-tier failure surfaces verbatim as a toast and stops the flip', async () => {
        harness = installIpcHarness({
            get_article_state: rejectWith(
                commandErrorFixture({ kind: 'user', message: 'no such article' }),
            ),
            set_starred: true,
        });
        await toggleStar(7);
        expect(harness.callsFor('set_starred')).toHaveLength(0);
        expect(uiStore.toasts).toHaveLength(1);
        expect(uiStore.toasts[0]?.message).toBe('no such article');
        expect(uiStore.toasts[0]?.tone).toBe('error');
    });

    it('a thrown (non-contract) failure becomes the generic internal toast', async () => {
        harness = installIpcHarness({}); // no responder → the harness throws
        await toggleStar(7);
        expect(uiStore.toasts).toHaveLength(1);
        expect(uiStore.toasts[0]?.message).toBe('Something went wrong. Retrying may help.');
    });

    it('openInBrowser refuses non-http(s) URLs outright — no open, no event', async () => {
        harness = installIpcHarness({
            'plugin:opener|open_url': null,
            record_opened: null,
        });
        await openInBrowser(7, 'file:///etc/passwd');
        expect(harness.calls).toHaveLength(0);
    });
});

describe('actions — views and shortcut routing', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        uiStore.reset();
        searchStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('selectView swaps the backend filter set and clears selection', () => {
        selectionStore.selectedFeedId = 3;
        selectionStore.selectedArticleId = 9;
        selectView('starred');
        expect(articlesStore.filters).toEqual({ ...ALL_ARTICLES, starred: true });
        expect(selectionStore.selectedFeedId).toBeNull();
        expect(selectionStore.selectedArticleId).toBeNull();
    });

    it('activeView recognizes exactly the built-in filter sets', () => {
        expect(activeView(ALL_ARTICLES)).toBe('all');
        expect(activeView({ ...ALL_ARTICLES, starred: true })).toBe('starred');
        expect(activeView({ ...ALL_ARTICLES, readLater: true })).toBe('readLater');
        expect(activeView({ ...ALL_ARTICLES, feedId: 3 })).toBeNull();
        expect(activeView({ ...ALL_ARTICLES, read: false })).toBeNull();
    });

    it('the g-chord view shortcuts swap the backend filter set', () => {
        harness = installIpcHarness({});
        handleShortcut('view.starred');
        expect(activeView(articlesStore.filters)).toBe('starred');
        handleShortcut('view.readLater');
        expect(activeView(articlesStore.filters)).toBe('readLater');
        handleShortcut('view.all');
        expect(activeView(articlesStore.filters)).toBe('all');
    });

    it('a view switch leaves search mode', () => {
        harness = installIpcHarness({});
        searchStore.setQuery('rust', 0);
        expect(searchStore.active).toBe(true);
        handleShortcut('view.starred');
        expect(searchStore.active).toBe(false);
    });

    it('search.focus bumps the search focus nonce (drives the `/` shortcut)', () => {
        harness = installIpcHarness({});
        const before = searchStore.focusNonce;
        handleShortcut('search.focus');
        expect(searchStore.focusNonce).toBe(before + 1);
        expect(harness.calls).toHaveLength(0);
    });

    it('view.feeds moves focus to the sidebar and clears search', () => {
        harness = installIpcHarness({});
        searchStore.setQuery('rust', 0);
        handleShortcut('view.feeds');
        expect(selectionStore.focus).toBe('sidebar');
        expect(searchStore.active).toBe(false);
    });

    it('app.refreshAll kicks off a full sweep', async () => {
        harness = installIpcHarness({ refresh_all: [] });
        handleShortcut('app.refreshAll');
        await Promise.resolve();
        expect(harness.callsFor('refresh_all')).toHaveLength(1);
    });

    it('feed.refresh refreshes the selected feed, and no-ops with none selected', async () => {
        harness = installIpcHarness({ refresh_feed: refreshOutcomeFixture() });
        handleShortcut('feed.refresh');
        await Promise.resolve();
        expect(harness.callsFor('refresh_feed')).toHaveLength(0);

        selectionStore.selectedFeedId = 3;
        handleShortcut('feed.refresh');
        await Promise.resolve();
        expect(harness.callsFor('refresh_feed')).toEqual([{ feedId: 3 }]);
    });

    it('shortcut actions on an empty selection are no-ops', () => {
        harness = installIpcHarness({});
        handleShortcut('article.open');
        handleShortcut('article.toggleStar');
        handleShortcut('article.toggleRead');
        handleShortcut('article.toggleReadLater');
        expect(harness.calls).toHaveLength(0);
    });
});
