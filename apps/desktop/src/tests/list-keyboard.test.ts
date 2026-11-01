/**
 * List keyboard semantics: the virtualizer is a listbox whose
 * `aria-activedescendant` tracks the selected row; ↑/↓ move like k/j and
 * Home/End/Page keys jump while focus stays on the listbox; Enter/→ hand the
 * keyboard to the reader body and ←/Esc hand it back.
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import { articleOptionId } from '$components/articles/ArticleRow.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    flushIpc,
    installIpcHarness,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

const ARTICLES = Array.from({ length: 6 }, (_, index) =>
    articleSummaryFixture({ id: 2000 + index, title: `Article ${index}` }),
);

function harness(): IpcHarness {
    return installIpcHarness({
        list_articles: ARTICLES,
        list_feeds: [],
        get_article: (args) => articleFixture({ id: args.articleId as number }),
        get_article_state: articleStateFixture(),
        mark_read: true,
    });
}

/** The list beside the reader, as the shell lays them out. */
async function renderListAndReader(): Promise<{ listbox: HTMLElement; reader: HTMLElement }> {
    const listbox = await renderList();
    const { container } = render(ReaderPane);
    await flushIpc();
    await tick();
    return { listbox, reader: container };
}

async function renderList(): Promise<HTMLElement> {
    const { container } = render(ArticleList);
    await flushIpc();
    await tick();
    return container.querySelector('.virtual-list') as HTMLElement;
}

describe('list keyboard navigation', () => {
    let installed: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        resetUiStores();
        menuStore.reset();
        installed?.teardown();
        installed = null;
    });

    it('tracks selection via aria-activedescendant', async () => {
        installed = harness();
        selectionStore.selectedArticleId = 2000;
        const listbox = await renderList();
        expect(listbox.getAttribute('aria-activedescendant')).toBe(articleOptionId(2000));
    });

    it('jumps selection with Home and End', async () => {
        installed = harness();
        selectionStore.selectedArticleId = 2000;
        const listbox = await renderList();

        await fireEvent.keyDown(listbox, { key: 'End' });
        expect(selectionStore.selectedArticleId).toBe(2005);
        expect(listbox.getAttribute('aria-activedescendant')).toBe(articleOptionId(2005));

        await fireEvent.keyDown(listbox, { key: 'Home' });
        expect(selectionStore.selectedArticleId).toBe(2000);
    });

    it('ArrowDown / ArrowUp move the selection row by row, like j / k', async () => {
        installed = harness();
        const listbox = await renderList();

        // Nothing selected: ↓ picks the first row, exactly as j would.
        const first = new KeyboardEvent('keydown', {
            key: 'ArrowDown',
            bubbles: true,
            cancelable: true,
        });
        listbox.dispatchEvent(first);
        await tick();
        expect(first.defaultPrevented).toBe(true); // no native line-scroll fight
        expect(selectionStore.selectedArticleId).toBe(2000);

        await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
        await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
        expect(selectionStore.selectedArticleId).toBe(2002);
        expect(listbox.getAttribute('aria-activedescendant')).toBe(articleOptionId(2002));

        await fireEvent.keyDown(listbox, { key: 'ArrowUp' });
        expect(selectionStore.selectedArticleId).toBe(2001);

        // Clamped at the end, like j.
        await fireEvent.keyDown(listbox, { key: 'End' });
        await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
        expect(selectionStore.selectedArticleId).toBe(2005);

        // Modified arrows stay the platform's.
        await fireEvent.keyDown(listbox, { key: 'ArrowUp', metaKey: true });
        expect(selectionStore.selectedArticleId).toBe(2005);
    });

    it('Enter focuses the reader body; Escape hands the keyboard back to the list', async () => {
        installed = harness();
        selectionStore.selectedArticleId = 2001;
        const { listbox, reader } = await renderListAndReader();
        listbox.focus();

        await fireEvent.keyDown(listbox, { key: 'Enter' });
        await flushIpc();
        await tick();
        const body = reader.querySelector('.reader-scroll') as HTMLElement;
        expect(body).toBeTruthy();
        expect(document.activeElement).toBe(body);

        await fireEvent.keyDown(body, { key: 'Escape' });
        await tick();
        expect(document.activeElement).toBe(listbox);
        // Back in the list, not a deselect: the article stays open.
        expect(selectionStore.selectedArticleId).toBe(2001);
    });

    it('→ focuses the reader body (waiting for the article to load); ← returns', async () => {
        installed = harness();
        const { listbox, reader } = await renderListAndReader();
        listbox.focus();

        // Nothing selected: → selects the first row, then focuses its body once it renders.
        await fireEvent.keyDown(listbox, { key: 'ArrowRight' });
        expect(selectionStore.selectedArticleId).toBe(2000);
        await flushIpc();
        await tick();
        const body = reader.querySelector('.reader-scroll') as HTMLElement;
        expect(document.activeElement).toBe(body);

        await fireEvent.keyDown(body, { key: 'ArrowLeft' });
        await tick();
        expect(document.activeElement).toBe(listbox);
    });

    it('the reader body is a named, focusable region', async () => {
        installed = harness();
        selectionStore.selectedArticleId = 2003;
        const { reader } = await renderListAndReader();
        const body = reader.querySelector('.reader-scroll') as HTMLElement;
        expect(body.getAttribute('role')).toBe('region');
        expect(body.tabIndex).toBe(0);
        const labelledBy = body.getAttribute('aria-labelledby') ?? '';
        expect(document.getElementById(labelledBy)?.tagName).toBe('H1');
    });

    it('right-clicking a row opens its context menu with the article actions', async () => {
        installed = harness();
        const listbox = await renderList();
        const row = listbox.querySelector('.article-row') as HTMLElement;

        await fireEvent.contextMenu(row, { clientX: 30, clientY: 40 });
        expect(menuStore.isOpen).toBe(true);
        expect(menuStore.current?.items.map((item) => item.id)).toEqual([
            'star',
            'read',
            'readLater',
            'archive',
            'open',
            'markAllRead',
        ]);
    });

    it('the keyboard menu key opens the menu for the selected row', async () => {
        installed = harness();
        selectionStore.selectedArticleId = 2000;
        const listbox = await renderList();

        await fireEvent.keyDown(listbox, { key: 'ContextMenu' });
        expect(menuStore.isOpen).toBe(true);
        expect(menuStore.current?.invoker?.id).toBe(articleOptionId(2000));
    });
});
