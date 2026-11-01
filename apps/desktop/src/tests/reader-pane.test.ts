import ReaderPane from '$components/reader/ReaderPane.svelte';
/**
 * The reader pane over mockIPC: article loading through the query cache,
 * mark-read-on-open, the sanitized body (through SanitizedHtml), the action
 * toolbar's state flips with event-driven refresh, the error tiers, and the
 * j/k path: a toolbar + header that stay mounted while a body loads, and the
 * neighbour prefetch that lets the next step render straight from cache.
 */
import { type ArticleDto, events } from '$lib/bindings';
import { localeStore } from '$lib/i18n';
import { keyLabel } from '$lib/keyboard/labels';
import { SHORTCUTS } from '$lib/keyboard/registry';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { PREFETCH_SETTLE_MS, articlePrefetcher } from '$lib/state/prefetch';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const CONTENT = '<p>Hello <em>world</em></p><script>alert(1)</script>';

function baseResponders(): Record<string, Responder> {
    return {
        get_article: articleFixture({
            id: 100,
            title: 'The article',
            content_html: CONTENT,
            author: 'A. Writer',
            word_count: 250,
        }),
        get_article_state: articleStateFixture(),
        mark_read: true,
        list_feeds: [],
    };
}

describe('ReaderPane', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        // Unmount first: a mounted tree re-creates queries on cache reset.
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        articlePrefetcher.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('shows the empty state without a selection', () => {
        harness = installIpcHarness({});
        const { getByText } = render(ReaderPane);
        expect(getByText('Select an article to read')).toBeTruthy();
    });

    it('the empty-state hint names only keys the registry binds', () => {
        harness = installIpcHarness({});
        const { container } = render(ReaderPane);
        const hint = container.querySelector('.empty-hint') as HTMLElement;
        expect(hint.textContent?.replace(/\s+/g, ' ').trim()).toBe(
            '↑↓ to move· Enter/→ to read· ? for shortcuts',
        );
        const bound = new Set(
            SHORTCUTS.flatMap((shortcut) => [
                ...shortcut.keys,
                ...(shortcut.paneKeys?.keys ?? []),
            ]).map(keyLabel),
        );
        const printed = [...hint.querySelectorAll('kbd')].map((kbd) => kbd.textContent ?? '');
        expect(printed.length).toBeGreaterThan(0);
        for (const label of printed) {
            expect(bound, label).toContain(label);
        }
    });

    it('the empty-state hint is localized', async () => {
        harness = installIpcHarness({});
        localeStore.active = 'de';
        try {
            const { container } = render(ReaderPane);
            const hint = container.querySelector('.empty-hint') as HTMLElement;
            expect(hint.textContent).toContain('zum Bewegen');
            expect(hint.textContent).not.toContain('to move');
        } finally {
            localeStore.reset();
        }
    });

    it('loads the selected article, marks it read, and renders sanitized content', async () => {
        harness = installIpcHarness(baseResponders());
        selectionStore.selectedArticleId = 100;
        const { container, getByText } = render(ReaderPane);
        await flushIpc();

        expect(getByText('The article')).toBeTruthy();
        expect(getByText('A. Writer')).toBeTruthy();
        expect(getByText('250 words')).toBeTruthy();
        expect(container.querySelector('em')?.textContent).toBe('world');
        expect(container.querySelector('script')).toBeNull();

        // Displaying the article marked it read (idempotent core-side).
        expect(harness.callsFor('mark_read')).toEqual([{ articleId: 100, read: true }]);
    });

    it('marks read once per selection, not per render', async () => {
        harness = installIpcHarness(baseResponders());
        selectionStore.selectedArticleId = 100;
        render(ReaderPane);
        await flushIpc();
        selectionStore.selectedArticleId = 100; // same selection re-assigned
        await flushIpc();
        expect(harness.callsFor('mark_read')).toHaveLength(1);
    });

    it('does not mark read when the article fails to load', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            get_article: rejectWith(commandErrorFixture({ kind: 'user', message: 'gone' })),
        });
        selectionStore.selectedArticleId = 100;
        render(ReaderPane);
        await flushIpc();
        expect(harness.callsFor('mark_read')).toHaveLength(0);
    });

    it('the typography popover opens and dismisses on Escape', async () => {
        harness = installIpcHarness(baseResponders());
        selectionStore.selectedArticleId = 100;
        const { getByRole, queryByRole } = render(ReaderPane);
        await flushIpc();

        await fireEvent.click(getByRole('button', { name: 'Typography' }));
        expect(getByRole('dialog', { name: 'Typography' })).toBeTruthy();

        await fireEvent.keyDown(window, { key: 'Escape' });
        expect(queryByRole('dialog', { name: 'Typography' })).toBeNull();
    });

    it('toolbar star flips through the backend and refreshes on the event', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            set_starred: true,
        });
        const unwire = await wireInvalidation();
        selectionStore.selectedArticleId = 100;
        const { getByRole } = render(ReaderPane);
        await flushIpc();

        const star = getByRole('button', { name: 'Star' });
        expect(star.getAttribute('aria-pressed')).toBe('false');
        await fireEvent.click(star);
        await flushIpc();
        expect(harness.callsFor('set_starred')).toEqual([{ articleId: 100, starred: true }]);

        // Rust emits ArticlesChanged after a real flip → the state refetches.
        harness.respond('get_article_state', articleStateFixture({ starred: true }));
        await events.articlesChanged.emit({ feed_id: null });
        await flushIpc();
        expect(getByRole('button', { name: 'Unstar' }).getAttribute('aria-pressed')).toBe('true');
        unwire();
    });

    it('toolbar wires read-later and archive to their commands', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            set_read_later: true,
            set_archived: true,
        });
        selectionStore.selectedArticleId = 100;
        const { getByRole } = render(ReaderPane);
        await flushIpc();

        await fireEvent.click(getByRole('button', { name: 'Read later' }));
        await flushIpc();
        expect(harness.callsFor('set_read_later')).toEqual([{ articleId: 100, readLater: true }]);

        await fireEvent.click(getByRole('button', { name: 'Archive' }));
        await flushIpc();
        expect(harness.callsFor('set_archived')).toEqual([{ articleId: 100, archived: true }]);
    });

    it('open-in-browser goes through the scoped opener and records the open', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            'plugin:opener|open_url': null,
            record_opened: null,
        });
        selectionStore.selectedArticleId = 100;
        const { getByRole } = render(ReaderPane);
        await flushIpc();

        await fireEvent.click(getByRole('button', { name: 'Open in browser' }));
        await flushIpc();
        const opens = harness.callsFor('plugin:opener|open_url');
        expect(opens).toHaveLength(1);
        expect(opens[0]?.url).toBe('https://example.org/post/100');
        expect(harness.callsFor('record_opened')).toEqual([{ articleId: 100, dwellMs: null }]);
    });

    it('surfaces user-tier load failures verbatim and internal ones generically', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            get_article: rejectWith(
                commandErrorFixture({ kind: 'user', message: 'article was deleted' }),
            ),
        });
        selectionStore.selectedArticleId = 100;
        const { getByRole } = render(ReaderPane);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('article was deleted');

        resetQueryCache();
        harness.respond(
            'get_article',
            rejectWith(commandErrorFixture({ kind: 'internal', code: 'INTERNAL', message: 'sql' })),
        );
        selectionStore.selectedArticleId = 101;
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('Something went wrong');
        expect(getByRole('alert').textContent).not.toContain('sql');
    });

    it('keeps the article under a banner when a refetch fails (stale-while-revalidate)', async () => {
        harness = installIpcHarness(baseResponders());
        const unwire = await wireInvalidation();
        selectionStore.selectedArticleId = 100;
        const { getByText, getByRole, queryByText } = render(ReaderPane);
        await flushIpc();
        expect(getByText('The article')).toBeTruthy();

        // A refetch now fails; the loaded article must survive under a banner
        // rather than collapsing to the full-screen error tier.
        harness.respond(
            'get_article',
            rejectWith(commandErrorFixture({ kind: 'user', message: 'feed went away' })),
        );
        await events.articlesChanged.emit({ feed_id: null });
        await flushIpc();

        expect(getByText('The article')).toBeTruthy(); // still shown
        expect(getByRole('alert').textContent).toContain('feed went away'); // banner
        expect(queryByText('This article no longer exists')).toBeNull();
        unwire();
    });

    it('the load-full-article button hydrates through load_full_article', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            load_full_article: articleFixture({ id: 100, content_html: '<p>Full body</p>' }),
        });
        selectionStore.selectedArticleId = 100;
        const { getByRole } = render(ReaderPane);
        await flushIpc();

        await fireEvent.click(getByRole('button', { name: 'Load full article' }));
        await flushIpc();
        expect(harness.callsFor('load_full_article')).toEqual([{ articleId: 100 }]);
    });

    it('shows the missing state when the article no longer exists', async () => {
        harness = installIpcHarness({
            ...baseResponders(),
            get_article: null,
        });
        selectionStore.selectedArticleId = 100;
        const { getByText } = render(ReaderPane);
        await flushIpc();
        expect(getByText('This article no longer exists')).toBeTruthy();
    });

    describe('j/k: stable chrome and neighbour prefetch', () => {
        // The prefetch settle delay runs on a fake clock, so "within the delay"
        // and "after it" never depend on how loaded the test machine is.
        beforeEach(() => {
            vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
        });

        afterEach(() => {
            vi.useRealTimers();
        });

        /** Lets pending IPC replies and effects land, without moving the clock. */
        async function flush(): Promise<void> {
            await vi.advanceTimersByTimeAsync(0);
        }
        /** A loaded window of ten rows, ids 110 down to 101. */
        const ROWS = Array.from({ length: 10 }, (_, i) =>
            articleSummaryFixture({
                id: 110 - i,
                title: `Row ${110 - i}`,
                source_url: `https://example.org/post/${110 - i}`,
                author: 'Row Author',
            }),
        );

        /** Responders where every article loads, by id, with its own body. */
        function windowResponders(): Record<string, Responder> {
            return {
                ...baseResponders(),
                list_articles: ROWS,
                get_article: (args) => {
                    const id = args.articleId as number;
                    return articleFixture({
                        id,
                        title: `Row ${id}`,
                        source_url: `https://example.org/post/${id}`,
                        content_html: `<p>Body of ${id}</p>`,
                    });
                },
            };
        }

        async function loadWindow(): Promise<void> {
            articlesStore.filters = ALL_ARTICLES;
            void articlesStore.current; // touch to start the list fetch
            await flush();
        }

        /** Waits out the prefetch settle delay, then lets the fetches land. */
        async function settle(): Promise<void> {
            await vi.advanceTimersByTimeAsync(PREFETCH_SETTLE_MS);
            await flush();
        }

        function fetchedIds(): number[] {
            return (harness?.callsFor('get_article') ?? []).map((args) => args.articleId as number);
        }

        it('keeps the toolbar and the row header while get_article is pending', async () => {
            let release: (value: unknown) => void = () => undefined;
            harness = installIpcHarness({
                ...windowResponders(),
                get_article: () =>
                    new Promise((resolve) => {
                        release = resolve;
                    }),
            });
            await loadWindow();
            selectionStore.selectedArticleId = 107;
            const { container, getByRole, getByText, queryByText } = render(ReaderPane);
            await flush();

            // Body still in flight: the chrome is up, the header comes from the
            // list row, and the body is a skeleton — not a bare "Loading…".
            const toolbar = getByRole('toolbar');
            expect(getByRole('heading', { level: 1 }).textContent).toContain('Row 107');
            expect(getByText('Row Author')).toBeTruthy();
            expect(getByRole('button', { name: 'Star' })).toBeTruthy();
            expect(container.querySelector('.reader-status')).toBeNull();
            expect(container.querySelector('.body-skeleton')?.getAttribute('aria-busy')).toBe(
                'true',
            );
            // The body-dependent action waits for the body.
            expect(
                getByRole('button', { name: 'Load full article' }).hasAttribute('disabled'),
            ).toBe(true);

            release(articleFixture({ id: 107, title: 'Row 107', content_html: '<p>Landed</p>' }));
            await flush();

            expect(getByText('Landed')).toBeTruthy();
            expect(container.querySelector('.body-skeleton')).toBeNull();
            // Landing over its skeleton, the body fades in on its own.
            expect(container.querySelector('.reader-body')?.classList.contains('landing')).toBe(
                true,
            );
            expect(queryByText('Loading…')).toBeNull();
            // Same toolbar node: the body landing did not remount the chrome.
            expect(getByRole('toolbar')).toBe(toolbar);
        });

        it('selecting row N prefetches N±1 and N±2, nearest and forward first', async () => {
            harness = installIpcHarness(windowResponders());
            await loadWindow();
            selectionStore.selectedArticleId = ROWS[4].id; // 106
            render(ReaderPane);
            await flush();
            // Only the selection until it has sat still for the settle delay.
            expect(fetchedIds()).toEqual([106]);
            await settle();

            // The selected body first, then the neighbours: +1, -1, +2, -2.
            expect(fetchedIds()).toEqual([106, 105, 107, 104, 108]);
            expect(harness.callsFor('get_article_state').map((a) => a.articleId)).toEqual(
                expect.arrayContaining([105, 107, 104, 108]),
            );
        });

        it('moving to a prefetched neighbour renders from cache with no loading state', async () => {
            harness = installIpcHarness(windowResponders());
            await loadWindow();
            selectionStore.selectedArticleId = 106;
            const { container, getByRole, getByText, queryByText } = render(ReaderPane);
            await settle();
            const toolbar = getByRole('toolbar');
            const callsBefore = fetchedIds().length;

            selectionStore.selectNextArticle(); // j → 105, already prefetched
            await tick();

            // Rendered synchronously from cache: no skeleton, no "Loading…",
            // no new fetch for the landed article, and the chrome stayed put.
            expect(getByText('Body of 105')).toBeTruthy();
            expect(container.querySelector('.body-skeleton')).toBeNull();
            expect(queryByText('Loading…')).toBeNull();
            expect(fetchedIds().slice(callsBefore)).not.toContain(105);
            expect(getByRole('toolbar')).toBe(toolbar);
            // A cached body rides the switch fade alone — no second fade on it.
            expect(container.querySelector('.reader-body')?.classList.contains('landing')).toBe(
                false,
            );

            // …and the window slides: the new outer neighbour is fetched.
            await settle();
            expect(fetchedIds()).toContain(103);
        });

        it('evicts prefetched bodies the selection has moved away from', async () => {
            harness = installIpcHarness(windowResponders());
            await loadWindow();
            selectionStore.selectedArticleId = 109;
            render(ReaderPane);
            await settle();
            expect(articlePrefetcher.owned).toEqual(expect.arrayContaining([108, 110, 107]));

            selectionStore.selectedArticleId = 103;
            await settle();
            // 110/108/107 are now more than two rows away and were dropped.
            expect([...articlePrefetcher.owned].sort((a, b) => a - b)).toEqual([
                101, 102, 104, 105,
            ]);

            selectionStore.selectedArticleId = null;
            await flush();
            expect(articlePrefetcher.owned).toEqual([]);
        });

        it('a held j fetches no neighbours until the selection settles', async () => {
            harness = installIpcHarness(windowResponders());
            await loadWindow();
            selectionStore.selectedArticleId = 109;
            selectionStore.rememberIndex(1);
            const { getByText } = render(ReaderPane);
            await settle();
            expect([...articlePrefetcher.owned].sort((a, b) => a - b)).toEqual([107, 108, 110]);
            const callsBefore = fetchedIds().length;

            // Key repeat: two steps onto cached rows, each well inside the
            // settle delay of the one before.
            selectionStore.selectNextArticle(); // → 108, cached
            await flush();
            selectionStore.selectNextArticle(); // → 107, cached
            await flush();

            expect(getByText('Body of 107')).toBeTruthy();
            // No fetch for the outer rows the burst only passed by…
            expect(fetchedIds().slice(callsBefore)).toEqual([]);
            // …while what fell out of range (110) was dropped at once.
            expect([...articlePrefetcher.owned].sort((a, b) => a - b)).toEqual([107, 108]);

            // Holding still on 107 fetches its window.
            await settle();
            expect(fetchedIds().slice(callsBefore)).toEqual([106, 105]);
        });

        it('centres on the vacated slot when the selection leaves the window', async () => {
            let rows = ROWS;
            harness = installIpcHarness({
                ...windowResponders(),
                list_articles: () => rows,
                set_archived: true,
            });
            const unwire = await wireInvalidation();
            await loadWindow();
            selectionStore.selectedArticleId = 107;
            selectionStore.rememberIndex(3);
            const { getByRole } = render(ReaderPane);
            await settle();
            expect([...articlePrefetcher.owned].sort((a, b) => a - b)).toEqual([
                105, 106, 108, 109,
            ]);

            // Archived out of the view: the read-hold lets go of it, the backend
            // stops returning it, so 106 shifts into slot 3 and the selection is
            // no longer in the window.
            await fireEvent.click(getByRole('button', { name: 'Archive' }));
            await flush();
            rows = ROWS.filter((row) => row.id !== 107);
            await events.articlesChanged.emit({ feed_id: null });
            await flush();
            expect(selectionStore.selectedIndex).toBe(-1);
            await settle();

            // Centred on slot 3 (106, j's next landing), not on the -1 index
            // (which would prefetch nothing and evict every owned body): 104 at
            // +2 is new, and the rest of the window is kept.
            expect(fetchedIds()).toContain(104);
            expect([...articlePrefetcher.owned].sort((a, b) => a - b)).toEqual([
                104, 105, 106, 108, 109,
            ]);
            unwire();
        });
    });
});

