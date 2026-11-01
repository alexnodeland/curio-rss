/**
 * The whole shell (+page.svelte) over mockIPC: the keyboard registry wired
 * to the action layer — j/k selection with scroll-into-view and load-more
 * at the window edge, o/s/l/m state flips against the backend, the `?`
 * help overlay, chord swallowing, and typing-context/modifier immunity —
 * plus the reading keys: Space / Shift+Space paging the reader body (Space at
 * the end advances to the next unread), `e` archive, `A` mark-view-read, and
 * the list's ARIA keys working from an unfocused page.
 */
import type { ArticleSummaryDto, ListArticlesDto } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { layoutStore } from '$lib/state/layout.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import Page from '../routes/+page.svelte';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    articleStateFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

/** `count` summaries with descending ids starting at `topId`. */
function pageOf(topId: number, count: number): ArticleSummaryDto[] {
    return Array.from({ length: count }, (_, index) => {
        const id = topId - index;
        return articleSummary(id);
    });
}

function articleSummary(id: number): ArticleSummaryDto {
    return {
        id,
        feed_id: 1,
        title: `Article ${id}`,
        source_url: `https://example.org/post/${id}`,
        author: null,
        published_at: '2026-07-01T09:00:00.000Z',
        saved_at: '2026-07-01T10:00:00.000Z',
        word_count: 100,
        lang: 'en',
        image: null,
        snippet: null,
    };
}

function shellResponders(): Record<string, Responder> {
    return {
        list_feeds: [feedFixture({ id: 1, title: 'Alpha' })],
        get_unread_counts: unreadCountsFixture({ total: 1, by_feed: [[1, 1]] }),
        list_articles: (args) => {
            const params = args.params as ListArticlesDto;
            if (params.before === null) {
                return pageOf(500, 50);
            }
            if (params.before === 451) {
                return pageOf(450, 10);
            }
            return [];
        },
        get_article: (args) => articleFixture({ id: args.articleId as number }),
        get_article_state: articleStateFixture(),
        mark_read: true,
        set_starred: true,
        set_read_later: true,
        set_archived: true,
        mark_all_read: 7,
        set_setting: null,
        record_opened: null,
        'plugin:opener|open_url': null,
    };
}

/**
 * Answers the limit-1 unread probe behind next-unread with the article two
 * below `before` (the one in between is read); everything else as usual.
 */
function withNextUnread(responders: Record<string, Responder>): Record<string, Responder> {
    const list = responders.list_articles as (args: Record<string, unknown>) => unknown;
    return {
        ...responders,
        list_articles: (args) => {
            const params = args.params as ListArticlesDto;
            if (params.limit === 1 && params.read === false) {
                return params.before === null
                    ? [articleSummary(500)]
                    : [articleSummary(params.before - 2)];
            }
            return list(args);
        },
    };
}

/** Gives the mounted reader body a real geometry (jsdom lays nothing out). */
function layOut(body: HTMLElement, scrollHeight: number, clientHeight: number): void {
    Object.defineProperty(body, 'scrollHeight', { configurable: true, value: scrollHeight });
    Object.defineProperty(body, 'clientHeight', { configurable: true, value: clientHeight });
}

function readerBody(container: HTMLElement): HTMLElement {
    const body = container.querySelector('.reader-scroll');
    if (!(body instanceof HTMLElement)) {
        throw new Error('no reader body mounted');
    }
    return body;
}

async function mountShell() {
    const mounted = render(Page);
    await flushIpc();
    return mounted;
}

