/**
 * The action layer over mockIPC: toggles read the authoritative backend
 * flag before writing the flip, failures land in toasts (user tier
 * verbatim, internal generic), view selection swaps the backend filter
 * set, and unwired registry ids are deliberate no-ops.
 */
import type { ListArticlesDto } from '$lib/bindings';
import {
    activeView,
    goToNextUnread,
    handleShortcut,
    importFromFile,
    markViewRead,
    openInBrowser,
    pageReader,
    registerReaderBody,
    routeMenuAction,
    selectFolder,
    selectView,
    toggleArchived,
    toggleStar,
} from '$lib/state/actions';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { searchStore } from '$lib/state/search.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
    feedFixture,
    flushIpc,
    importOutcomeFixture,
    installIpcHarness,
    refreshOutcomeFixture,
    rejectWith,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

describe('actions — toggles', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
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

    it('serializes rapid presses so the second alternates instead of being dropped', async () => {
        // Two fast presses off the same backend flag: a naive read-then-write
        // has both read `false` and both write `true`. Serialized, the second
        // sees the first's written value and writes its negation.
        harness = installIpcHarness({
            get_article_state: articleStateFixture({ starred: false }),
            set_starred: true,
        });
        await Promise.all([toggleStar(7), toggleStar(7)]);
        expect(harness.callsFor('set_starred')).toEqual([
            { articleId: 7, starred: true },
            { articleId: 7, starred: false },
        ]);
        // The backend flag was read once; the follow-up alternated from cache.
        expect(harness.callsFor('get_article_state')).toHaveLength(1);
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
        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.message).toBe('no such article');
        expect(toastStore.toasts[0]?.tone).toBe('error');
    });

    it('a thrown (non-contract) failure becomes the generic internal toast', async () => {
        harness = installIpcHarness({}); // no responder → the harness throws
        await toggleStar(7);
        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.message).toBe('Something went wrong. Retrying may help.');
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
        resetUiStores();
        searchStore.reset();
        feedsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('goToNextUnread selects the next unread without rewriting the visible view', async () => {
        harness = installIpcHarness({ list_articles: [articleSummaryFixture({ id: 42 })] });
        selectionStore.selectedArticleId = 100;
        await goToNextUnread();

        expect(selectionStore.selectedArticleId).toBe(42);
        // The displayed filter is left alone — no silent switch to unread-only.
        expect(articlesStore.filters.read).toBeNull();
        // The query still scopes to unread rows below the selection.
        const sent = harness.callsFor('list_articles')[0].params as ListArticlesDto;
        expect(sent.read).toBe(false);
        expect(sent.before).toBe(100);
        expect(sent.limit).toBe(1);
    });

    it('goToNextUnread toasts on the dead end when nothing unread remains', async () => {
        harness = installIpcHarness({ list_articles: [] });
        selectionStore.selectedArticleId = 100;
        await goToNextUnread();

        expect(selectionStore.selectedArticleId).toBe(100);
        expect(toastStore.toasts.some((toast) => toast.message === 'No more unread')).toBe(true);
    });

    it('goToNextUnread hops to the next feed with unread when the scope is dry', async () => {
        harness = installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1, title: 'A' }),
                feedFixture({ id: 2, title: 'B', url: 'https://b.example/feed' }),
            ],
            get_unread_counts: unreadCountsFixture({ total: 3, by_feed: [[2, 3]] }),
            list_articles: (args) => {
                const params = args.params as ListArticlesDto;
                return params.feed_id === 2 ? [articleSummaryFixture({ id: 99 })] : [];
            },
        });
        feedsStore.prime();
        await flushIpc();

        await goToNextUnread();
        expect(selectionStore.selectedFeedId).toBe(2);
        expect(articlesStore.filters.feedId).toBe(2);
        // The hopped-to feed shows all its rows — the unread one is present
        // without forcing an unread-only view.
        expect(articlesStore.filters.read).toBeNull();
        expect(selectionStore.selectedArticleId).toBe(99);
    });

    it('selectView swaps the backend filter set and clears selection', () => {
        selectionStore.selectedFeedId = 3;
        selectionStore.selectedArticleId = 9;
        selectView('starred');
        expect(articlesStore.filters).toEqual({ ...ALL_ARTICLES, starred: true });
        expect(selectionStore.selectedFeedId).toBeNull();
        expect(selectionStore.selectedArticleId).toBeNull();
    });

    it('selectFolder scopes the filters to a feed-tag and clears feed + selection', () => {
        harness = installIpcHarness({});
        selectionStore.selectedFeedId = 5;
        selectionStore.selectedArticleId = 9;
        selectFolder('Tech/Databases');
        expect(articlesStore.filters).toEqual({ ...ALL_ARTICLES, feedTag: 'Tech/Databases' });
        expect(selectionStore.selectedFeedId).toBeNull();
        expect(selectionStore.selectedArticleId).toBeNull();
    });

    it('activeView recognizes exactly the built-in filter sets', () => {
        expect(activeView(ALL_ARTICLES)).toBe('all');
        expect(activeView({ ...ALL_ARTICLES, starred: true })).toBe('starred');
        expect(activeView({ ...ALL_ARTICLES, readLater: true })).toBe('readLater');
        expect(activeView({ ...ALL_ARTICLES, archived: true })).toBe('archived');
        expect(activeView({ ...ALL_ARTICLES, feedId: 3 })).toBeNull();
        expect(activeView({ ...ALL_ARTICLES, read: false })).toBeNull();
    });

    it('the g-chord view shortcuts swap the backend filter set', () => {
        harness = installIpcHarness({});
        handleShortcut('view.starred');
        expect(activeView(articlesStore.filters)).toBe('starred');
        handleShortcut('view.readLater');
        expect(activeView(articlesStore.filters)).toBe('readLater');
        handleShortcut('view.archived');
        expect(activeView(articlesStore.filters)).toBe('archived');
        expect(articlesStore.filters.archived).toBe(true);
        handleShortcut('view.all');
        expect(activeView(articlesStore.filters)).toBe('all');
        expect(articlesStore.filters.archived).toBe(false);
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

    it('article.open skips a link-less entry rather than opening the feed XML', async () => {
        const feedUrl = 'https://example.org/feed.xml';
        harness = installIpcHarness({
            list_feeds: [feedFixture({ id: 1, url: feedUrl })],
            list_articles: [
                articleSummaryFixture({ id: 1, feed_id: 1, source_url: feedUrl }),
                articleSummaryFixture({ id: 2, feed_id: 1 }),
            ],
            record_opened: null,
            'plugin:opener|open_url': null,
        });
        feedsStore.prime();
        await articlesStore.current.loadMore();
        await flushIpc();

        selectionStore.selectedArticleId = 1;
        handleShortcut('article.open');
        await flushIpc();
        expect(harness.callsFor('plugin:opener|open_url')).toHaveLength(0);
        expect(harness.callsFor('record_opened')).toHaveLength(0);

        // A linked entry in the same feed still opens.
        selectionStore.selectedArticleId = 2;
        handleShortcut('article.open');
        await flushIpc();
        expect(harness.callsFor('plugin:opener|open_url')).toHaveLength(1);
    });

    it('routeMenuAction runs a shortcut id through the action layer', () => {
        harness = installIpcHarness({});
        routeMenuAction('app.settings');
        expect(layoutStore.activeModal).toBe('settings');
        routeMenuAction('view.starred');
        expect(activeView(articlesStore.filters)).toBe('starred');
    });

    it('routeMenuAction opens the docs / issue tracker for the menu-only ids', async () => {
        harness = installIpcHarness({ 'plugin:opener|open_url': null });
        routeMenuAction('menu.docs');
        routeMenuAction('menu.reportIssue');
        await flushIpc();
        expect(harness.callsFor('plugin:opener|open_url')).toHaveLength(2);
    });

    it('routeMenuAction ignores predefined / unknown menu ids', () => {
        harness = installIpcHarness({});
        routeMenuAction('copy');
        routeMenuAction('quit');
        expect(layoutStore.activeModal).toBeNull();
        expect(harness.calls).toHaveLength(0);
    });
});

