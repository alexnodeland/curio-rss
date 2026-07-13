/**
 * List keyboard semantics: the virtualizer is a listbox whose
 * `aria-activedescendant` tracks the selected row, and Home/End/Page keys
 * jump selection while focus stays on the listbox (row-by-row is `j`/`k`).
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import { articleOptionId } from '$components/articles/ArticleRow.svelte';
import { articlesStore } from '$lib/state/articles.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { uiStore } from '$lib/state/ui.svelte';
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

const ARTICLES = Array.from({ length: 6 }, (_, index) =>
    articleSummaryFixture({ id: 2000 + index, title: `Article ${index}` }),
);

function harness(): IpcHarness {
    return installIpcHarness({
        list_articles: ARTICLES,
        list_feeds: [],
        get_article_state: articleStateFixture(),
    });
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
        uiStore.reset();
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

    it('jumps selection with Home and End; arrows are deliberately unbound', async () => {
        installed = harness();
        selectionStore.selectedArticleId = 2000;
        const listbox = await renderList();

        await fireEvent.keyDown(listbox, { key: 'End' });
        expect(selectionStore.selectedArticleId).toBe(2005);
        expect(listbox.getAttribute('aria-activedescendant')).toBe(articleOptionId(2005));

        await fireEvent.keyDown(listbox, { key: 'Home' });
        expect(selectionStore.selectedArticleId).toBe(2000);

        // Arrow keys no longer move selection (j/k are the row-by-row moves).
        await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
        expect(selectionStore.selectedArticleId).toBe(2000);
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
