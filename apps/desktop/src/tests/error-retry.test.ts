/**
 * Retryable error states and view-specific empty states (#151). Every pane
 * whose load failed offers a Retry that re-invokes its query's `refetch()`
 * (or the search's re-run) and recovers in place; a refetch-failure banner
 * gets one too. An empty view offers what it can: "Show read" when unread-only
 * hides everything, and a failing feed shows its classified failure with a
 * Retry (a refresh of that feed) and its health panel.
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import SearchResults from '$components/articles/SearchResults.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { events } from '$lib/bindings';
import { selectView } from '$lib/state/actions';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { articlePrefetcher } from '$lib/state/prefetch';
import {
    ensureQuery,
    queryKeys,
    resetQueryCache,
    wireInvalidation,
} from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const failure = commandErrorFixture({ kind: 'user', code: 'IO', message: 'disk went away' });

describe('Retry on error states', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        searchStore.reset();
        articlePrefetcher.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
        vi.restoreAllMocks();
    });

    it('the article list first-load Retry re-invokes refetch() and recovers', async () => {
        harness = installIpcHarness({
            list_articles: rejectWith(failure),
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        const { getByRole, findByText } = render(ArticleList);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('disk went away');

        const refetch = vi.spyOn(articlesStore.current.query, 'refetch');
        harness.respond('list_articles', [articleSummaryFixture({ id: 3, title: 'Back again' })]);
        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();

        expect(refetch).toHaveBeenCalledOnce();
        expect(await findByText('Back again')).toBeTruthy();
        expect(harness.callsFor('list_articles')).toHaveLength(2);
    });

    it('the list refetch banner keeps its rows and offers a Retry', async () => {
        harness = installIpcHarness({
            list_articles: [articleSummaryFixture({ id: 3, title: 'Kept row' })],
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        const unwire = await wireInvalidation();
        const { getByRole, getByText, queryByRole } = render(ArticleList);
        await flushIpc();

        harness.respond('list_articles', rejectWith(failure));
        await events.articlesChanged.emit({ feed_id: null });
        await flushIpc();
        expect(getByText('Kept row')).toBeTruthy();

        const refetch = vi.spyOn(articlesStore.current.query, 'refetch');
        harness.respond('list_articles', [articleSummaryFixture({ id: 3, title: 'Kept row' })]);
        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();
        expect(refetch).toHaveBeenCalledOnce();
        expect(queryByRole('alert')).toBeNull();
        unwire();
    });

    it('the reader load Retry re-invokes refetch() and renders the article', async () => {
        harness = installIpcHarness({
            get_article: rejectWith(failure),
            get_article_state: articleStateFixture(),
            mark_read: true,
            list_feeds: [],
        });
        selectionStore.selectedArticleId = 100;
        const { getByRole, findByText } = render(ReaderPane);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('disk went away');

        const query = ensureQuery(queryKeys.article(100), () =>
            Promise.reject(new Error('unused')),
        );
        const refetch = vi.spyOn(query, 'refetch');
        harness.respond('get_article', articleFixture({ id: 100, title: 'Loaded at last' }));
        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();

        expect(refetch).toHaveBeenCalledOnce();
        expect(await findByText('Loaded at last')).toBeTruthy();
    });

    it('the sidebar subscription-list Retry reloads the feeds', async () => {
        harness = installIpcHarness({
            list_feeds: rejectWith(failure),
            get_unread_counts: unreadCountsFixture(),
        });
        const { getByRole, findByText } = render(Sidebar);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('disk went away');

        const query = ensureQuery(queryKeys.feeds, () => Promise.reject(new Error('unused')));
        const refetch = vi.spyOn(query, 'refetch');
        harness.respond('list_feeds', [feedFixture({ id: 1, title: 'Alpha' })]);
        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();

        expect(refetch).toHaveBeenCalledOnce();
        expect(await findByText('Alpha')).toBeTruthy();
    });

    it('the search Retry re-runs the same query', async () => {
        harness = installIpcHarness({
            search_articles: rejectWith(failure),
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        searchStore.setQuery('rust', 0);
        const { getByRole, findByText } = render(SearchResults);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('disk went away');

        harness.respond('search_articles', [
            articleSummaryFixture({ id: 5, title: 'Found again' }),
        ]);
        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();

        expect(harness.callsFor('search_articles').map((call) => call.query)).toEqual([
            'rust',
            'rust',
        ]);
        expect(await findByText('Found again')).toBeTruthy();
    });
});

describe('view-specific empty states', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('Starred explains how to star, naming the bound key', async () => {
        harness = installIpcHarness({ list_articles: [], list_feeds: [] });
        selectView('starred');
        const { getByText, container } = render(ArticleList);
        await flushIpc();
        expect(getByText('No starred articles')).toBeTruthy();
        expect(container.querySelector('.empty-hint')?.textContent).toBe(
            'Press s on an article to star it and keep it here.',
        );
        expect(container.querySelector('.empty-hint kbd')?.textContent).toBe('s');
    });

    it('an unread-only view is all caught up, and "Show read" brings the read rows back', async () => {
        harness = installIpcHarness({ list_articles: [], list_feeds: [] });
        articlesStore.filters = { ...ALL_ARTICLES, read: false };
        const { getByText, getByRole } = render(ArticleList);
        await flushIpc();
        expect(getByText('You’re all caught up')).toBeTruthy();

        await fireEvent.click(getByRole('button', { name: 'Show read articles' }));
        expect(articlesStore.filters.read).toBeNull();
    });

    it('a failing feed shows its classified failure, a Retry and its health panel', async () => {
        const broken = feedFixture({
            id: 4,
            url: 'https://gone.example.org/feed',
            last_error: 'HTTP 410 Gone',
            last_error_kind: 'gone',
        });
        harness = installIpcHarness({
            list_articles: [],
            list_feeds: [broken],
            refresh_feed: { feed_id: 4, status: 'ok', articles_new: 0, error: null },
        });
        selectionStore.selectFeed(4);
        const { getByText, getByRole, queryByText } = render(ArticleList);
        await flushIpc();

        expect(getByText('Curio couldn’t fetch this feed')).toBeTruthy();
        expect(getByText('The site has removed this feed (HTTP 410).')).toBeTruthy();
        expect(queryByText('HTTP 410 Gone')).toBeNull(); // never the engine's text

        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();
        expect(harness.callsFor('refresh_feed')).toEqual([{ feedId: 4 }]);

        await fireEvent.click(getByRole('button', { name: 'Feed health' }));
        expect(layoutStore.activeModal).toBe('edit-feed');
        expect(layoutStore.editFeedId).toBe(4);
        expect(layoutStore.editFeedSection).toBe('health');
    });
});
