/**
 * The first-seen date fallback in the two "home page" list layouts (Reddit
 * cards, YouTube cards): a date-less entry (`published_at` null) shows when
 * Curio first saw it, labelled as such — the same label the list row uses.
 */
import RedditFeed from '$components/articles/RedditFeed.svelte';
import YouTubeGrid from '$components/articles/YouTubeGrid.svelte';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const ROWS = [
    articleSummaryFixture({
        id: 2,
        title: 'Date-less post',
        source_url: 'https://www.reddit.com/r/rust/comments/2/x/',
        published_at: null,
        saved_at: '2026-07-02T12:00:00.000Z',
    }),
    articleSummaryFixture({
        id: 1,
        title: 'Dated post',
        source_url: 'https://www.reddit.com/r/rust/comments/1/y/',
        published_at: '2026-07-01T12:00:00.000Z',
        saved_at: '2026-07-03T10:00:00.000Z',
    }),
];

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

async function renderLayout(layout: typeof RedditFeed | typeof YouTubeGrid): Promise<HTMLElement> {
    harness = installIpcHarness({
        list_articles: ROWS,
        list_feeds: [],
        get_article_state: articleStateFixture(),
    });
    articlesStore.filters = ALL_ARTICLES;
    const { container } = render(layout);
    await flushIpc();
    return container;
}

function cardText(container: HTMLElement, title: string): string {
    const card = [...container.querySelectorAll('.card')].find((node) =>
        node.textContent?.includes(title),
    );
    return card?.textContent ?? '';
}

describe.each([
    ['RedditFeed', RedditFeed],
    ['YouTubeGrid', YouTubeGrid],
] as const)('%s', (_name, layout) => {
    it('dates a date-less entry by its first-seen day, labelled First seen', async () => {
        const container = await renderLayout(layout);
        const marked = container.querySelectorAll('[data-first-seen]');
        expect(marked).toHaveLength(1);
        expect(marked[0]?.textContent).toMatch(/^First seen July 2/);
        expect(cardText(container, 'Date-less post')).toContain('First seen July 2');
    });

    it('shows a dated entry’s publication date, unmarked', async () => {
        const container = await renderLayout(layout);
        const text = cardText(container, 'Dated post');
        expect(text).toContain('July 1');
        expect(text).not.toContain('First seen');
    });
});
