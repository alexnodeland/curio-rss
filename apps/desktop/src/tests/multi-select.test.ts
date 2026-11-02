/**
 * Multi-select and bulk actions (#152): Shift-click selects a contiguous
 * range, ⌘/Ctrl-click toggles a row, Shift+↓/↑ (and Shift+J/K) extend from
 * the keyboard, and a plain click or Esc collapses back to one row. The
 * listbox is `aria-multiselectable` with `aria-selected` on exactly the
 * selected rows, and every bulk-bar action is ONE `set_flag_many` IPC call
 * with an Undo that flips back exactly the ids that changed.
 */
import ArticleList from '$components/articles/ArticleList.svelte';
import { selectGesture } from '$components/articles/ArticleRow.svelte';
import ReaderPane from '$components/reader/ReaderPane.svelte';
import type { ArticleSummaryDto } from '$lib/bindings';
import { handleShortcut } from '$lib/state/actions';
import { ALL_ARTICLES, articlesStore } from '$lib/state/articles.svelte';
import { rangeBetween, stillListed } from '$lib/state/multi-select';
import { applyArticlesChanged, resetQueryCache } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
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

const IDS = [306, 305, 304, 303, 302, 301];

/** `set_flag_many` over an in-memory flag table: returns the ids it changed. */
function flagBackend(): Record<string, Set<number>> {
    return { read: new Set(), starred: new Set(), read_later: new Set(), archived: new Set() };
}

/** One article's flags as the backend reports them, on its row or alone. */
function stateOf(flags: Record<string, Set<number>>, id: number) {
    return articleStateFixture({
        read: flags.read.has(id),
        starred: flags.starred.has(id),
        read_later: flags.read_later.has(id),
        archived: flags.archived.has(id),
    });
}

function rows(flags = flagBackend()): ArticleSummaryDto[] {
    return IDS.map((id) =>
        articleSummaryFixture({ id, title: `Article ${id}`, state: stateOf(flags, id) }),
    );
}

function install(flags = flagBackend()): IpcHarness {
    return installIpcHarness({
        list_articles: () => rows(flags),
        list_feeds: [],
        get_article_state: (args) => stateOf(flags, args.articleId as number),
        get_article: (args) => articleFixture({ id: args.articleId as number }),
        mark_read: true,
        list_tags: [],
        get_article_tags: [],
        set_flag_many: (args) => {
            const set = flags[args.flag as string] as Set<number>;
            const value = args.value as boolean;
            return (args.articleIds as number[]).filter((id) => {
                if (set.has(id) === value) return false;
                if (value) set.add(id);
                else set.delete(id);
                return true;
            });
        },
    });
}

function row(container: HTMLElement, id: number): HTMLElement {
    const el = container.querySelector(`#article-option-${id}`);
    if (!(el instanceof HTMLElement)) throw new Error(`row ${id} not rendered`);
    return el;
}

/** The ids whose row reports `aria-selected="true"`, in list order. */
function ariaSelected(container: HTMLElement): number[] {
    return [...container.querySelectorAll('[role="option"][aria-selected="true"]')].map((el) =>
        Number(el.id.replace('article-option-', '')),
    );
}

function bulkButton(container: HTMLElement, action: string): HTMLButtonElement {
    const el = container.querySelector(`[data-testid="bulk-bar"] [data-action="${action}"]`);
    if (!(el instanceof HTMLButtonElement)) throw new Error(`no bulk ${action} button`);
    return el;
}

function undoToastId(): number {
    const toast = toastStore.toasts.find((candidate) => candidate.action?.label === 'Undo');
    if (toast === undefined) throw new Error('no Undo toast');
    return toast.id;
}

describe('multi-select list math', () => {
    const items = IDS.map((id) => ({ id }));

    it('spans the anchor to the target in list order, either direction', () => {
        expect(rangeBetween(items, 305, 302)).toEqual({
            anchor: 305,
            index: 4,
            ids: [305, 304, 303, 302],
        });
        expect(rangeBetween(items, 302, 305)?.ids).toEqual([305, 304, 303, 302]);
        expect(rangeBetween(items, 304, 304)?.ids).toEqual([304]);
    });

    it('collapses to the target when the anchor left the list, and refuses an unlisted target', () => {
        expect(rangeBetween(items, 999, 303)).toEqual({ anchor: 303, index: 3, ids: [303] });
        expect(rangeBetween(items, 305, 999)).toBeNull();
    });

    it('keeps only the selected ids still listed', () => {
        expect(stillListed([301, 999, 305], items)).toEqual([301, 305]);
        expect(stillListed([], items)).toEqual([]);
    });
});