describe('actions — mark the current view read (Shift+A)', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        searchStore.reset();
        feedsStore.reset();
        harness?.teardown();
        harness = null;
    });

    function lastToast(): string | undefined {
        return toastStore.toasts[toastStore.toasts.length - 1]?.message;
    }

    it('a feed view rides the bulk command for that feed', async () => {
        harness = installIpcHarness({ mark_all_read: [1, 2, 3, 4] });
        articlesStore.filters = { ...ALL_ARTICLES, feedId: 3 };
        await markViewRead();
        expect(harness.callsFor('mark_all_read')).toEqual([{ feedId: 3 }]);
        expect(lastToast()).toBe('Marked 4 read');
    });

    it('a folder view bulk-marks exactly its feeds and sums the count', async () => {
        harness = installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1, tags: ['Tech'] }),
                feedFixture({ id: 2, url: 'https://b.example/feed', tags: ['Tech/Rust'] }),
                feedFixture({ id: 3, url: 'https://c.example/feed', tags: ['News'] }),
            ],
            get_unread_counts: unreadCountsFixture(),
            mark_all_read: (args) => (args.feedId === 1 ? [1, 2] : [3, 4, 5, 6, 7]),
        });
        feedsStore.prime();
        await flushIpc();
        articlesStore.filters = { ...ALL_ARTICLES, feedTag: 'Tech' };

        await markViewRead();
        expect(harness.callsFor('mark_all_read')).toEqual([{ feedId: 1 }, { feedId: 2 }]);
        expect(lastToast()).toBe('Marked 7 read');
    });

    it('a narrowed view (Starred) marks only its own unread rows, paged', async () => {
        const page = (top: number, count: number) =>
            Array.from({ length: count }, (_, index) => articleSummaryFixture({ id: top - index }));
        harness = installIpcHarness({
            list_articles: (args) => {
                const params = args.params as ListArticlesDto;
                return params.before === null ? page(1000, 200) : page(800, 3);
            },
            mark_read: (args) => (args.articleId as number) !== 799, // one already read
        });
        selectView('starred');

        await markViewRead();
        const listings = harness
            .callsFor('list_articles')
            .map((call) => call.params as ListArticlesDto);
        expect(listings).toHaveLength(2);
        expect(listings[0]).toMatchObject({ starred: true, read: false, before: null, limit: 200 });
        expect(listings[1]).toMatchObject({ starred: true, read: false, before: 801 });
        expect(harness.callsFor('mark_all_read')).toHaveLength(0);
        expect(harness.callsFor('mark_read')).toHaveLength(203);
        expect(lastToast()).toBe('Marked 202 read');
    });

    it('search results mark exactly the listed rows', async () => {
        harness = installIpcHarness({ mark_read: true });
        searchStore.setQuery('rust', 0);
        searchStore.results = [articleSummaryFixture({ id: 5 }), articleSummaryFixture({ id: 6 })];

        await markViewRead();
        expect(harness.callsFor('mark_read')).toEqual([
            { articleId: 5, read: true },
            { articleId: 6, read: true },
        ]);
        expect(harness.callsFor('mark_all_read')).toHaveLength(0);
    });

    it('a read-only view has nothing to mark', async () => {
        harness = installIpcHarness({});
        articlesStore.filters = { ...ALL_ARTICLES, read: true };
        await markViewRead();
        expect(harness.calls).toHaveLength(0);
        expect(lastToast()).toBe('Marked 0 read');
    });

    it('a failure stops the sweep and toasts the error instead of a count', async () => {
        harness = installIpcHarness({
            list_articles: [articleSummaryFixture({ id: 9 }), articleSummaryFixture({ id: 8 })],
            mark_read: rejectWith(commandErrorFixture({ kind: 'internal', message: 'disk full' })),
        });
        selectView('readLater');
        await markViewRead();
        expect(harness.callsFor('mark_read')).toHaveLength(1);
        expect(toastStore.toasts.some((toast) => toast.tone === 'error')).toBe(true);
        expect(toastStore.toasts.some((toast) => toast.tone === 'success')).toBe(false);
    });
});

