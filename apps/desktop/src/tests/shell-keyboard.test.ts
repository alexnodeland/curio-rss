/**
 * The whole shell (+page.svelte) over mockIPC: the keyboard registry wired
 * to the action layer — j/k selection with scroll-into-view and load-more
 * at the window edge, o/s/l/m state flips against the backend, the `?`
 * help overlay, chord swallowing, and typing-context/modifier immunity.
 */
import type { ArticleSummaryDto, ListArticlesDto } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
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
        set_setting: null,
        record_opened: null,
        'plugin:opener|open_url': null,
    };
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
        uiStore.reset();
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
        expect(harness.callsFor('get_article')).toEqual([{ articleId: 500 }]);
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

        const startWidth = uiStore.listWidth;
        const separators = getAllByRole('separator');
        const listSeparator = separators[separators.length - 1];
        expect(listSeparator).toBeTruthy();
        if (listSeparator === undefined) {
            return;
        }
        await fireEvent.keyDown(listSeparator, { key: 'ArrowRight' });
        await flushIpc();
        expect(uiStore.listWidth).toBe(startWidth + 16);
        expect(harness.callsFor('set_setting')).toContainEqual({
            key: 'ui.panes.list-width',
            value: String(startWidth + 16),
        });
    });
});
