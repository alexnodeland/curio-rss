/**
 * Per-article tags UI over mockIPC: listing, adding (trimmed, via
 * `tag_article`) with event-driven refresh, and removing (via
 * `untag_article`). The tag list rides the query cache like every read.
 */
import ArticleTags from '$components/reader/ArticleTags.svelte';
import { events } from '$lib/bindings';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness } from './ipc-harness';

describe('ArticleTags', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('lists the article tags', async () => {
        harness = installIpcHarness({ get_article_tags: ['rust', 'reading'] });
        const { getByText } = render(ArticleTags, { articleId: 100 });
        await flushIpc();
        expect(getByText('rust')).toBeTruthy();
        expect(getByText('reading')).toBeTruthy();
    });

    it('adds a trimmed tag through tag_article and refreshes on the event', async () => {
        harness = installIpcHarness({ get_article_tags: [], tag_article: true });
        const unwire = await wireInvalidation();
        const { getByLabelText, getByText } = render(ArticleTags, { articleId: 100 });
        await flushIpc();

        const input = getByLabelText('Add tag');
        await fireEvent.input(input, { target: { value: '  rust  ' } });
        await fireEvent.keyDown(input, { key: 'Enter' });
        await flushIpc();
        expect(harness.callsFor('tag_article')).toEqual([{ articleId: 100, tag: 'rust' }]);

        // Rust emits ArticlesChanged after a real tag change → the list refetches.
        harness.respond('get_article_tags', ['rust']);
        await events.articlesChanged.emit({ feed_id: null });
        await flushIpc();
        expect(getByText('rust')).toBeTruthy();
        unwire();
    });

    it('does not fire tag_article for an empty draft', async () => {
        harness = installIpcHarness({ get_article_tags: [], tag_article: true });
        const { getByLabelText } = render(ArticleTags, { articleId: 100 });
        await flushIpc();
        const input = getByLabelText('Add tag');
        await fireEvent.input(input, { target: { value: '   ' } });
        await fireEvent.keyDown(input, { key: 'Enter' });
        await flushIpc();
        expect(harness.callsFor('tag_article')).toHaveLength(0);
    });

    it('removes a tag through untag_article', async () => {
        harness = installIpcHarness({ get_article_tags: ['rust'], untag_article: true });
        const { getByLabelText } = render(ArticleTags, { articleId: 100 });
        await flushIpc();
        await fireEvent.click(getByLabelText('Remove tag rust'));
        await flushIpc();
        expect(harness.callsFor('untag_article')).toEqual([{ articleId: 100, tag: 'rust' }]);
    });
});