describe('actions — reader paging (Space / Shift+Space)', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('drops the press while the selected article has no body yet', () => {
        harness = installIpcHarness({});
        selectionStore.selectedArticleId = 4;
        pageReader('down');
        pageReader('up');
        expect(harness.calls).toHaveLength(0);
        expect(selectionStore.selectedArticleId).toBe(4);
    });

    it('pages a registered body and clamps at the top; unregistering detaches it', () => {
        harness = installIpcHarness({});
        selectionStore.selectedArticleId = 4;
        const body = document.createElement('div');
        Object.defineProperty(body, 'scrollHeight', { value: 2000 });
        Object.defineProperty(body, 'clientHeight', { value: 400 });
        const unregister = registerReaderBody(body);

        pageReader('down');
        expect(body.scrollTop).toBe(350);
        pageReader('up');
        pageReader('up');
        expect(body.scrollTop).toBe(0);

        unregister();
        pageReader('down');
        expect(body.scrollTop).toBe(0);
    });

    it('a stale unregister never detaches the newer body', () => {
        harness = installIpcHarness({});
        selectionStore.selectedArticleId = 4;
        const older = document.createElement('div');
        const newer = document.createElement('div');
        Object.defineProperty(newer, 'scrollHeight', { value: 2000 });
        Object.defineProperty(newer, 'clientHeight', { value: 400 });
        const unregisterOlder = registerReaderBody(older);
        const unregisterNewer = registerReaderBody(newer);
        unregisterOlder();
        pageReader('down');
        expect(newer.scrollTop).toBe(350);
        unregisterNewer();
    });
});

