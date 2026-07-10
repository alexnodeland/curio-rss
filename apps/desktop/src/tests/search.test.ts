/**
 * FTS search: the store (debounce, generation guard, clear), the SearchBar
 * wiring (query, `/`-focus nonce, Escape), the SearchResults pane, and the
 * fact that an active search becomes the list selection navigates over
 * (`articlesStore.activeItems`).
 */
import SearchResults from '$components/articles/SearchResults.svelte';
import SearchBar from '$components/common/SearchBar.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { SEARCH_LIMIT, searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';

function resetAll(harness: IpcHarness | null): void {
    cleanup();
    searchStore.reset();
    articlesStore.reset();
    selectionStore.reset();
    resetQueryCache();
    harness?.teardown();
}

describe('search store', () => {
    let harness: IpcHarness | null = null;
    afterEach(() => {
        resetAll(harness);
        harness = null;
    });

    it('runs search_articles and exposes the backend-ordered results', async () => {
        harness = installIpcHarness({
            search_articles: [articleSummaryFixture({ id: 7, title: 'match' })],
        });
        searchStore.setQuery('rust', 0);
        await flushIpc();

        expect(searchStore.active).toBe(true);
        expect(searchStore.results.map((row) => row.id)).toEqual([7]);
        expect(harness.callsFor('search_articles')[0]).toEqual({
            query: 'rust',
            limit: SEARCH_LIMIT,
        });
    });

    it('an empty query clears the results without a call', async () => {
        harness = installIpcHarness({ search_articles: [articleSummaryFixture()] });
        searchStore.setQuery('rust', 0);
        await flushIpc();
        searchStore.setQuery('  ', 0);
        await flushIpc();
        expect(searchStore.active).toBe(false);
        expect(searchStore.results).toEqual([]);
    });

    it('becomes the active list selection navigates over', async () => {
        harness = installIpcHarness({
            search_articles: [articleSummaryFixture({ id: 7 }), articleSummaryFixture({ id: 8 })],
        });
        searchStore.setQuery('x', 0);
        await flushIpc();

        expect(articlesStore.activeItems.map((row) => row.id)).toEqual([7, 8]);
        selectionStore.selectNextArticle();
        expect(selectionStore.selectedArticleId).toBe(7);
        selectionStore.selectNextArticle();
        expect(selectionStore.selectedArticleId).toBe(8);
    });

    it('is unsettled (pending) during the debounce gap, then settles', async () => {
        vi.useFakeTimers();
        try {
            harness = installIpcHarness({ search_articles: [] });
            searchStore.setQuery('rust');
            // Query text is live but the request has not fired: not settled.
            expect(searchStore.active).toBe(true);
            expect(searchStore.pending).toBe(true);
            expect(searchStore.settled).toBe(false);
            await vi.advanceTimersByTimeAsync(300);
            expect(searchStore.pending).toBe(false);
            expect(searchStore.settled).toBe(true);
        } finally {
            vi.useRealTimers();
        }
    });

    it('debounces a burst of keystrokes into a single query', async () => {
        vi.useFakeTimers();
        try {
            harness = installIpcHarness({ search_articles: [] });
            searchStore.setQuery('r');
            searchStore.setQuery('ru');
            searchStore.setQuery('rus');
            expect(harness.callsFor('search_articles')).toHaveLength(0);
            await vi.advanceTimersByTimeAsync(300);
            const calls = harness.callsFor('search_articles');
            expect(calls).toHaveLength(1);
            expect(calls[0]).toEqual({ query: 'rus', limit: SEARCH_LIMIT });
        } finally {
            vi.useRealTimers();
        }
    });
});

describe('SearchBar', () => {
    let harness: IpcHarness | null = null;
    afterEach(() => {
        resetAll(harness);
        harness = null;
    });

    it('reflects typing into the query', async () => {
        harness = installIpcHarness({ search_articles: [] });
        const { getByRole } = render(SearchBar);
        await fireEvent.input(getByRole('searchbox'), { target: { value: 'zig' } });
        expect(searchStore.query).toBe('zig');
    });

    it('focuses when the focus nonce is bumped (the `/` shortcut)', () => {
        harness = installIpcHarness({ search_articles: [] });
        const { getByRole } = render(SearchBar);
        const input = getByRole('searchbox');
        expect(document.activeElement).not.toBe(input);

        searchStore.requestFocus();
        flushSync();
        expect(document.activeElement).toBe(input);
    });

    it('Escape clears the query', async () => {
        harness = installIpcHarness({ search_articles: [] });
        const { getByRole } = render(SearchBar);
        const input = getByRole('searchbox');
        searchStore.setQuery('rust', 0);
        await flushIpc();
        await fireEvent.keyDown(input, { key: 'Escape' });
        expect(searchStore.query).toBe('');
    });
});

describe('SearchResults', () => {
    let harness: IpcHarness | null = null;
    afterEach(() => {
        resetAll(harness);
        harness = null;
    });

    it('renders result rows and selects on click', async () => {
        harness = installIpcHarness({
            search_articles: [articleSummaryFixture({ id: 7 }), articleSummaryFixture({ id: 8 })],
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        searchStore.setQuery('x', 0);
        await flushIpc();
        const { container } = render(SearchResults);
        await flushIpc();

        const rows = [...container.querySelectorAll('.article-row')];
        expect(rows.length).toBe(2);
        await fireEvent.click(rows[0]);
        expect(selectionStore.selectedArticleId).toBe(7);
    });

    it('shows the empty state when a query matches nothing', async () => {
        harness = installIpcHarness({ search_articles: [], list_feeds: [] });
        searchStore.setQuery('nomatch', 0);
        await flushIpc();
        const { getByText } = render(SearchResults);
        await flushIpc();
        expect(getByText('No matches')).toBeTruthy();
    });

    it('does not flash "No matches" while the query is still pending', () => {
        harness = installIpcHarness({ search_articles: [], list_feeds: [] });
        // A live but not-yet-fired query (debounce gap): pending, no results.
        searchStore.setQuery('rust');
        const { queryByText, getByText } = render(SearchResults);
        expect(queryByText('No matches')).toBeNull();
        expect(getByText('Searching…')).toBeTruthy();
    });
});
