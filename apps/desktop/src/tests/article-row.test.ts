/**
 * The article row over mockIPC, for the two feed-entry gaps core leaves to
 * the UI: a link-less entry (its `source_url` is the feed's own URL) offers no
 * open-in-browser, and a date-less entry (`published_at` null) shows its
 * first-seen date, marked as such.
 */
import ArticleRow, { buildArticleMenu } from '$components/articles/ArticleRow.svelte';
import type { ArticleSummaryDto } from '$lib/bindings';
import { feedsStore } from '$lib/state/feeds.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const FEED_URL = 'https://example.org/feed.xml';

function harness(): IpcHarness {
    return installIpcHarness({
        list_feeds: [feedFixture({ id: 1, url: FEED_URL, title: 'Example Feed' })],
        get_article_state: articleStateFixture(),
    });
}

async function renderRow(article: ArticleSummaryDto): Promise<HTMLElement> {
    const { container } = render(ArticleRow, {
        props: {
            article,
            index: 0,
            rowHeight: 72,
            compact: false,
            setsize: 1,
            selected: false,
            onselect: () => {},
        },
    });
    await flushIpc();
    return container.querySelector('.article-row') as HTMLElement;
}

async function menuIds(row: HTMLElement): Promise<string[] | undefined> {
    await fireEvent.contextMenu(row, { clientX: 10, clientY: 10 });
    return menuStore.current?.items.map((item) => item.id);
}

describe('ArticleRow — link-less and date-less feed entries', () => {
    let installed: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        resetUiStores();
        menuStore.reset();
        installed?.teardown();
        installed = null;
    });

    it('a link-less entry’s context menu has no open-in-browser', async () => {
        installed = harness();
        const row = await renderRow(articleSummaryFixture({ feed_id: 1, source_url: FEED_URL }));
        expect(await menuIds(row)).toEqual(['star', 'read', 'readLater', 'archive', 'markAllRead']);
    });

    it('an entry with its own link keeps open-in-browser', async () => {
        installed = harness();
        const row = await renderRow(articleSummaryFixture({ feed_id: 1 }));
        expect(await menuIds(row)).toContain('open');
    });

    it('buildArticleMenu reads the feed list, so a feedless save keeps the open item', async () => {
        installed = harness();
        feedsStore.prime();
        await flushIpc();
        const linkless = buildArticleMenu(
            articleSummaryFixture({ feed_id: 1, source_url: FEED_URL }),
            undefined,
        );
        expect(linkless.some((item) => item.id === 'open')).toBe(false);
        const feedless = buildArticleMenu(
            articleSummaryFixture({ feed_id: null, source_url: FEED_URL }),
            undefined,
        );
        expect(feedless.some((item) => item.id === 'open')).toBe(true);
    });

    it('a date-less entry shows its first-seen date, marked as first seen', async () => {
        installed = harness();
        const row = await renderRow(
            articleSummaryFixture({ published_at: null, saved_at: '2026-07-02T12:00:00.000Z' }),
        );
        const date = row.querySelector('.row-meta [data-first-seen]');
        expect(date?.textContent).toMatch(/^First seen July 2/);
    });

    it('a dated entry shows its publication date, unmarked', async () => {
        installed = harness();
        const row = await renderRow(
            articleSummaryFixture({
                published_at: '2026-07-01T12:00:00.000Z',
                saved_at: '2026-07-02T10:00:00.000Z',
            }),
        );
        const meta = row.querySelector('.row-meta');
        expect(meta?.querySelector('[data-first-seen]')).toBeNull();
        expect(meta?.textContent).toContain('July 1');
        expect(meta?.textContent).not.toContain('First seen');
    });
});
