import ArticleList, { ROW_HEIGHT } from '$components/articles/ArticleList.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
/**
 * The list's view sessions (#142): each filter combination keeps its own
 * scroll position (a cached view never inherits another's offset), mark-on-
 * scroll only marks rows the reader saw scroll past, and a row read in an
 * unread-only view stays put — still selected — until the view changes.
 */
import { events, type ArticleSummaryDto, type ListArticlesDto } from '$lib/bindings';
import {
    markAllRead,
    refreshAll,
    refreshFeed,
    toggleArchived,
    toggleRead,
} from '$lib/state/actions';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    refreshOutcomeFixture,
    rejectWith,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const FEED_A = 1;
const FEED_B = 2;

/** 80 articles per feed, `id DESC`: feed A is 1080..1001, feed B 2080..2001. */
function feedArticles(feedId: number): ArticleSummaryDto[] {
    return Array.from({ length: 80 }, (_, index) => {
        const id = feedId * 1000 + 80 - index;
        return articleSummaryFixture({ id, feed_id: feedId, title: `Article ${id}` });
    });
}

/** A backend over both feeds that honours the feed, read, and keyset filters. */
function harnessFor(read: Set<number>): IpcHarness {
    const all = [...feedArticles(FEED_A), ...feedArticles(FEED_B)];
    return installIpcHarness({
        list_articles: (args) => {
            const params = args.params as ListArticlesDto;
            return all
                .filter((a) => params.feed_id === null || a.feed_id === params.feed_id)
                .filter((a) => params.read !== false || !read.has(a.id))
                .filter((a) => params.before === null || a.id < params.before)
                .slice(0, params.limit);
        },
        list_feeds: [],
        get_article_state: (args) =>
            articleStateFixture({ read: read.has(args.articleId as number) }),
        get_article: (args) => articleFixture({ id: args.articleId as number, feed_id: FEED_A }),
        mark_read: (args) => {
            if (args.read === true) {
                read.add(args.articleId as number);
            } else {
                read.delete(args.articleId as number);
            }
            return true;
        },
        set_archived: true,
        mark_all_read: 0,
        refresh_all: [refreshOutcomeFixture()],
        refresh_feed: refreshOutcomeFixture(),
    });
}

function viewport(container: HTMLElement): HTMLElement {
    return container.querySelector('.virtual-list') as HTMLElement;
}

async function settle(): Promise<void> {
    await flushIpc();
    await tick();
    await flushIpc();
}

async function scrollTo(container: HTMLElement, scrollTop: number): Promise<void> {
    const list = viewport(container);
    list.scrollTop = scrollTop;
    await fireEvent.scroll(list);
    await settle();
}

async function openFeed(feedId: number): Promise<void> {
    selectionStore.selectFeed(feedId);
    await settle();
}

/** The rendered rows' option ids, top of the window first. */
function renderedIds(container: HTMLElement): string[] {
    return [...container.querySelectorAll('.article-row')].map((row) => row.id);
}

function markedIds(harness: IpcHarness): number[] {
    return harness
        .callsFor('mark_read')
        .filter((args) => args.read === true)
        .map((args) => args.articleId as number);
}

