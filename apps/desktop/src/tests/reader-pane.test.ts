import ReaderPane from '$components/reader/ReaderPane.svelte';
/**
 * The reader pane over mockIPC: article loading through the query cache,
 * mark-read-on-open, the sanitized body (through SanitizedHtml), the action
 * toolbar's state flips with event-driven refresh, the error tiers, and the
 * j/k path: a toolbar + header that stay mounted while a body loads, and the
 * neighbour prefetch that lets the next step render straight from cache.
 */
import { events, type ArticleDto } from '$lib/bindings';
import { localeStore } from '$lib/i18n';
import { keyLabel } from '$lib/keyboard/labels';
import { SHORTCUTS } from '$lib/keyboard/registry';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { PREFETCH_SETTLE_MS, articlePrefetcher } from '$lib/state/prefetch';
import {
    hasQuery,
    queryKeys,
    resetQueryCache,
    wireInvalidation,
} from '$lib/state/query-cache.svelte';
import { HIDDEN_END_MS, IDLE_CAP_MS } from '$lib/state/reading-session';
import { selectionStore } from '$lib/state/selection.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { cleanup, fireEvent, render, within } from '@testing-library/svelte';
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
import { installSpeech, voice } from './speech-fake';
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
        await events.articlesChanged.emit({ feed_id: null, flags: null });
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

    it('open-in-browser goes through the scoped opener and records the open with its dwell', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        vi.spyOn(document, 'hasFocus').mockReturnValue(true);
        try {
            harness = installIpcHarness({
                ...baseResponders(),
                'plugin:opener|open_url': null,
                record_opened: null,
            });
            selectionStore.selectedArticleId = 100;
            const { getByRole } = render(ReaderPane);
            await flushIpc();

            vi.advanceTimersByTime(4_000);
            await fireEvent.click(getByRole('button', { name: 'Open in browser' }));
            await flushIpc();
            const opens = harness.callsFor('plugin:opener|open_url');
            expect(opens).toHaveLength(1);
            expect(opens[0]?.url).toBe('https://example.org/post/100');
            // Recorded at once, with the time read in the reader so far.
            expect(harness.callsFor('record_opened')).toEqual([{ articleId: 100, dwellMs: 4_000 }]);

            // The count restarts: a quick close afterwards records nothing more.
            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('record_opened')).toHaveLength(1);
        } finally {
            vi.useRealTimers();
            vi.restoreAllMocks();
        }
    });

    describe('reading aids (#153)', () => {
        /** Lays the scroll body out as `scrollHeight` tall in a `clientHeight` viewport. */
        function layOut(
            container: HTMLElement,
            scrollHeight: number,
            clientHeight: number,
        ): HTMLElement {
            const body = container.querySelector('.reader-scroll') as HTMLElement;
            Object.defineProperty(body, 'scrollHeight', {
                value: scrollHeight,
                configurable: true,
            });
            Object.defineProperty(body, 'clientHeight', {
                value: clientHeight,
                configurable: true,
            });
            return body;
        }

        /**
         * Holds animation frames until the test runs them, so the body is laid
         * out before the frame the resume waits for — deterministically, however
         * long the loads took.
         */
        function holdFrames(): () => Promise<void> {
            const pending: FrameRequestCallback[] = [];
            vi.spyOn(window, 'requestAnimationFrame').mockImplementation((callback) => {
                pending.push(callback);
                return pending.length;
            });
            return async () => {
                for (const callback of pending.splice(0)) {
                    callback(0);
                }
                await tick();
            };
        }

        afterEach(() => {
            vi.useRealTimers();
            vi.restoreAllMocks();
        });

        it('shows the estimated reading time next to the word count', async () => {
            harness = installIpcHarness({
                ...baseResponders(),
                get_article: articleFixture({ id: 100, word_count: 1_840 }),
            });
            selectionStore.selectedArticleId = 100;
            const { container } = render(ReaderPane);
            await flushIpc();
            expect(container.querySelector('[data-read-time]')?.textContent).toBe('8 min read');
        });

        it('records article.opened with the foreground dwell when the selection moves on', async () => {
            vi.useFakeTimers({ toFake: ['Date', 'performance'] });
            const focused = vi.spyOn(document, 'hasFocus').mockReturnValue(true);
            harness = installIpcHarness({ ...baseResponders(), record_opened: null });
            selectionStore.selectedArticleId = 100;
            render(ReaderPane);
            await flushIpc();

            vi.advanceTimersByTime(3_000);
            // Behind another app for a minute: not reading.
            focused.mockReturnValue(false);
            window.dispatchEvent(new FocusEvent('blur'));
            vi.advanceTimersByTime(60_000);
            focused.mockReturnValue(true);
            window.dispatchEvent(new FocusEvent('focus'));
            vi.advanceTimersByTime(2_000);
            expect(harness.callsFor('record_opened')).toEqual([]);

            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('record_opened')).toEqual([{ articleId: 100, dwellMs: 5_000 }]);
        });

        it('records nothing for a pass too short to be a read, nor for a failed load', async () => {
            vi.useFakeTimers({ toFake: ['Date', 'performance'] });
            vi.spyOn(document, 'hasFocus').mockReturnValue(true);
            harness = installIpcHarness({ ...baseResponders(), record_opened: null });
            selectionStore.selectedArticleId = 100;
            render(ReaderPane);
            await flushIpc();
            vi.advanceTimersByTime(400);
            harness.respond('get_article', rejectWith(commandErrorFixture()));
            selectionStore.selectedArticleId = 101;
            await flushIpc();
            vi.advanceTimersByTime(10_000);
            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('record_opened')).toEqual([]);
        });

        it('a brief trip to the background pauses the session; one read stays one open', async () => {
            vi.useFakeTimers({ toFake: ['Date', 'performance', 'setTimeout', 'clearTimeout'] });
            vi.spyOn(document, 'hasFocus').mockReturnValue(true);
            const visibility = vi.spyOn(document, 'visibilityState', 'get');
            visibility.mockReturnValue('visible');
            harness = installIpcHarness({ ...baseResponders(), record_opened: null });
            selectionStore.selectedArticleId = 100;
            render(ReaderPane);
            await vi.advanceTimersByTimeAsync(0);

            vi.advanceTimersByTime(2_000);
            // Covered by a full-screen app for a minute: paused, not ended.
            visibility.mockReturnValue('hidden');
            document.dispatchEvent(new Event('visibilitychange'));
            await vi.advanceTimersByTimeAsync(60_000);
            expect(harness.callsFor('record_opened')).toEqual([]);

            visibility.mockReturnValue('visible');
            document.dispatchEvent(new Event('visibilitychange'));
            vi.advanceTimersByTime(1_600);
            window.dispatchEvent(new Event('pagehide'));
            await vi.advanceTimersByTimeAsync(0);
            expect(harness.callsFor('record_opened')).toEqual([{ articleId: 100, dwellMs: 3_600 }]);
        });

        it('a window that stays in the background ends the session; coming back starts a fresh one', async () => {
            vi.useFakeTimers({ toFake: ['Date', 'performance', 'setTimeout', 'clearTimeout'] });
            vi.spyOn(document, 'hasFocus').mockReturnValue(true);
            const visibility = vi.spyOn(document, 'visibilityState', 'get');
            visibility.mockReturnValue('visible');
            harness = installIpcHarness({ ...baseResponders(), record_opened: null });
            selectionStore.selectedArticleId = 100;
            render(ReaderPane);
            await vi.advanceTimersByTimeAsync(0);

            vi.advanceTimersByTime(2_000);
            visibility.mockReturnValue('hidden');
            document.dispatchEvent(new Event('visibilitychange'));
            await vi.advanceTimersByTimeAsync(HIDDEN_END_MS);
            expect(harness.callsFor('record_opened')).toEqual([{ articleId: 100, dwellMs: 2_000 }]);

            visibility.mockReturnValue('visible');
            document.dispatchEvent(new Event('visibilitychange'));
            vi.advanceTimersByTime(1_600);
            window.dispatchEvent(new Event('pagehide'));
            await vi.advanceTimersByTimeAsync(0);
            expect(harness.callsFor('record_opened')).toEqual([
                { articleId: 100, dwellMs: 2_000 },
                { articleId: 100, dwellMs: 1_600 },
            ]);
        });

        it('stops counting a reader who has gone quiet, and counts again on input', async () => {
            vi.useFakeTimers({ toFake: ['Date', 'performance'] });
            vi.spyOn(document, 'hasFocus').mockReturnValue(true);
            harness = installIpcHarness({ ...baseResponders(), record_opened: null });
            selectionStore.selectedArticleId = 100;
            render(ReaderPane);
            await flushIpc();

            // Walked away (or the Mac slept) for an hour, window in front.
            vi.advanceTimersByTime(60 * 60_000);
            window.dispatchEvent(new KeyboardEvent('keydown', { key: 'ArrowDown' }));
            vi.advanceTimersByTime(3_000);
            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('record_opened')).toEqual([
                { articleId: 100, dwellMs: IDLE_CAP_MS + 3_000 },
            ]);
        });

        it('resumes a half-read article where reading stopped, and saves where it moves to', async () => {
            harness = installIpcHarness({
                ...baseResponders(),
                get_article_state: articleStateFixture({ read_permille: 420 }),
                set_read_progress: true,
            });
            const runFrames = holdFrames();
            selectionStore.selectedArticleId = 100;
            const { container } = render(ReaderPane);
            await flushIpc();
            const body = layOut(container, 5_000, 1_000);
            await runFrames();

            // 42 % of the 4000px extent: back within the same paragraph.
            expect(body.scrollTop).toBe(1_680);
            const fill = container.querySelector('.reading-progress-fill') as HTMLElement;
            expect(fill.style.transform).toBe('scaleX(0.42)');

            body.scrollTop = 3_000;
            await fireEvent.scroll(body);
            expect(fill.style.transform).toBe('scaleX(0.75)');
            // The save waits for the scroll to settle — or the session's end.
            expect(harness.callsFor('set_read_progress')).toEqual([]);
            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('set_read_progress')).toEqual([
                { articleId: 100, readPermille: 750 },
            ]);
        });

        it('opens a finished article at the top, and never saves before resuming', async () => {
            harness = installIpcHarness({
                ...baseResponders(),
                get_article_state: articleStateFixture({ read_permille: 1000 }),
                set_read_progress: true,
            });
            const runFrames = holdFrames();
            selectionStore.selectedArticleId = 100;
            const { container } = render(ReaderPane);
            await flushIpc();
            const body = layOut(container, 5_000, 1_000);
            // A scroll before the stored position is applied is not tracked.
            await fireEvent.scroll(body);
            await runFrames();
            expect(body.scrollTop).toBe(0);

            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('set_read_progress')).toEqual([]);
        });

        it('keeps saving the position when the stored one could not be read', async () => {
            harness = installIpcHarness({
                ...baseResponders(),
                get_article_state: rejectWith(commandErrorFixture()),
                set_read_progress: true,
            });
            const runFrames = holdFrames();
            selectionStore.selectedArticleId = 100;
            const { container } = render(ReaderPane);
            await flushIpc();
            const body = layOut(container, 5_000, 1_000);
            await runFrames();
            expect(body.scrollTop).toBe(0); // nothing to resume

            body.scrollTop = 2_000;
            await fireEvent.scroll(body);
            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness.callsFor('set_read_progress')).toEqual([
                { articleId: 100, readPermille: 500 },
            ]);
        });

        it('shows no progress bar for a body that fits without scrolling', async () => {
            harness = installIpcHarness(baseResponders());
            const runFrames = holdFrames();
            selectionStore.selectedArticleId = 100;
            const { container } = render(ReaderPane);
            await flushIpc();
            layOut(container, 800, 1_000);
            await runFrames();
            expect(container.querySelector('.reading-progress')).toBeNull();
        });

        it('copy link writes the source URL and toasts', async () => {
            const writeText = vi.fn().mockResolvedValue(undefined);
            Object.defineProperty(navigator, 'clipboard', {
                value: { writeText },
                configurable: true,
            });
            harness = installIpcHarness(baseResponders());
            selectionStore.selectedArticleId = 100;
            const { getByRole } = render(ReaderPane);
            await flushIpc();

            await fireEvent.click(getByRole('button', { name: 'Copy link' }));
            await flushIpc();
            expect(writeText).toHaveBeenCalledWith('https://example.org/post/100');
            expect(toastStore.toasts.map((toast) => toast.message)).toContain('Link copied');

            writeText.mockRejectedValueOnce(new Error('denied'));
            await fireEvent.click(getByRole('button', { name: 'Copy link' }));
            await flushIpc();
            expect(toastStore.toasts.map((toast) => toast.message)).toContain(
                'Couldn’t copy the link',
            );
        });
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
        await events.articlesChanged.emit({ feed_id: null, flags: null });
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
            // The neighbours' flags came with the list rows: already cached,
            // so prefetching them costs no state call.
            for (const id of [105, 107, 104, 108]) {
                expect(hasQuery(queryKeys.articleState(id))).toBe(true);
            }
            expect(harness.callsFor('get_article_state')).toEqual([]);
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

            // Archived out of the view: the read-hold lets go of it, and the
            // archive's own flag event takes it out of the (archived-excluding)
            // window in place, so 106 shifts into slot 3 and the selection is no
            // longer in the window.
            const listCalls = harness.callsFor('list_articles').length;
            await fireEvent.click(getByRole('button', { name: 'Archive' }));
            await flush();
            rows = ROWS.filter((row) => row.id !== 107);
            await events.articlesChanged.emit({
                feed_id: null,
                flags: { article_ids: [107], field: 'archived', value: true },
            });
            await flush();
            expect(harness.callsFor('list_articles')).toHaveLength(listCalls);
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

    it('a link-less list row hides both actions before its body lands', async () => {
        // The header renders from the list row while get_article is pending;
        // the row already tells the link-less case apart.
        harness = installIpcHarness({
            ...responders({}),
            get_article: () => new Promise(() => undefined),
            list_articles: [
                articleSummaryFixture({
                    id: 100,
                    title: 'The row',
                    feed_id: 1,
                    source_url: FEED_URL,
                    published_at: null,
                    saved_at: '2026-07-02T10:00:00.000Z',
                }),
            ],
        });
        articlesStore.filters = ALL_ARTICLES;
        void articlesStore.current; // touch to start the list fetch
        await flushIpc();
        selectionStore.selectedArticleId = 100;
        const { container, getByRole, queryByRole } = render(ReaderPane);
        await flushIpc();

        const title = getByRole('heading', { level: 1 });
        expect(title.textContent?.trim()).toBe('The row');
        expect(title.querySelector('a')).toBeNull();
        expect(queryByRole('button', { name: 'Open in browser' })).toBeNull();
        expect(queryByRole('button', { name: 'Load full article' })).toBeNull();
        expect(container.querySelector('.body-skeleton')).not.toBeNull();
        const date = container.querySelector('.reader-meta [data-first-seen]');
        expect(date?.textContent).toMatch(/^First seen: .*2026/);
        // Assistive tech gets the hint the pointer tooltip shows.
        expect(date?.querySelector('.sr-only')?.textContent).toContain(
            'The feed gave no publication date',
        );
    });

    it('a non-http(s) source is never a live link (#262)', async () => {
        harness = installIpcHarness(
            responders({ source_url: 'javascript:alert(1)', feed_id: null }),
        );
        selectionStore.selectedArticleId = 100;
        const { container, getByRole, queryByRole } = render(ReaderPane);
        await flushIpc();

        const title = getByRole('heading', { level: 1 });
        expect(title.textContent?.trim()).toBe('The article');
        expect(container.querySelector('a[href^="javascript"]')).toBeNull();
        expect(title.querySelector('a')).toBeNull();
        expect(queryByRole('button', { name: 'Open in browser' })).toBeNull();
        expect(queryByRole('button', { name: 'Load full article' })).toBeNull();
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
        expect(date?.textContent).toMatch(/^First seen: .*2026/);
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

describe('ReaderPane — read aloud and share (#153)', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        articlePrefetcher.reset();
        resetUiStores();
        vi.unstubAllGlobals();
        Reflect.deleteProperty(navigator, 'share');
        harness?.teardown();
        harness = null;
    });

    async function open(responders: Record<string, Responder> = {}) {
        harness = installIpcHarness({
            ...baseResponders(),
            set_setting: null,
            record_opened: null,
            ...responders,
        });
        selectionStore.selectedArticleId = 100;
        const view = render(ReaderPane);
        await flushIpc();
        return view;
    }

    it('offers no read aloud where the webview cannot speak', async () => {
        const { queryByRole } = await open();
        expect(queryByRole('button', { name: 'Read aloud' })).toBeNull();
    });

    it('reads the title, then the sanitized body, under a play/pause/speed/stop bar', async () => {
        const engine = installSpeech([voice('Samantha', 'en-US')]);
        const { getByRole, queryByRole } = await open();

        await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
        expect(engine.texts()).toEqual(['The article']);
        expect(engine.spoken[0]?.voice?.name).toBe('Samantha');
        expect(getByRole('button', { name: 'Stop reading aloud', pressed: true })).toBeTruthy();

        const bar = getByRole('toolbar', { name: 'Read aloud' });
        expect(bar.textContent).toContain('Sentence 1 of 2');
        engine.finish();
        await tick();
        // The body as rendered: sanitized (no script text), inline markup joined.
        expect(engine.texts()).toEqual(['The article', 'Hello world']);
        expect(bar.textContent).toContain('Sentence 2 of 2');

        await fireEvent.click(within(bar).getByRole('button', { name: 'Pause' }));
        expect(engine.synth.cancel).toHaveBeenCalled();
        await fireEvent.click(within(bar).getByRole('button', { name: 'Resume' }));
        expect(engine.texts()).toEqual(['The article', 'Hello world', 'Hello world']);

        await fireEvent.change(within(bar).getByRole('combobox', { name: 'Speed' }), {
            target: { value: '1.5' },
        });
        await flushIpc();
        expect(engine.spoken.at(-1)?.rate).toBe(1.5);
        expect(harness?.callsFor('set_setting')).toContainEqual({
            key: 'ui.reading.speech-rate',
            value: '1.5',
        });

        await fireEvent.click(within(bar).getByRole('button', { name: 'Stop reading aloud' }));
        expect(queryByRole('toolbar', { name: 'Read aloud' })).toBeNull();
        expect(getByRole('button', { name: 'Read aloud', pressed: false })).toBeTruthy();
    });

    it('reads in the article’s declared language', async () => {
        const engine = installSpeech([voice('Samantha', 'en-US'), voice('Anna', 'de-DE')]);
        const { getByRole } = await open({
            get_article: articleFixture({
                id: 100,
                title: 'Der Artikel',
                content_html: CONTENT,
                lang: 'de',
            }),
        });
        await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
        expect(engine.spoken[0]?.lang).toBe('de');
        expect(engine.spoken[0]?.voice?.name).toBe('Anna');
    });

    it('the toolbar toggle stops the read, and so does moving to another article', async () => {
        const engine = installSpeech();
        const { getByRole, queryByRole } = await open();
        await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
        await fireEvent.click(getByRole('button', { name: 'Stop reading aloud', pressed: true }));
        expect(queryByRole('toolbar', { name: 'Read aloud' })).toBeNull();

        await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
        const cancels = engine.synth.cancel.mock.calls.length;
        selectionStore.selectedArticleId = 101;
        await flushIpc();
        expect(engine.synth.cancel.mock.calls.length).toBeGreaterThan(cancels);
        expect(queryByRole('toolbar', { name: 'Read aloud' })).toBeNull();
        // The late end of the cancelled sentence does not read on.
        engine.finish();
        expect(engine.texts()).toEqual(['The article', 'The article']);
    });

    it('scrolls a sentence that is off screen into view as it is read', async () => {
        installSpeech();
        const rect = (top: number, height: number): DOMRect =>
            ({
                top,
                bottom: top + height,
                height,
                left: 0,
                right: 0,
                width: 0,
                x: 0,
                y: top,
            }) as DOMRect;
        Object.defineProperty(Range.prototype, 'getBoundingClientRect', {
            value: () => rect(1_400, 20),
            configurable: true,
        });
        try {
            const { container, getByRole } = await open();
            const body = container.querySelector('.reader-scroll') as HTMLElement;
            body.getBoundingClientRect = () => rect(0, 600);
            const scrollTo = vi.fn();
            body.scrollTo = scrollTo as unknown as typeof body.scrollTo;
            await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
            expect(scrollTo).toHaveBeenCalledWith({ top: 1_400 - 200 });
        } finally {
            Reflect.deleteProperty(Range.prototype, 'getBoundingClientRect');
        }
    });

    it('a scrolled article is read from the first sentence on screen', async () => {
        const engine = installSpeech();
        const at = (top: number): DOMRect =>
            ({
                top,
                bottom: top + 20,
                height: 20,
                left: 0,
                right: 0,
                width: 0,
                x: 0,
                y: top,
            }) as DOMRect;
        Object.defineProperty(Range.prototype, 'getBoundingClientRect', {
            // The title has scrolled off the top; the body's sentence is in view.
            value(this: Range) {
                return at(this.toString() === 'The article' ? -300 : 100);
            },
            configurable: true,
        });
        try {
            const { container, getByRole } = await open();
            const body = container.querySelector('.reader-scroll') as HTMLElement;
            body.getBoundingClientRect = () =>
                ({ top: 0, bottom: 600, height: 600 }) as unknown as DOMRect;
            const scrollTo = vi.fn();
            body.scrollTo = scrollTo as unknown as typeof body.scrollTo;
            body.scrollTop = 400;
            await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
            expect(engine.texts()).toEqual(['Hello world']);
            expect(getByRole('toolbar', { name: 'Read aloud' }).textContent).toContain(
                'Sentence 2 of 2',
            );
            // Already on screen: the view stays where the reader left it.
            expect(scrollTo).not.toHaveBeenCalled();
        } finally {
            Reflect.deleteProperty(Range.prototype, 'getBoundingClientRect');
        }
    });

    it('a body that re-renders (Load full article) ends the read', async () => {
        const engine = installSpeech();
        const full = articleFixture({
            id: 100,
            title: 'The article',
            content_html: '<p>Full body.</p>',
        });
        harness = installIpcHarness({
            ...baseResponders(),
            record_opened: null,
            load_full_article: full,
        });
        const unwire = await wireInvalidation();
        try {
            selectionStore.selectedArticleId = 100;
            const { getByRole, queryByRole } = render(ReaderPane);
            await flushIpc();
            await fireEvent.click(getByRole('button', { name: 'Read aloud' }));

            // A refetch that brings the same body back leaves the read alone.
            await events.articlesChanged.emit({ feed_id: null, flags: null });
            await flushIpc();
            expect(engine.synth.cancel).not.toHaveBeenCalled();
            expect(getByRole('toolbar', { name: 'Read aloud' })).toBeTruthy();

            // The full article lands (Rust emits ArticlesChanged): a new body.
            await fireEvent.click(getByRole('button', { name: 'Load full article' }));
            harness?.respond('get_article', full);
            await events.articlesChanged.emit({ feed_id: null, flags: null });
            await flushIpc();
            expect(engine.synth.cancel).toHaveBeenCalled();
            expect(queryByRole('toolbar', { name: 'Read aloud' })).toBeNull();
            engine.finish();
            expect(engine.texts()).toEqual(['The article']);
        } finally {
            unwire();
        }
    });

    it('closing the reader, or the page going away, ends the read', async () => {
        const engine = installSpeech();
        const first = await open();
        await fireEvent.click(first.getByRole('button', { name: 'Read aloud' }));
        window.dispatchEvent(new Event('pagehide'));
        await tick();
        expect(engine.synth.cancel).toHaveBeenCalledTimes(1);
        expect(first.queryByRole('toolbar', { name: 'Read aloud' })).toBeNull();

        await fireEvent.click(first.getByRole('button', { name: 'Read aloud' }));
        first.unmount();
        expect(engine.synth.cancel).toHaveBeenCalledTimes(2);
        engine.finish();
        expect(engine.texts()).toEqual(['The article', 'The article']);
    });

    it('listening counts as reading: the idle cap does not cut a long listen short', async () => {
        vi.useFakeTimers({ toFake: ['Date', 'performance'] });
        vi.spyOn(document, 'hasFocus').mockReturnValue(true);
        try {
            const engine = installSpeech();
            const { getByRole } = await open({
                get_article: articleFixture({
                    id: 100,
                    title: 'The article',
                    content_html: '<p>One. Two. Three. Four.</p>',
                }),
            });
            await fireEvent.click(getByRole('button', { name: 'Read aloud' }));
            // No input at all for three minutes, past the idle cap, while each
            // sentence takes a minute to say.
            for (let minute = 0; minute < 3; minute += 1) {
                vi.advanceTimersByTime(60_000);
                engine.finish();
            }
            expect(3 * 60_000).toBeGreaterThan(IDLE_CAP_MS);
            vi.advanceTimersByTime(1_000);
            selectionStore.selectedArticleId = null;
            await flushIpc();
            expect(harness?.callsFor('record_opened')).toEqual([
                { articleId: 100, dwellMs: 3 * 60_000 + 1_000 },
            ]);
        } finally {
            vi.useRealTimers();
            vi.restoreAllMocks();
        }
    });

    it('offers no share where the webview has no share sheet', async () => {
        const { queryByRole } = await open();
        expect(queryByRole('button', { name: 'Share…' })).toBeNull();
    });

    it('share opens the OS share sheet with the article’s URL and title', async () => {
        const share = vi.fn().mockResolvedValue(undefined);
        Object.defineProperty(navigator, 'share', { value: share, configurable: true });
        const { getByRole } = await open();
        await fireEvent.click(getByRole('button', { name: 'Share…' }));
        await flushIpc();
        expect(share).toHaveBeenCalledWith({
            url: 'https://example.org/post/100',
            title: 'The article',
        });
        expect(toastStore.toasts).toHaveLength(0);
    });
});
