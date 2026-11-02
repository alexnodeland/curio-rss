/**
 * Error and empty states in the two "home page" list layouts (#151, #338).
 * The YouTube grid and the Reddit feed are mounted the way the app mounts
 * them — through `ListPane`, with a YouTube- or Reddit-typed feed selected
 * and its home layout switched on — so each case also proves the right
 * layout took the list. A failed first load shows the shared ErrorState
 * whose Retry re-invokes the list query's `refetch()` and recovers in place;
 * an empty feed shows the view-specific `ListEmpty` state, and a failing
 * feed its classified failure with a Retry (a refresh) and its health panel.
 */
import ListPane from '$components/layout/ListPane.svelte';
import type { FeedDto } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    type Responder,
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

interface LayoutCase {
    /** The home type the feed's URL resolves to. */
    readonly type: 'youtube' | 'reddit';
    /** The layout's scroll container — proof the home layout is mounted. */
    readonly scroller: string;
    readonly feedUrl: string;
    readonly articleUrl: string;
}

const LAYOUTS: readonly (readonly [string, LayoutCase])[] = [
    [
        'YouTube grid',
        {
            type: 'youtube',
            scroller: '.yt-scroll',
            feedUrl: 'https://www.youtube.com/feeds/videos.xml?channel_id=UC0000000000000000000000',
            articleUrl: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
        },
    ],
    [
        'Reddit feed',
        {
            type: 'reddit',
            scroller: '.rd-scroll',
            feedUrl: 'https://www.reddit.com/r/rust/.rss',
            articleUrl: 'https://www.reddit.com/r/rust/comments/9/back/',
        },
    ],
];

const FEED_ID = 7;

let harness: IpcHarness | null = null;

function renderPane() {
    return render(ListPane);
}

afterEach(() => {
    cleanup();
    resetQueryCache();
    articlesStore.reset();
    selectionStore.reset();
    searchStore.reset();
    feedsStore.reset();
    resetUiStores();
    // `layoutStore.reset()` leaves the home-section toggles alone.
    layoutStore.homeYoutube = false;
    layoutStore.homeReddit = false;
    harness?.teardown();
    harness = null;
    vi.restoreAllMocks();
});

/**
 * Wires the IPC for one layout case, selects its typed feed with the home
 * layout on, renders the list pane and lets the first loads settle.
 */
async function mountHome(
    layout: LayoutCase,
    feed: FeedDto,
    listArticles: Responder,
): Promise<ReturnType<typeof renderPane>> {
    harness = installIpcHarness({
        list_articles: listArticles,
        list_feeds: [feed],
        search_articles: [],
        get_article_state: articleStateFixture(),
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        refresh_feed: { feed_id: FEED_ID, status: 'ok', articles_new: 0, error: null },
    });
    if (layout.type === 'youtube') {
        layoutStore.homeYoutube = true;
    } else {
        layoutStore.homeReddit = true;
    }
    // In the app the sidebar loads the feed list; the pane reads the selected
    // feed's URL from it to pick the home layout.
    feedsStore.prime();
    await flushIpc();
    selectionStore.selectFeed(FEED_ID);
    const view = renderPane();
    await flushIpc();
    expect(view.container.querySelector(layout.scroller)).not.toBeNull();
    return view;
}

describe.each(LAYOUTS)('%s', (_name, layout) => {
    const typedFeed = (overrides: Partial<FeedDto> = {}): FeedDto =>
        feedFixture({ id: FEED_ID, title: 'Typed feed', url: layout.feedUrl, ...overrides });

    it('a failed first load shows Retry, which re-invokes refetch() and recovers', async () => {
        const { container, getByRole, findByText } = await mountHome(
            layout,
            typedFeed(),
            rejectWith(failure),
        );
        expect(getByRole('alert').textContent).toContain('disk went away');
        expect(container.querySelector('.card')).toBeNull();

        const refetch = vi.spyOn(articlesStore.current.query, 'refetch');
        harness?.respond('list_articles', [
            articleSummaryFixture({
                id: 9,
                feed_id: FEED_ID,
                title: 'Back again',
                source_url: layout.articleUrl,
            }),
        ]);
        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();

        expect(refetch).toHaveBeenCalledOnce();
        expect(await findByText('Back again')).toBeTruthy();
        expect(harness?.callsFor('list_articles')).toHaveLength(2);
        // Recovered in the same layout, as a card — not the row list.
        expect(container.querySelector(`${layout.scroller} .card`)?.textContent).toContain(
            'Back again',
        );
        expect(container.querySelector('[role="alert"]')).toBeNull();
    });

    it("an empty feed shows the feed's own empty state, and Refresh now refreshes it", async () => {
        const { container, getByText, getByRole } = await mountHome(layout, typedFeed(), []);

        expect(getByText('No articles in this feed yet')).toBeTruthy();
        expect(container.querySelector(`${layout.scroller} .empty-state`)).not.toBeNull();

        await fireEvent.click(getByRole('button', { name: 'Refresh now' }));
        await flushIpc();
        expect(harness?.callsFor('refresh_feed')).toEqual([{ feedId: FEED_ID }]);
    });

    it('a failing feed shows its classified failure, a Retry and its health panel', async () => {
        const broken = typedFeed({
            last_error: 'HTTP 410 Gone',
            last_error_kind: 'gone',
        });
        const { getByText, getByRole, queryByText } = await mountHome(layout, broken, []);

        expect(getByText('Curio couldn’t fetch this feed')).toBeTruthy();
        expect(getByText('The site has removed this feed (HTTP 410).')).toBeTruthy();
        expect(queryByText('HTTP 410 Gone')).toBeNull(); // never the engine's text

        await fireEvent.click(getByRole('button', { name: 'Retry' }));
        await flushIpc();
        expect(harness?.callsFor('refresh_feed')).toEqual([{ feedId: FEED_ID }]);

        await fireEvent.click(getByRole('button', { name: 'Feed health' }));
        expect(layoutStore.activeModal).toBe('edit-feed');
        expect(layoutStore.editFeedId).toBe(FEED_ID);
        expect(layoutStore.editFeedSection).toBe('health');
    });
});