describe('list view sessions', () => {
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

    it('each view keeps its own scroll position; a first visit opens at the top', async () => {
        harness = harnessFor(new Set());
        const { container } = render(ArticleList);
        await openFeed(FEED_B); // B is cached from here on
        await openFeed(FEED_A);

        await scrollTo(container, ROW_HEIGHT * 40);
        expect(viewport(container).scrollTop).toBe(ROW_HEIGHT * 40);

        // Cached feed B opens at its own (never-scrolled) position: the top.
        await openFeed(FEED_B);
        expect(viewport(container).scrollTop).toBe(0);
        expect(renderedIds(container)[0]).toContain('2080');

        // Leave B somewhere of its own, then go back to A: row 40 again.
        await scrollTo(container, ROW_HEIGHT * 10 + 20);
        await openFeed(FEED_A);
        expect(viewport(container).scrollTop).toBe(ROW_HEIGHT * 40);
        expect(renderedIds(container)).toContain('article-option-1040');

        // And B comes back exactly where it was left, offset into the row included.
        await openFeed(FEED_B);
        expect(viewport(container).scrollTop).toBe(ROW_HEIGHT * 10 + 20);
    });

    it('the remount on a view switch keeps focus where the keyboard is', async () => {
        harness = harnessFor(new Set());
        const { container } = render(ArticleList);
        await openFeed(FEED_A);

        // The list owns the keyboard: the fresh listbox takes focus over.
        selectionStore.focusList();
        await settle();
        expect(document.activeElement).toBe(viewport(container));
        await openFeed(FEED_B);
        expect(document.activeElement).toBe(viewport(container));

        // The sidebar owns it: switching views must not pull focus out of it.
        const tree = document.createElement('button');
        document.body.append(tree);
        selectionStore.focusSidebar();
        tree.focus();
        await openFeed(FEED_A);
        expect(document.activeElement).toBe(tree);

        // Handing the keyboard back to the list focuses the listbox again.
        selectionStore.focusList();
        await settle();
        expect(document.activeElement).toBe(viewport(container));
        tree.remove();
    });

    it('a view switch hands focus to the new listbox when the old one had it', async () => {
        harness = harnessFor(new Set());
        const { container } = render(ArticleList);
        await openFeed(FEED_B);
        await openFeed(FEED_A);

        // Focus reached the listbox directly (a click, Tab) — no explicit
        // focusList, so the focus nonce is still 0.
        expect(selectionStore.listFocusNonce).toBe(0);
        viewport(container).focus();

        // `n` / `g s` / the unread toggle: a cached view remounts the listbox
        // and the fresh one takes focus over, so Home/End/Page keys keep working.
        await openFeed(FEED_B);
        expect(document.activeElement).toBe(viewport(container));

        // An uncached view shows a skeleton first; focus is handed over once
        // its rows mount.
        articlesStore.toggleUnreadOnly();
        await settle();
        expect(document.activeElement).toBe(viewport(container));
    });

    it('a view switch leaves focus on the control that switched views', async () => {
        harness = harnessFor(new Set());
        const { container } = render(ArticleList);
        await openFeed(FEED_B);
        await openFeed(FEED_A);

        // The list was entered explicitly once (the nonce is non-zero, and the
        // focus zone stays 'list' from here on) ...
        selectionStore.focusList();
        await settle();
        expect(document.activeElement).toBe(viewport(container));

        // ... then the reader clicks the unread toggle / a sidebar row.
        const control = document.createElement('button');
        document.body.append(control);
        control.focus();
        articlesStore.toggleUnreadOnly();
        await settle();
        expect(document.activeElement).toBe(control);
        await openFeed(FEED_B);
        expect(document.activeElement).toBe(control);
        control.remove();
    });

    it('a restored position follows its anchor row when rows arrive above it', async () => {
        harness = harnessFor(new Set());
        const { container } = render(ArticleList);
        await openFeed(FEED_A);
        await scrollTo(container, ROW_HEIGHT * 5);
        await openFeed(FEED_B);

        // Two new articles land at the top of A while it is out of view.
        const extra = [
            articleSummaryFixture({ id: 1082, feed_id: FEED_A }),
            articleSummaryFixture({ id: 1081, feed_id: FEED_A }),
        ];
        harness.respond('list_articles', (args) => {
            const params = args.params as ListArticlesDto;
            return [...extra, ...feedArticles(FEED_A)]
                .filter((a) => params.before === null || a.id < params.before)
                .slice(0, params.limit);
        });
        await articlesStore.list({ ...ALL_ARTICLES, feedId: FEED_A }).query.refetch();

        await openFeed(FEED_A);
        // The anchor (1075, row 5 before) is now row 7.
        expect(viewport(container).scrollTop).toBe(ROW_HEIGHT * 7);
    });

    it('with mark-on-scroll on, switching to a cached feed and scrolling one row marks one article', async () => {
        harness = harnessFor(new Set());
        prefsStore.markOnScroll = true;
        const { container } = render(ArticleList);
        await openFeed(FEED_B);
        await openFeed(FEED_A);
        await scrollTo(container, ROW_HEIGHT * 40);
        const beforeSwitch = markedIds(harness).length;

        await openFeed(FEED_B);
        const top = viewport(container).scrollTop;
        await scrollTo(container, top + ROW_HEIGHT);
        const marked = markedIds(harness).slice(beforeSwitch);
        expect(marked).toEqual([2080]);

        // Back on A at its restored row 40: rows 0..39 were never on screen
        // this session, so one more row scrolled marks just row 40.
        await openFeed(FEED_A);
        const beforeReturn = markedIds(harness).length;
        await scrollTo(container, ROW_HEIGHT * 41);
        expect(markedIds(harness).slice(beforeReturn)).toEqual([1040]);
    });

    it('mark-on-scroll marks only rows that were on screen, by id', async () => {
        harness = harnessFor(new Set());
        prefsStore.markOnScroll = true;
        const { container } = render(ArticleList);
        await openFeed(FEED_A);

        // A jump far down (scrollbar drag): only the rows that had been on
        // screen at the top are marked — not the 32 rows skipped over. The
        // opening screenful is rows 0..7 (600px over 84px rows).
        await scrollTo(container, ROW_HEIGHT * 40);
        expect(markedIds(harness)).toEqual([1080, 1079, 1078, 1077, 1076, 1075, 1074, 1073]);
    });

    it('unread view: opening an article keeps its row, still selected, until the view changes', async () => {
        const read = new Set<number>();
        harness = harnessFor(read);
        const unwire = await wireInvalidation();
        articlesStore.filters = { ...ALL_ARTICLES, feedId: FEED_A, read: false };
        const { container } = render(ArticleList);
        render(ReaderPane);
        await settle();

        const row = container.querySelector('#article-option-1078') as HTMLElement;
        await fireEvent.click(row);
        await settle();
        // The reader marked it read; the backend's unread window now drops it.
        expect(read.has(1078)).toBe(true);
        await events.articlesChanged.emit({ feed_id: null });
        await settle();

        const list = articlesStore.current;
        expect(list.query.data?.some((a) => a.id === 1078)).toBe(false);
        const kept = container.querySelector('#article-option-1078');
        expect(kept).not.toBeNull();
        expect(kept?.getAttribute('aria-selected')).toBe('true');
        expect(selectionStore.selectedIndex).toBe(2);

        // A view change ends the session; coming back shows the backend window.
        articlesStore.toggleUnreadOnly();
        await settle();
        articlesStore.toggleUnreadOnly();
        await settle();
        expect(container.querySelector('#article-option-1078')).toBeNull();
        unwire();
    });

    it('mark-on-scroll in the unread view holds the passed rows so nothing shifts', async () => {
        const read = new Set<number>();
        harness = harnessFor(read);
        prefsStore.markOnScroll = true;
        const unwire = await wireInvalidation();
        articlesStore.filters = { ...ALL_ARTICLES, feedId: FEED_A, read: false };
        const { container } = render(ArticleList);
        await settle();

        await scrollTo(container, ROW_HEIGHT * 3);
        expect(markedIds(harness)).toEqual([1080, 1079, 1078]);
        await events.articlesChanged.emit({ feed_id: null });
        await settle();

        // The backend window dropped the three read rows, but they are held
        // in place: the next row scrolled past is the fourth, not a row that
        // shifted up unseen.
        const list = articlesStore.current;
        expect(list.query.data?.[0]?.id).toBe(1077);
        expect(list.items.slice(0, 4).map((a) => a.id)).toEqual([1080, 1079, 1078, 1077]);
        await scrollTo(container, ROW_HEIGHT * 4);
        expect(markedIds(harness)).toEqual([1080, 1079, 1078, 1077]);
        unwire();
    });

    it('releasing the hold keeps the viewport on the same rows and marks nothing unseen', async () => {
        const read = new Set<number>();
        harness = harnessFor(read);
        prefsStore.markOnScroll = true;
        const unwire = await wireInvalidation();
        articlesStore.filters = { ...ALL_ARTICLES, feedId: FEED_A, read: false };
        const { container } = render(ArticleList);
        await settle();

        // Scroll twenty rows, one at a time: 1080..1061 are read and held.
        for (let row = 1; row <= 20; row++) {
            await scrollTo(container, ROW_HEIGHT * row + 10);
        }
        const scrolled = Array.from({ length: 20 }, (_, index) => 1080 - index);
        expect(markedIds(harness)).toEqual(scrolled);
        await events.articlesChanged.emit({ feed_id: null });
        await settle();
        expect(articlesStore.current.items[20]?.id).toBe(1060);

        // An explicit refresh lets the twenty held rows go. The row the reader
        // was looking at (1060, 10px in) stays at the viewport top, so nothing
        // on screen counts as scrolled past and nothing unseen slides above.
        refreshAll();
        await settle();
        expect(articlesStore.current.items[0]?.id).toBe(1060);
        expect(viewport(container).scrollTop).toBe(10);
        expect(renderedIds(container)[0]).toBe('article-option-1060');
        expect(markedIds(harness)).toEqual(scrolled);

        // Scrolling on marks the next row, exactly as before the release.
        await scrollTo(container, ROW_HEIGHT + 10);
        expect(markedIds(harness)).toEqual([...scrolled, 1060]);
        unwire();
    });
});

