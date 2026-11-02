/**
 * Mark-on-scroll: when the setting is on, rows that scroll up past the top
 * of the list mark themselves read (once each); when off, scrolling touches
 * nothing. The virtualizer reports the scrolled-past count; ArticleList
 * marks the newly passed rows through one silent `mark_read_many` per report.
 */
import ArticleList, { ROW_HEIGHT } from '$components/articles/ArticleList.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    articleSummaryFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const ARTICLES = Array.from({ length: 12 }, (_, index) =>
    articleSummaryFixture({ id: 1000 + index, title: `Article ${index}` }),
);

function harnessWithArticles(): IpcHarness {
    return installIpcHarness({
        list_articles: ARTICLES,
        list_feeds: [],
        get_article_state: articleStateFixture(),
        mark_read_many: (args) => (args.articleIds as number[]).length,
    });
}

/** Scrolls the virtualizer viewport to `scrollTop` and fires the event. */
async function scrollTo(container: HTMLElement, scrollTop: number): Promise<void> {
    const viewport = container.querySelector('.virtual-list') as HTMLElement;
    viewport.scrollTop = scrollTop;
    await fireEvent.scroll(viewport);
    await flushIpc();
}

/** The article ids batched into `mark_read_many` with read=true, in call order. */
function markedIds(harness: IpcHarness): number[] {
    return harness
        .callsFor('mark_read_many')
        .filter((args) => args.read === true)
        .flatMap((args) => args.articleIds as number[]);
}

describe('mark-on-scroll', () => {
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

    it('marks rows read as they scroll past the top when enabled', async () => {
        harness = harnessWithArticles();
        prefsStore.markOnScroll = true;
        const { container } = render(ArticleList);
        await flushIpc();
        await tick();

        // Scroll three rows up out of view: floor(3*ROW_HEIGHT / ROW_HEIGHT) = 3.
        await scrollTo(container, ROW_HEIGHT * 3);
        expect(markedIds(harness)).toEqual([1000, 1001, 1002]);
        // One batch for the three rows that one scroll passed, never a call per row.
        expect(harness.callsFor('mark_read_many')).toHaveLength(1);
        expect(harness.callsFor('mark_read')).toHaveLength(0);

        // Scrolling further only marks the newly passed rows (no repeats).
        await scrollTo(container, ROW_HEIGHT * 5);
        expect(markedIds(harness)).toEqual([1000, 1001, 1002, 1003, 1004]);

        // Scrolling back up marks nothing new — the high-water only advances.
        await scrollTo(container, ROW_HEIGHT);
        expect(markedIds(harness)).toEqual([1000, 1001, 1002, 1003, 1004]);
    });

    it('touches nothing while the setting is off', async () => {
        harness = harnessWithArticles();
        prefsStore.markOnScroll = false;
        const { container } = render(ArticleList);
        await flushIpc();
        await tick();

        await scrollTo(container, ROW_HEIGHT * 4);
        expect(markedIds(harness)).toEqual([]);
    });
});