describe('ReaderPane — link-less and date-less feed entries', () => {
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

    const FEED_URL = 'https://example.org/feed.xml';

    function responders(overrides: Partial<ArticleDto>): Record<string, Responder> {
        return {
            ...baseResponders(),
            get_article: articleFixture({
                id: 100,
                title: 'The article',
                feed_id: 1,
                ...overrides,
            }),
            list_feeds: [feedFixture({ id: 1, url: FEED_URL, title: 'Example Feed' })],
        };
    }

    it('a link-less entry offers no open-in-browser / load-full, and its title is not a link', async () => {
        harness = installIpcHarness(responders({ source_url: FEED_URL }));
        selectionStore.selectedArticleId = 100;
        const { getByRole, queryByRole } = render(ReaderPane);
        await flushIpc();

        const title = getByRole('heading', { level: 1 });
        expect(title.textContent?.trim()).toBe('The article');
        expect(title.querySelector('a')).toBeNull();
        expect(queryByRole('button', { name: 'Open in browser' })).toBeNull();
        expect(queryByRole('button', { name: 'Load full article' })).toBeNull();
        // The rest of the toolbar is untouched.
        expect(getByRole('button', { name: 'Star' })).toBeTruthy();
    });

    it('an entry with its own link keeps the link and both actions', async () => {
        harness = installIpcHarness(responders({ source_url: 'https://example.org/post/100' }));
        selectionStore.selectedArticleId = 100;
        const { getByRole } = render(ReaderPane);
        await flushIpc();

        const link = getByRole('heading', { level: 1 }).querySelector('a');
        expect(link?.getAttribute('href')).toBe('https://example.org/post/100');
        expect(getByRole('button', { name: 'Open in browser' })).toBeTruthy();
        expect(getByRole('button', { name: 'Load full article' })).toBeTruthy();
    });

    it('a date-less entry shows its first-seen date, marked as first seen', async () => {
        harness = installIpcHarness(
            responders({ published_at: null, saved_at: '2026-07-02T10:00:00.000Z' }),
        );
        selectionStore.selectedArticleId = 100;
        const { container } = render(ReaderPane);
        await flushIpc();

        const date = container.querySelector('.reader-meta [data-first-seen]');
        expect(date?.textContent).toMatch(/^First seen .*2026/);
    });

    it('a dated entry shows its publication date, unmarked', async () => {
        harness = installIpcHarness(responders({ published_at: '2026-07-01T09:00:00.000Z' }));
        selectionStore.selectedArticleId = 100;
        const { container } = render(ReaderPane);
        await flushIpc();

        const meta = container.querySelector('.reader-meta');
        expect(meta?.querySelector('[data-first-seen]')).toBeNull();
        expect(meta?.textContent).toContain('2026');
        expect(meta?.textContent).not.toContain('First seen');
    });
});