describe('shell keyboard wiring', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        // Unmount first: a mounted tree re-creates queries on cache reset.
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('j/k move the selection over the backend-ordered window', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();

        await fireEvent.keyDown(window, { key: 'j' });
        expect(selectionStore.selectedArticleId).toBe(500);
        await fireEvent.keyDown(window, { key: 'j' });
        expect(selectionStore.selectedArticleId).toBe(499);
        await fireEvent.keyDown(window, { key: 'k' });
        expect(selectionStore.selectedArticleId).toBe(500);
        await fireEvent.keyDown(window, { key: 'k' }); // clamped at the top
        expect(selectionStore.selectedArticleId).toBe(500);

        const selected = container.querySelector('.article-row.selected');
        expect(selected?.textContent).toContain('Article 500');
        expect(selected?.getAttribute('aria-selected')).toBe('true');
    });

    it('selecting via j shows the article in the reader and marks it read', async () => {
        harness = installIpcHarness(shellResponders());
        const { getByRole } = await mountShell();

        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        // The selected body first, then the reader's neighbour prefetch (the
        // top row has no previous neighbour) — and only the selection is marked.
        expect(harness.callsFor('get_article')).toEqual([
            { articleId: 500 },
            { articleId: 499 },
            { articleId: 498 },
        ]);
        expect(harness.callsFor('mark_read')).toEqual([{ articleId: 500, read: true }]);
        expect(getByRole('button', { name: 'Open in browser' })).toBeTruthy();
    });

    it('j at the window edge pages the next keyset page in', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();

        // Park the selection just outside the load-more margin, then j into it.
        const items = articlesStore.current.items;
        expect(items).toHaveLength(50);
        selectionStore.selectedArticleId = items[44]?.id ?? null;
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();

        const calls = harness.callsFor('list_articles');
        expect(calls).toHaveLength(2);
        expect((calls[1]?.params as ListArticlesDto).before).toBe(451);
        expect(articlesStore.current.items).toHaveLength(60);
    });

    it('s / l / m flip star, read-later, and read state on the selection', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();

        await fireEvent.keyDown(window, { key: 's' });
        await flushIpc();
        expect(harness.callsFor('set_starred')).toEqual([{ articleId: 500, starred: true }]);

        await fireEvent.keyDown(window, { key: 'l' });
        await flushIpc();
        expect(harness.callsFor('set_read_later')).toEqual([{ articleId: 500, readLater: true }]);

        await fireEvent.keyDown(window, { key: 'm' });
        await flushIpc();
        // fixture state says unread → m marks read (after the open's auto-mark)
        const markReads = harness.callsFor('mark_read');
        expect(markReads[markReads.length - 1]).toEqual({ articleId: 500, read: true });
    });

    it('o opens the selection externally and records the contract open', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();

        await fireEvent.keyDown(window, { key: 'o' });
        await flushIpc();
        const opens = harness.callsFor('plugin:opener|open_url');
        expect(opens).toHaveLength(1);
        expect(opens[0]?.url).toBe('https://example.org/post/500');
        expect(harness.callsFor('record_opened')).toEqual([{ articleId: 500, dwellMs: null }]);
    });

    it('? toggles the help overlay; Escape closes it; shortcuts stay inert under it', async () => {
        harness = installIpcHarness(shellResponders());
        const { queryByRole, getByRole } = await mountShell();

        expect(queryByRole('dialog')).toBeNull();
        await fireEvent.keyDown(window, { key: '?' });
        expect(getByRole('dialog')).toBeTruthy();

        // The modal owns the keyboard — j must not move the selection.
        const before = selectionStore.selectedArticleId;
        await fireEvent.keyDown(window, { key: 'j' });
        expect(selectionStore.selectedArticleId).toBe(before);

        await fireEvent.keyDown(window, { key: 'Escape' });
        expect(queryByRole('dialog')).toBeNull();

        await fireEvent.keyDown(window, { key: '?' });
        expect(getByRole('dialog')).toBeTruthy();
        await fireEvent.keyDown(window, { key: '?' });
        expect(queryByRole('dialog')).toBeNull();
    });

    it('a dead g-chord swallows the second key instead of firing its single', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();

        await fireEvent.keyDown(window, { key: 'g' });
        await fireEvent.keyDown(window, { key: 's' }); // g-s = starred view (unwired), NOT star
        await flushIpc();
        expect(harness.callsFor('set_starred')).toHaveLength(0);
    });

    it('modifier combinations and typing contexts bypass shortcuts', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();

        await fireEvent.keyDown(window, { key: 'j', ctrlKey: true });
        expect(selectionStore.selectedArticleId).toBeNull();

        const input = document.createElement('input');
        document.body.appendChild(input);
        input.focus();
        await fireEvent.keyDown(input, { key: 'j' });
        expect(selectionStore.selectedArticleId).toBeNull();
        input.remove();
    });

    it('pane separators resize with the keyboard and persist the width', async () => {
        harness = installIpcHarness(shellResponders());
        const { getAllByRole } = await mountShell();

        const startWidth = layoutStore.listWidth;
        const separators = getAllByRole('separator');
        const listSeparator = separators[separators.length - 1];
        expect(listSeparator).toBeTruthy();
        if (listSeparator === undefined) {
            return;
        }
        await fireEvent.keyDown(listSeparator, { key: 'ArrowRight' });
        await flushIpc();
        expect(layoutStore.listWidth).toBe(startWidth + 16);
        expect(harness.callsFor('set_setting')).toContainEqual({
            key: 'ui.panes.list-width',
            value: String(startWidth + 16),
        });
    });

    it('Space pages the reader body; Shift+Space pages back', async () => {
        harness = installIpcHarness(withNextUnread(shellResponders()));
        const { container } = await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        const body = readerBody(container);
        layOut(body, 3000, 800);

        await fireEvent.keyDown(window, { key: ' ' });
        expect(body.scrollTop).toBe(700); // 7/8 of the viewport
        await fireEvent.keyDown(window, { key: ' ' });
        expect(body.scrollTop).toBe(1400);
        await fireEvent.keyDown(window, { key: ' ', shiftKey: true });
        expect(body.scrollTop).toBe(700);
        // Mid-article: the selection never moved.
        expect(selectionStore.selectedArticleId).toBe(500);
    });

    it('Space at the reader bottom selects the next unread', async () => {
        harness = installIpcHarness(withNextUnread(shellResponders()));
        const { container } = await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        const body = readerBody(container);
        layOut(body, 1500, 800);
        body.scrollTop = 700; // the last screenful is showing

        await fireEvent.keyDown(window, { key: ' ' });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBe(498);
        const probe = harness
            .callsFor('list_articles')
            .map((call) => call.params as ListArticlesDto)
            .find((params) => params.limit === 1);
        expect(probe).toMatchObject({ before: 500, read: false });

        // Shift+Space at the top never advances.
        await fireEvent.keyDown(window, { key: ' ', shiftKey: true });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBe(498);
    });

    it('Space with nothing selected opens the first unread', async () => {
        harness = installIpcHarness(withNextUnread(shellResponders()));
        await mountShell();
        await fireEvent.keyDown(window, { key: ' ' });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBe(500);
    });

    it('Space on a focused button stays the button’s', async () => {
        harness = installIpcHarness(withNextUnread(shellResponders()));
        const { getByRole } = await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        const star = getByRole('button', { name: 'Star' });
        star.focus();
        await fireEvent.keyDown(star, { key: ' ' });
        await flushIpc();
        // Not paged into the next unread: the press belonged to the button.
        expect(selectionStore.selectedArticleId).toBe(500);
    });

    it('the reader keeps the keyboard across an article switch', async () => {
        harness = installIpcHarness(withNextUnread(shellResponders()));
        const { container } = await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        const listbox = container.querySelector('[role="listbox"]') as HTMLElement;
        await fireEvent.keyDown(listbox, { key: 'Enter' });
        await flushIpc();
        expect(document.activeElement).toBe(readerBody(container));

        // j remounts the body for the next article; focus follows it.
        await fireEvent.keyDown(readerBody(container), { key: 'j' });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBe(499);
        expect(document.activeElement).toBe(readerBody(container));
    });

    it('e archives the selection', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();
        await fireEvent.keyDown(window, { key: 'j' });
        await flushIpc();
        await fireEvent.keyDown(window, { key: 'e' });
        await flushIpc();
        expect(harness.callsFor('set_archived')).toEqual([{ articleId: 500, archived: true }]);
    });

    it('Shift+A marks the current view read (the whole library from All)', async () => {
        harness = installIpcHarness(shellResponders());
        const { findByText } = await mountShell();
        await fireEvent.keyDown(window, { key: 'A', shiftKey: true });
        await flushIpc();
        expect(harness.callsFor('mark_all_read')).toEqual([{ feedId: null }]);
        expect(await findByText('Marked 7 read')).toBeTruthy();
        // `a` (unshifted) is still Add feed.
        expect(layoutStore.activeModal).toBeNull();
    });

    it('with nothing focused, ↓ moves the selection and Enter opens the reader', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();
        expect(document.activeElement).toBe(document.body);

        await fireEvent.keyDown(document.body, { key: 'ArrowDown' });
        await fireEvent.keyDown(document.body, { key: 'ArrowDown' });
        expect(selectionStore.selectedArticleId).toBe(499);
        await fireEvent.keyDown(document.body, { key: 'Enter' });
        await flushIpc();
        expect(document.activeElement).toBe(readerBody(container));
    });

    it('Caps Lock a is still Add feed, never the bulk mark-view-read', async () => {
        harness = installIpcHarness(shellResponders());
        await mountShell();
        // Caps Lock on, no Shift: the browser reports key 'A'.
        await fireEvent.keyDown(window, { key: 'A' });
        await flushIpc();
        expect(harness.callsFor('mark_all_read')).toEqual([]);
        expect(layoutStore.activeModal).toBe('add-feed');
    });

    it('Esc in the reader body hands focus to the list and keeps the selection', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();
        await fireEvent.keyDown(document.body, { key: 'Enter' });
        await flushIpc();
        const body = readerBody(container);
        expect(document.activeElement).toBe(body);
        expect(selectionStore.selectedArticleId).toBe(500);

        await fireEvent.keyDown(body, { key: 'Escape' });
        await flushIpc();
        // The shell's own Escape step-back (deselect) must not also run.
        expect(selectionStore.selectedArticleId).toBe(500);
        expect(document.activeElement).toBe(container.querySelector('[role="listbox"]'));
    });

    it('after g a clears the selection from the reader, ↓ keeps moving the list', async () => {
        harness = installIpcHarness(shellResponders());
        const { container } = await mountShell();
        await fireEvent.keyDown(document.body, { key: 'Enter' });
        await flushIpc();
        const body = readerBody(container);
        expect(document.activeElement).toBe(body);

        await fireEvent.keyDown(body, { key: 'g' });
        await fireEvent.keyDown(body, { key: 'a' });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBeNull();
        expect(document.activeElement).toBe(document.body);

        // Each press goes where a real one would: the focused element.
        await fireEvent.keyDown(document.activeElement ?? document.body, { key: 'ArrowDown' });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBe(500);
        // The new article's body must not pull focus back into the reader.
        expect(document.activeElement).toBe(document.body);
        await fireEvent.keyDown(document.activeElement ?? document.body, { key: 'ArrowDown' });
        await flushIpc();
        expect(selectionStore.selectedArticleId).toBe(499);
    });
});
