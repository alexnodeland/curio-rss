/**
 * FTS search: the store (debounce, generation guard, clear), the SearchBar
 * wiring (query, `/`-focus nonce, Escape), the SearchResults pane, and the
 * fact that an active search becomes the list selection navigates over
 * (`articlesStore.activeItems`).
 */
import SearchResults from '$components/articles/SearchResults.svelte';
import SearchBar from '$components/common/SearchBar.svelte';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { EVERYWHERE, SEARCH_LIMIT, searchParams, searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { flushSync } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    feedFixture,
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
            params: {
                query: 'rust',
                limit: SEARCH_LIMIT,
                feed_id: null,
                read: null,
                starred: null,
                read_later: null,
                // The default All view keeps archived articles out.
                archived: false,
                tag: null,
                feed_tag: null,
            },
        });
    });

    it('searching inside a feed passes its feed_id', async () => {
        harness = installIpcHarness({ search_articles: [] });
        articlesStore.filters = { ...ALL_ARTICLES, feedId: 3 };
        searchStore.setQuery('rust', 0);
        await flushIpc();

        const [call] = harness.callsFor('search_articles');
        expect(call).toEqual({ params: searchParams('rust', { ...ALL_ARTICLES, feedId: 3 }) });
        expect((call as { params: { feed_id: number } }).params.feed_id).toBe(3);
    });

    it('is scoped to the folder, tag and built-in view, minus the unread toggle', async () => {
        harness = installIpcHarness({ search_articles: [] });
        articlesStore.filters = {
            ...ALL_ARTICLES,
            read: false,
            starred: true,
            tag: 'later',
            feedTag: 'Tech',
        };
        searchStore.setQuery('rust', 0);
        await flushIpc();

        const [call] = harness.callsFor('search_articles') as { params: Record<string, unknown> }[];
        expect(call.params).toMatchObject({
            read: null,
            starred: true,
            tag: 'later',
            feed_tag: 'Tech',
            archived: false,
        });
    });

    it('search everywhere drops the scope, re-runs, and resets on clear', async () => {
        harness = installIpcHarness({ search_articles: [] });
        articlesStore.filters = { ...ALL_ARTICLES, feedId: 3 };
        searchStore.setQuery('rust', 0);
        await flushIpc();
        searchStore.setEverywhere(true);
        await flushIpc();

        const calls = harness.callsFor('search_articles');
        expect(calls).toHaveLength(2);
        expect(calls[1]).toEqual({ params: searchParams('rust', EVERYWHERE) });

        searchStore.clear();
        expect(searchStore.everywhere).toBe(false);
        searchStore.setQuery('rust', 0);
        await flushIpc();
        expect(harness.callsFor('search_articles')[2]).toEqual({
            params: searchParams('rust', { ...ALL_ARTICLES, feedId: 3 }),
        });
    });

    it('toggling everywhere without a query fires nothing', async () => {
        harness = installIpcHarness({ search_articles: [] });
        searchStore.setEverywhere(true);
        searchStore.setEverywhere(true);
        await flushIpc();
        expect(harness.callsFor('search_articles')).toHaveLength(0);
        expect(searchStore.everywhere).toBe(true);
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
            expect(calls[0]).toEqual({ params: searchParams('rus', searchStore.scope) });
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

    it('names the scope and switches to everywhere', async () => {
        harness = installIpcHarness({
            search_articles: [articleSummaryFixture({ id: 7 }), articleSummaryFixture({ id: 8 })],
            get_article_state: articleStateFixture(),
            list_feeds: [feedFixture({ id: 3, title: 'Lobsters' })],
        });
        articlesStore.filters = { ...ALL_ARTICLES, feedId: 3 };
        searchStore.setQuery('x', 0);
        await flushIpc();
        const { getByRole, getByText } = render(SearchResults);
        await flushIpc();

        expect(getByText('2 results')).toBeTruthy();
        expect(getByRole('group', { name: 'Search in' })).toBeTruthy();
        const here = getByRole('button', { name: 'Lobsters' });
        const everywhere = getByRole('button', { name: 'Everywhere' });
        expect(here.getAttribute('aria-pressed')).toBe('true');
        expect(everywhere.getAttribute('aria-pressed')).toBe('false');

        await fireEvent.click(everywhere);
        await flushIpc();
        expect(searchStore.everywhere).toBe(true);
        expect(here.getAttribute('aria-pressed')).toBe('false');
        expect(everywhere.getAttribute('aria-pressed')).toBe('true');
        const calls = harness.callsFor('search_articles') as { params: { feed_id: unknown } }[];
        expect(calls.at(-1)?.params.feed_id).toBeNull();

        await fireEvent.click(here);
        await flushIpc();
        expect(searchStore.everywhere).toBe(false);
        expect(harness.callsFor('search_articles')).toHaveLength(3);
    });

    it('marks a count that hit the result cap as a lower bound', async () => {
        harness = installIpcHarness({
            search_articles: Array.from({ length: SEARCH_LIMIT }, (_, i) =>
                articleSummaryFixture({ id: i + 1 }),
            ),
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        searchStore.setQuery('x', 0);
        await flushIpc();
        const { getByText, queryByText } = render(SearchResults);
        await flushIpc();
        expect(searchStore.capped).toBe(true);
        expect(getByText(`${SEARCH_LIMIT}+ results`)).toBeTruthy();
        expect(queryByText(`${SEARCH_LIMIT} results`)).toBeNull();
    });

    it('titles the view half of the scope switch with its full name', async () => {
        const long = 'A feed title long enough to be cut off in the scope switch';
        harness = installIpcHarness({
            search_articles: [articleSummaryFixture({ id: 7 })],
            get_article_state: articleStateFixture(),
            list_feeds: [feedFixture({ id: 3, title: long })],
        });
        articlesStore.filters = { ...ALL_ARTICLES, feedId: 3 };
        searchStore.setQuery('x', 0);
        await flushIpc();
        const { getByRole, getByText } = render(SearchResults);
        await flushIpc();
        expect(getByText('1 result')).toBeTruthy();
        expect(searchStore.capped).toBe(false);
        expect(getByRole('button', { name: long }).getAttribute('title')).toBe(long);
    });

    it('names a built-in view when no feed or folder is selected', async () => {
        harness = installIpcHarness({ search_articles: [], list_feeds: [] });
        articlesStore.filters = { ...ALL_ARTICLES, starred: true };
        searchStore.setQuery('x', 0);
        await flushIpc();
        const { getByRole, queryByText } = render(SearchResults);
        await flushIpc();
        expect(getByRole('button', { name: 'Starred' })).toBeTruthy();
        expect(queryByText('0 results')).toBeNull(); // the empty state says it
    });

    it('names a folder or tag scope', async () => {
        harness = installIpcHarness({ search_articles: [], list_feeds: [] });
        articlesStore.filters = { ...ALL_ARTICLES, feedTag: 'Tech/Databases' };
        searchStore.setQuery('x', 0);
        await flushIpc();
        const { getByRole, unmount } = render(SearchResults);
        await flushIpc();
        expect(getByRole('button', { name: 'Tech/Databases' })).toBeTruthy();
        unmount();

        articlesStore.filters = { ...ALL_ARTICLES, tag: 'later' };
        const again = render(SearchResults);
        await flushIpc();
        expect(again.getByRole('button', { name: '#later' })).toBeTruthy();
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
