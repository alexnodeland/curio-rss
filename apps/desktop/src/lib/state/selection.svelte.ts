/**
 * Selection state: which feed and article are selected, and which pane owns
 * keyboard focus. Navigation helpers walk the *backend-ordered* current list
 * (never a client re-sort) and report when the selection nears the end of
 * the loaded window so the shell can trigger `loadMore`.
 */
import type { ArticleSummaryDto } from '$lib/bindings';
import { articlesStore } from './articles.svelte';
import { searchStore } from './search.svelte';

export type FocusZone = 'sidebar' | 'list' | 'reader';

/** How close to the window's end selection may get before paging in more. */
export const LOAD_MORE_MARGIN = 5;

/** Rows a PageUp/PageDown moves the selection by. */
export const PAGE_ROWS = 10;

export class SelectionStore {
    /** The pane that owns j/k and friends right now. */
    focus: FocusZone = $state('list');

    /**
     * Bumped to ask the article listbox to take DOM focus (leaving the
     * sidebar, opening a feed). Mirrors search's `focusNonce`: the listbox
     * `.focus()`es on change, so entering the list is an explicit event, not a
     * side effect of `focus` merely being `'list'` (which it is by default).
     */
    listFocusNonce: number = $state(0);

    /**
     * Bumped to hand the keyboard to the sidebar tree (`g f`). The tree grabs
     * DOM focus only on this signal — never as a side effect of `focus` being
     * `'sidebar'` while some other reactive input (the active row) changes,
     * which used to re-grab focus and make clicking out of the tree snap
     * straight back.
     */
    sidebarFocusNonce: number = $state(0);

    /** Bumped to move DOM focus onto the reader's scroll body (→ from the list),
     *  so ↑/↓ scroll the article. */
    readerFocusNonce: number = $state(0);

    /** Selected feed; `null` = the all-feeds views. */
    selectedFeedId: number | null = $state(null);

    /** Selected article row id, if any. */
    selectedArticleId: number | null = $state(null);

    /**
     * The last window index the selection sat at while present. When the
     * selected row leaves the window (auto-mark-read drops it from an unread
     * view), `selectedIndex` goes to -1; j/k resume relative to this captured
     * position instead of teleporting to the top. ArticleList keeps it current
     * via `rememberIndex` whenever a real row is selected.
     */
    lastPresentIndex: number = $state(0);

    /** The selected article's row, when it is inside the loaded window. */
    get selectedArticle(): ArticleSummaryDto | null {
        if (this.selectedArticleId === null) {
            return null;
        }
        const id = this.selectedArticleId;
        return articlesStore.activeItems.find((item) => item.id === id) ?? null;
    }

    /** Index of the selection in the current window; -1 when absent. */
    get selectedIndex(): number {
        if (this.selectedArticleId === null) {
            return -1;
        }
        const id = this.selectedArticleId;
        return articlesStore.activeItems.findIndex((item) => item.id === id);
    }

    /** Records the window index of the current (present) selection. */
    rememberIndex(index: number): void {
        if (index >= 0) {
            this.lastPresentIndex = index;
        }
    }

    /** The index j/k resume from: the live selection, or — when the selected
     *  row has left the window — its last present position, clamped. */
    private anchorIndex(length: number): number {
        const index = this.selectedIndex;
        return index >= 0 ? index : Math.min(this.lastPresentIndex, length - 1);
    }

    /** Commits a move to `next`, remembering the new index. Returns whether the
     *  shell should page more rows in (selection entered the load-more margin). */
    private moveTo(next: number, length: number): boolean {
        this.selectedArticleId = articlesStore.activeItems[next].id;
        this.lastPresentIndex = next;
        return length - next <= LOAD_MORE_MARGIN;
    }

    /**
     * Moves selection down one row (j). Returns true when the shell should
     * page more rows in (selection entered the load-more margin).
     */
    selectNextArticle(): boolean {
        const items = articlesStore.activeItems;
        if (items.length === 0) {
            return false;
        }
        const index = this.selectedIndex;
        // When the row was dropped from the window, resume *at* the captured
        // position (the row that shifted into the gap); otherwise step forward.
        const next =
            index === -1 ? this.anchorIndex(items.length) : Math.min(index + 1, items.length - 1);
        return this.moveTo(next, items.length);
    }

    /** Moves selection up one row (k). */
    selectPreviousArticle(): void {
        const items = articlesStore.activeItems;
        if (items.length === 0) {
            return;
        }
        const index = this.selectedIndex;
        const anchor = this.anchorIndex(items.length);
        const previous = index === -1 ? Math.max(0, anchor - 1) : anchor <= 0 ? 0 : anchor - 1;
        this.moveTo(previous, items.length);
    }

    /** Moves selection down a page (PageDown). Returns the load-more signal. */
    selectPageDown(): boolean {
        const items = articlesStore.activeItems;
        if (items.length === 0) {
            return false;
        }
        const next = Math.min(this.anchorIndex(items.length) + PAGE_ROWS, items.length - 1);
        return this.moveTo(next, items.length);
    }

    /** Moves selection up a page (PageUp). */
    selectPageUp(): void {
        const items = articlesStore.activeItems;
        if (items.length === 0) {
            return;
        }
        const previous = Math.max(this.anchorIndex(items.length) - PAGE_ROWS, 0);
        this.moveTo(previous, items.length);
    }

    /** Asks the article listbox to take DOM focus (leaving the sidebar). */
    focusList(): void {
        this.focus = 'list';
        this.listFocusNonce += 1;
    }

    /** Hands the keyboard to the sidebar tree (`g f`). */
    focusSidebar(): void {
        this.focus = 'sidebar';
        this.sidebarFocusNonce += 1;
    }

    /** Moves focus to the reader (→ from the list), if an article is open. */
    focusReader(): void {
        if (this.selectedArticleId === null) {
            return;
        }
        this.focus = 'reader';
        this.readerFocusNonce += 1;
    }

    /** Selects a feed (or all feeds) and re-scopes the article list to it. */
    selectFeed(feedId: number | null): void {
        searchStore.clear(); // leaving search: the feed's own list takes over
        this.selectedFeedId = feedId;
        this.selectedArticleId = null;
        this.lastPresentIndex = 0;
        articlesStore.filters = { ...articlesStore.filters, feedId };
    }

    /** Test isolation. */
    reset(): void {
        this.focus = 'list';
        this.listFocusNonce = 0;
        this.sidebarFocusNonce = 0;
        this.readerFocusNonce = 0;
        this.selectedFeedId = null;
        this.selectedArticleId = null;
        this.lastPresentIndex = 0;
    }
}

/** The app-wide singleton. */
export const selectionStore = new SelectionStore();
