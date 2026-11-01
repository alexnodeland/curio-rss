import ReaderPane from '$components/reader/ReaderPane.svelte';
/**
 * The reader pane over mockIPC: article loading through the query cache,
 * mark-read-on-open, the sanitized body (through SanitizedHtml), the action
 * toolbar's state flips with event-driven refresh, the error tiers, and the
 * j/k path: a toolbar + header that stay mounted while a body loads, and the
 * neighbour prefetch that lets the next step render straight from cache.
 */
import { events } from '$lib/bindings';
import { localeStore } from '$lib/i18n';
import { keyLabel } from '$lib/keyboard/labels';
import { SHORTCUTS } from '$lib/keyboard/registry';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { articlePrefetcher } from '$lib/state/prefetch';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
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
            await flushIpc();
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
            await flushIpc();

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
            await flushIpc();

            expect(getByText('Landed')).toBeTruthy();
            expect(container.querySelector('.body-skeleton')).toBeNull();
            expect(queryByText('Loading…')).toBeNull();
            // Same toolbar node: the body landing did not remount the chrome.
            expect(getByRole('toolbar')).toBe(toolbar);
        });

        it('selecting row N prefetches N±1 and N±2, nearest and forward first', async () => {
            harness = installIpcHarness(windowResponders());
            await loadWindow();
            selectionStore.selectedArticleId = ROWS[4].id; // 106
            render(ReaderPane);
            await flushIpc();

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
            await flushIpc();
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

            // …and the window slides: the new outer neighbour is fetched.
            await flushIpc();
            expect(fetchedIds()).toContain(103);
        });

        it('evicts prefetched bodies the selection has moved away from', async () => {
            harness = installIpcHarness(windowResponders());
            await loadWindow();
            selectionStore.selectedArticleId = 109;
            render(ReaderPane);
            await flushIpc();
            expect(articlePrefetcher.owned).toEqual(expect.arrayContaining([108, 110, 107]));

            selectionStore.selectedArticleId = 103;
            await flushIpc();
            // 110/108/107 are now more than two rows away and were dropped.
            expect([...articlePrefetcher.owned].sort((a, b) => a - b)).toEqual([
                101, 102, 104, 105,
            ]);

            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(articlePrefetcher.owned).toEqual([]);
        });
    });
});
