import ReaderPane from '$components/reader/ReaderPane.svelte';
/**
 * The reader pane over mockIPC: article loading through the query cache,
 * mark-read-on-open, the sanitized body (through SanitizedHtml), the action
 * toolbar's state flips with event-driven refresh, and the error tiers.
 */
import { events } from '$lib/bindings';
import { articlesStore } from '$lib/state/articles.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    articleStateFixture,
    commandErrorFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';

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
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('shows the empty state without a selection', () => {
        harness = installIpcHarness({});
        const { getByText } = render(ReaderPane);
        expect(getByText('Select an article to read')).toBeTruthy();
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
});
