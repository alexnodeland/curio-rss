import ArticleList from '$components/articles/ArticleList.svelte';
/**
 * The middle pane over mockIPC: virtualized rendering of the loaded window,
 * keyset `before`-cursor infinite scroll on near-end, per-row state from
 * the backend (unread dot / star), and event-driven refresh of row state.
 */
import type { ArticleSummaryDto, ListArticlesDto } from '$lib/bindings';
import { events } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';

/** `count` summaries with descending ids starting at `topId`. */
function pageOf(topId: number, count: number): ArticleSummaryDto[] {
    return Array.from({ length: count }, (_, index) => {
        const id = topId - index;
        return articleSummaryFixture({
            id,
            title: `Article ${id}`,
            source_url: `https://example.org/post/${id}`,
        });
    });
}

function paramsOf(call: Record<string, unknown>): ListArticlesDto {
    return call.params as ListArticlesDto;
}

describe('ArticleList', () => {
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

    it('renders the loaded window through the virtualizer, not the full page', async () => {
        harness = installIpcHarness({
            list_articles: (args) => (paramsOf(args).before === null ? pageOf(500, 50) : []),
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        const { container } = render(ArticleList);
        await flushIpc();

        const rows = container.querySelectorAll('.article-row');
        expect(rows.length).toBeGreaterThan(0);
        expect(rows.length).toBeLessThan(20); // 600px fallback / 84px rows + overscan
        expect(rows[0]?.textContent).toContain('Article 500');
    });

    it('pages the next keyset page in with a before-cursor as the window nears its end', async () => {
        harness = installIpcHarness({
            list_articles: (args) => {
                const params = paramsOf(args);
                if (params.before === null) {
                    return pageOf(500, 50);
                }
                if (params.before === 451) {
                    return pageOf(450, 20); // short page → exhausted
                }
                return [];
            },
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        const { getByRole } = render(ArticleList);
        await flushIpc();
        expect(articlesStore.current.items).toHaveLength(50);

        const listbox = getByRole('listbox');
        listbox.scrollTop = 50 * 84 - 600; // the bottom of the loaded window
        await fireEvent.scroll(listbox);
        await flushIpc();

        const calls = harness.callsFor('list_articles');
        expect(calls).toHaveLength(2);
        expect(paramsOf(calls[1]).before).toBe(451); // keyset cursor: last loaded id
        expect(articlesStore.current.items).toHaveLength(70);
        expect(articlesStore.current.exhausted).toBe(true);

        // Exhausted: further scrolling asks the backend for nothing more.
        listbox.scrollTop = 70 * 84;
        await fireEvent.scroll(listbox);
        await flushIpc();
        expect(harness.callsFor('list_articles')).toHaveLength(2);
    });

    it('rows surface backend per-article state: unread dot and star', async () => {
        harness = installIpcHarness({
            list_articles: (args) => (paramsOf(args).before === null ? pageOf(500, 2) : []),
            get_article_state: (args) =>
                articleStateFixture({
                    read: args.articleId !== 500,
                    starred: args.articleId === 500,
                }),
            list_feeds: [],
        });
        const { container } = render(ArticleList);
        await flushIpc();

        const rows = [...container.querySelectorAll('.article-row')];
        expect(rows).toHaveLength(2);
        expect(rows[0]?.classList.contains('unread')).toBe(true);
        expect(rows[0]?.textContent).toContain('★');
        expect(rows[1]?.classList.contains('unread')).toBe(false);
        expect(rows[1]?.textContent).not.toContain('★');
    });

    it('row state flips when the Rust-emitted invalidation event lands', async () => {
        harness = installIpcHarness({
            list_articles: (args) => (paramsOf(args).before === null ? pageOf(500, 1) : []),
            get_article_state: articleStateFixture({ read: false }),
            list_feeds: [],
        });
        const unwire = await wireInvalidation();
        const { container } = render(ArticleList);
        await flushIpc();
        expect(container.querySelector('.article-row')?.classList.contains('unread')).toBe(true);

        harness.respond('get_article_state', articleStateFixture({ read: true }));
        await events.articlesChanged.emit({ feed_id: null });
        await flushIpc();
        expect(container.querySelector('.article-row')?.classList.contains('unread')).toBe(false);
        unwire();
    });

    it('shows the empty state for an empty filter result', async () => {
        harness = installIpcHarness({ list_articles: [], list_feeds: [] });
        const { getByText } = render(ArticleList);
        await flushIpc();
        expect(getByText('No articles here')).toBeTruthy();
    });

    it('surfaces user-tier list errors verbatim', async () => {
        harness = installIpcHarness({
            list_articles: rejectWith(
                commandErrorFixture({ kind: 'user', message: 'that tag does not exist' }),
            ),
            list_feeds: [],
        });
        const { getByRole } = render(ArticleList);
        await flushIpc();
        expect(getByRole('alert').textContent).toContain('that tag does not exist');
    });

    it('clicking a row selects the article', async () => {
        harness = installIpcHarness({
            list_articles: (args) => (paramsOf(args).before === null ? pageOf(500, 3) : []),
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        const { container } = render(ArticleList);
        await flushIpc();

        const rows = [...container.querySelectorAll('.article-row')];
        const second = rows[1];
        expect(second).toBeTruthy();
        if (second === undefined) {
            return;
        }
        await fireEvent.click(second);
        expect(selectionStore.selectedArticleId).toBe(499);
        expect(second.getAttribute('aria-selected')).toBe('true');
    });

    it('Enter drills into the reader rather than opening the browser', async () => {
        harness = installIpcHarness({
            list_articles: (args) => (paramsOf(args).before === null ? pageOf(500, 3) : []),
            get_article_state: articleStateFixture(),
            list_feeds: [],
        });
        const { container, getByRole } = render(ArticleList);
        await flushIpc();

        await fireEvent.click(container.querySelector('.article-row') as Element);
        expect(selectionStore.selectedArticleId).not.toBeNull();

        await fireEvent.keyDown(getByRole('listbox'), { key: 'Enter' });
        // Enter moves focus into the reader — it does not open the source in
        // the browser (that is the `o` shortcut / the reader's own button).
        expect(selectionStore.focus).toBe('reader');
        expect(harness.callsFor('record_opened')).toHaveLength(0);
    });
});