describe('read-hold actions', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        harness?.teardown();
        harness = null;
    });

    /** Opens feed A unread-only and reads `ids` with `toggleRead` (holding them). */
    async function readInUnreadView(ids: number[]): Promise<Set<number>> {
        const read = new Set<number>();
        harness = harnessFor(read);
        articlesStore.filters = { ...ALL_ARTICLES, feedId: FEED_A, read: false };
        const list = articlesStore.current;
        await flushIpc();
        for (const id of ids) {
            await toggleRead(id);
        }
        await list.query.refetch();
        return read;
    }

    function shownIds(): number[] {
        return articlesStore.current.items.slice(0, 4).map((a) => a.id);
    }

    it('toggleRead holds the row it marks read', async () => {
        const read = await readInUnreadView([1079]);
        expect(read.has(1079)).toBe(true);
        expect(shownIds()).toEqual([1080, 1079, 1078, 1077]);
    });

    it('archiving lets a held row leave the view', async () => {
        await readInUnreadView([1079]);
        await toggleArchived(1079);
        expect(shownIds()).toEqual([1080, 1078, 1077, 1076]);
    });

    it('an explicit refresh ends the hold', async () => {
        await readInUnreadView([1079]);
        refreshAll();
        await flushIpc();
        expect(shownIds()).toEqual([1080, 1078, 1077, 1076]);
    });

    it('mark-all-read ends the hold', async () => {
        await readInUnreadView([1079]);
        await markAllRead(FEED_A);
        expect(shownIds()).toEqual([1080, 1078, 1077, 1076]);
    });

    it('a library-wide mark-all-read ends the hold', async () => {
        await readInUnreadView([1079]);
        await markAllRead(null);
        expect(shownIds()).toEqual([1080, 1078, 1077, 1076]);
    });

    it('refreshing or sweeping another feed keeps this view held', async () => {
        await readInUnreadView([1079]);
        refreshFeed(FEED_B);
        await flushIpc();
        expect(shownIds()).toEqual([1080, 1079, 1078, 1077]);
        await markAllRead(FEED_B);
        expect(shownIds()).toEqual([1080, 1079, 1078, 1077]);

        // Refreshing the view's own feed does end it.
        refreshFeed(FEED_A);
        await flushIpc();
        expect(shownIds()).toEqual([1080, 1078, 1077, 1076]);
    });

    it('a failed mark-all-read keeps the hold', async () => {
        await readInUnreadView([1079]);
        harness?.respond('mark_all_read', rejectWith(commandErrorFixture()));
        await markAllRead(FEED_A);
        expect(shownIds()).toEqual([1080, 1079, 1078, 1077]);
    });

    it('a failed archive keeps the held row in place', async () => {
        await readInUnreadView([1079]);
        harness?.respond('set_archived', rejectWith(commandErrorFixture()));
        await toggleArchived(1079);
        expect(shownIds()).toEqual([1080, 1079, 1078, 1077]);
    });

    it('unarchiving leaves a held row held', async () => {
        await readInUnreadView([1079]);
        harness?.respond('get_article_state', articleStateFixture({ read: true, archived: true }));
        await toggleArchived(1079);
        expect(harness?.callsFor('set_archived').at(-1)?.archived).toBe(false);
        expect(shownIds()).toEqual([1080, 1079, 1078, 1077]);
    });
});