describe('multi-select', () => {
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

    it('Shift-click selects the range from the anchor; aria-selected follows', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();
        const listbox = container.querySelector('[role="listbox"]') as HTMLElement;
        expect(listbox.getAttribute('aria-multiselectable')).toBe('true');

        await fireEvent.click(row(container, 305));
        expect(ariaSelected(container)).toEqual([305]);
        expect(container.querySelector('[data-testid="bulk-bar"]')).toBeNull();

        await fireEvent.click(row(container, 302), { shiftKey: true });
        expect(ariaSelected(container)).toEqual([305, 304, 303, 302]);
        expect(selectionStore.selectedArticleId).toBe(302);
        expect(row(container, 302).classList.contains('cursor')).toBe(true);
        expect(row(container, 305).classList.contains('cursor')).toBe(false);
        const bar = container.querySelector('[data-testid="bulk-bar"]');
        expect(bar?.getAttribute('role')).toBe('toolbar');
        expect(bar?.textContent).toContain('4 selected');

        // The anchor stays put: Shift-clicking above it pivots the range.
        await fireEvent.click(row(container, 306), { shiftKey: true });
        expect(ariaSelected(container)).toEqual([306, 305]);

        // A plain click collapses back to that one row (and the bar goes).
        await fireEvent.click(row(container, 303));
        expect(ariaSelected(container)).toEqual([303]);
        expect(container.querySelector('[data-testid="bulk-bar"]')).toBeNull();
    });

    it('⌘/Ctrl-click toggles rows in and out of the selection', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();

        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 303), { metaKey: true });
        await fireEvent.click(row(container, 301), { ctrlKey: true });
        expect(ariaSelected(container)).toEqual([306, 303, 301]);
        expect(selectionStore.selectedArticleId).toBe(301);

        // Toggling the cursor's row out hands the cursor to a remaining row.
        await fireEvent.click(row(container, 301), { metaKey: true });
        expect(ariaSelected(container)).toEqual([306, 303]);
        expect(selectionStore.selectedArticleId).toBe(303);

        // Down to one: that row is simply the selection again.
        await fireEvent.click(row(container, 306), { metaKey: true });
        expect(selectionStore.multiActive).toBe(false);
        expect(ariaSelected(container)).toEqual([303]);
    });

    it('Shift+↓/↑ in the listbox and Shift+J/K grow and shrink the range', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();
        const listbox = container.querySelector('[role="listbox"]') as HTMLElement;

        await fireEvent.click(row(container, 305));
        await fireEvent.keyDown(listbox, { key: 'ArrowDown', shiftKey: true });
        await fireEvent.keyDown(listbox, { key: 'ArrowDown', shiftKey: true });
        expect(ariaSelected(container)).toEqual([305, 304, 303]);
        await fireEvent.keyDown(listbox, { key: 'ArrowUp', shiftKey: true });
        expect(ariaSelected(container)).toEqual([305, 304]);

        // The global letters run the same actions (the shell's matcher).
        handleShortcut('nav.extendUp');
        handleShortcut('nav.extendUp');
        await flushIpc();
        expect(ariaSelected(container)).toEqual([306, 305]);
        handleShortcut('nav.extendDown');
        handleShortcut('nav.extendDown');
        handleShortcut('nav.extendDown');
        await flushIpc();
        expect(ariaSelected(container)).toEqual([305, 304, 303]);

        // A plain ↓ is a single move again.
        await fireEvent.keyDown(listbox, { key: 'ArrowDown' });
        expect(ariaSelected(container)).toEqual([302]);
    });

    it('bulk mark-read is ONE set_flag_many call over the whole selection, with Undo', async () => {
        const flags = flagBackend();
        harness = install(flags);
        const { container } = render(ArticleList);
        await flushIpc();

        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 301), { shiftKey: true });
        const before = harness.callsFor('mark_read').length;
        await fireEvent.click(bulkButton(container, 'read'));
        await flushIpc();

        expect(harness.callsFor('set_flag_many')).toEqual([
            { articleIds: IDS, flag: 'read', value: true },
        ]);
        expect(harness.callsFor('mark_read')).toHaveLength(before);
        expect([...flags.read].sort()).toEqual([...IDS].sort());
        expect(toastStore.toasts.map((toast) => toast.message)).toContain('Marked 6 read');

        // Undo flips back exactly the ids that changed — one more batch call.
        toastStore.runAction(undoToastId());
        await flushIpc();
        expect(harness.callsFor('set_flag_many').at(-1)).toEqual({
            articleIds: IDS,
            flag: 'read',
            value: false,
        });
        expect(flags.read.size).toBe(0);
        expect(toastStore.toasts.map((toast) => toast.message)).toContain('Restored 6 articles');
    });

    it('s/e over a multi-selection run the bar action; inside the Archived view it unarchives', async () => {
        const flags = flagBackend();
        flags.archived = new Set(IDS);
        harness = install(flags);
        articlesStore.filters = { ...ALL_ARTICLES, archived: true };
        const { container } = render(ArticleList);
        await flushIpc();

        await fireEvent.click(row(container, 305));
        await fireEvent.click(row(container, 304), { shiftKey: true });
        expect(bulkButton(container, 'archive').getAttribute('aria-label')).toBe('Unarchive');
        handleShortcut('article.toggleArchived');
        await flushIpc();
        expect(harness.callsFor('set_flag_many')).toEqual([
            { articleIds: [305, 304], flag: 'archived', value: false },
        ]);
        expect(toastStore.toasts.map((toast) => toast.message)).toContain('Unarchived 2 articles');

        handleShortcut('article.toggleStar');
        await flushIpc();
        expect(harness.callsFor('set_flag_many').at(-1)).toEqual({
            articleIds: [305, 304],
            flag: 'starred',
            value: true,
        });
        expect(harness.callsFor('set_starred')).toEqual([]);
    });

    it('rows that leave the list leave the selection', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();
        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 303), { shiftKey: true });
        expect(selectionStore.selectedIds).toEqual([306, 305, 304, 303]);

        selectionStore.prune(rows().filter((item) => item.id !== 305 && item.id !== 304));
        expect(selectionStore.selectedIds).toEqual([306, 303]);
        selectionStore.prune(rows().filter((item) => item.id !== 306));
        expect(selectionStore.multiActive).toBe(false);
        expect(selectionStore.selectedIds).toEqual([303]);
    });

    it('a view switch drops the multi-selection', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();
        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 304), { shiftKey: true });
        expect(selectionStore.multiActive).toBe(true);

        articlesStore.filters = { ...ALL_ARTICLES, starred: true };
        await flushIpc();
        expect(selectionStore.multiActive).toBe(false);
    });

    it('the reader shows the count instead of an article, and marks nothing read', async () => {
        harness = install();
        const list = render(ArticleList);
        const reader = render(ReaderPane);
        await flushIpc();

        await fireEvent.click(row(list.container, 306), { metaKey: true });
        await fireEvent.click(row(list.container, 304), { metaKey: true });
        await flushIpc();
        expect(selectionStore.multiActive).toBe(true);
        expect(reader.container.textContent).toContain('2 articles selected');
        expect(harness.callsFor('mark_read')).toEqual([]);

        // Back to one row: the reader opens it (and marks it read) as usual.
        selectionStore.clearMulti();
        await flushIpc();
        expect(reader.container.textContent).not.toContain('articles selected');
        expect(harness.callsFor('mark_read')).toEqual([{ articleId: 304, read: true }]);
    });

    it('Shift+J/K are plain j/k once the article rows are off screen (search, home layouts)', async () => {
        harness = install();
        const view = render(ArticleList);
        await flushIpc();
        await fireEvent.click(row(view.container, 305));
        expect(selectionStore.multiSelectable).toBe(true);

        // Search results / the YouTube grid replace the rows: they mark only
        // the cursor, so no hidden range may be built behind them.
        view.unmount();
        expect(selectionStore.multiSelectable).toBe(false);
        handleShortcut('nav.extendDown');
        handleShortcut('nav.extendDown');
        expect(selectionStore.multiActive).toBe(false);
        expect(selectionStore.selectedIds).toEqual([303]);
        handleShortcut('nav.extendUp');
        expect(selectionStore.selectedIds).toEqual([304]);
    });

    it('⌘-click after the open row left the view selects just the clicked row', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();
        await fireEvent.click(row(container, 305));

        // `e` archived 305 out of the inbox: the cursor stays on its id, unlisted.
        harness.respond(
            'list_articles',
            rows().filter((item) => item.id !== 305),
        );
        applyArticlesChanged({ feed_id: null });
        await flushIpc();
        expect(selectionStore.selectedIndex).toBe(-1);

        await fireEvent.click(row(container, 303), { metaKey: true });
        expect(selectionStore.multiActive).toBe(false);
        expect(selectionStore.selectedArticleId).toBe(303);
        expect(selectionStore.selectedIds).toEqual([303]);
        expect(container.querySelector('[data-testid="bulk-bar"]')).toBeNull();
    });

    it('the bulk count lives in a persistent live region, empty until two rows are selected', async () => {
        harness = install();
        const { container } = render(ArticleList);
        await flushIpc();
        const announcer = container.querySelector('[data-testid="bulk-announcer"]');
        expect(announcer?.getAttribute('aria-live')).toBe('polite');
        expect(announcer?.textContent).toBe('');

        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 305), { shiftKey: true });
        // The same node (mounted before the bar) now carries the first count.
        expect(container.querySelector('[data-testid="bulk-announcer"]')).toBe(announcer);
        expect(announcer?.textContent).toBe('2 selected');
        expect(container.querySelector('[data-testid="bulk-bar"] [aria-live]')).toBeNull();

        selectionStore.clearMulti();
        await flushIpc();
        expect(announcer?.textContent).toBe('');
    });

    it('star / read later toggle off when every selected row already has the flag', async () => {
        const flags = flagBackend();
        flags.starred = new Set([306, 305]);
        flags.read_later = new Set([306]);
        harness = install(flags);
        const { container } = render(ArticleList);
        await flushIpc();

        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 305), { shiftKey: true });
        await flushIpc();
        expect(bulkButton(container, 'star').getAttribute('aria-label')).toBe('Unstar');
        // Only one of the two is queued: the bar adds, not removes.
        expect(bulkButton(container, 'readLater').getAttribute('aria-label')).toBe('Read later');

        handleShortcut('article.toggleStar');
        await flushIpc();
        expect(harness.callsFor('set_flag_many')).toEqual([
            { articleIds: [306, 305], flag: 'starred', value: false },
        ]);
        expect(toastStore.toasts.map((toast) => toast.message)).toContain('Unstarred 2 articles');
    });

    it('m over an all-read selection marks it unread; a no-op is not reported as a success', async () => {
        const flags = flagBackend();
        flags.read = new Set([306, 305]);
        harness = install(flags);
        const { container } = render(ArticleList);
        await flushIpc();
        await fireEvent.click(row(container, 306));
        await fireEvent.click(row(container, 305), { shiftKey: true });
        await flushIpc();

        // The bar's explicit Mark read changes nothing: an info note, no Undo.
        await fireEvent.click(bulkButton(container, 'read'));
        await flushIpc();
        const noop = toastStore.toasts.at(-1);
        expect(noop?.message).toBe('Nothing to change');
        expect(noop?.tone).toBe('info');
        expect(noop?.action).toBeUndefined();

        handleShortcut('article.toggleRead');
        await flushIpc();
        expect(harness.callsFor('set_flag_many').at(-1)).toEqual({
            articleIds: [306, 305],
            flag: 'read',
            value: false,
        });
    });
});

describe('selectGesture', () => {
    const click = (init: MouseEventInit) => new MouseEvent('click', init);

    it('Shift is a range and ⌘ a toggle everywhere', () => {
        expect(selectGesture(click({ shiftKey: true }), true)).toBe('range');
        expect(selectGesture(click({ metaKey: true }), true)).toBe('toggle');
        expect(selectGesture(click({ metaKey: true }), false)).toBe('toggle');
        expect(selectGesture(click({}), false)).toBe('single');
    });

    it('Ctrl toggles off macOS only — there ⌃-click is the context-menu click', () => {
        expect(selectGesture(click({ ctrlKey: true }), false)).toBe('toggle');
        expect(selectGesture(click({ ctrlKey: true }), true)).toBe('single');
    });
});