describe('actions — importFromFile', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('imports the picked file and toasts the outcome', async () => {
        harness = installIpcHarness({
            pick_import_file: { token: 'tok-1' },
            import_file: importOutcomeFixture({
                feeds_added: 3,
                articles_added: 10,
                feeds_skipped: 1,
                articles_skipped: 0,
            }),
        });
        await importFromFile('opml');

        expect(harness.callsFor('import_file')).toEqual([{ pathToken: 'tok-1', source: 'opml' }]);
        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.tone).toBe('success');
    });

    it('warns with the invalid-row count when some rows were unusable', async () => {
        harness = installIpcHarness({
            pick_import_file: { token: 'tok-2' },
            import_file: importOutcomeFixture({
                feeds_added: 5,
                feeds_skipped: 0,
                feeds_invalid: 1,
                articles_added: 2,
                articles_skipped: 1,
                articles_invalid: 1,
                rejected: [
                    { kind: 'feed', reason: 'unsupported_scheme', detail: 'webcal://x.test/a' },
                    { kind: 'article', reason: 'invalid_url', detail: 'not a url' },
                ],
            }),
        });
        await importFromFile('pocket_csv');

        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.tone).toBe('warning');
        expect(toastStore.toasts[0]?.message).toBe(
            'Imported 5 feeds and 2 articles, skipped 1; ignored 2 invalid entries',
        );
    });

    it('a cancelled picker toasts politely and never imports', async () => {
        harness = installIpcHarness({ pick_import_file: null, import_file: null });
        await importFromFile('opml');

        expect(harness.callsFor('import_file')).toHaveLength(0);
        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.tone).toBe('info');
    });

    it('a picker failure surfaces an error toast and never imports', async () => {
        harness = installIpcHarness({
            pick_import_file: rejectWith(commandErrorFixture({ kind: 'user', message: 'denied' })),
            import_file: null,
        });
        await importFromFile('opml');

        expect(harness.callsFor('import_file')).toHaveLength(0);
        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.tone).toBe('error');
    });
});
