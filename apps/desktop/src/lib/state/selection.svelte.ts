/**
 * Selection state: which feed and article are selected, and which pane owns
 * keyboard focus. Navigation helpers walk the *backend-ordered* current list
 * (never a client re-sort) and report when the selection nears the end of
 * the loaded window so the shell can trigger `loadMore`.
 *
 * Multi-selection (Shift-click / Shift+J/K ranges, ⌘/Ctrl-click toggles)
 * rides alongside: `selectedArticleId` stays the cursor (the row the
 * keyboard sits on), and `selectedIds` is the set the bulk bar acts on. Any
 * plain single selection — a click, j/k, a view switch — goes through the
 * `selectedArticleId` setter, which collapses the set, so a stale
 * multi-selection can never outlive the gesture that replaced it.
 */
import type { ArticleSummaryDto } from '$lib/bindings';
import { SvelteSet } from 'svelte/reactivity';
import { articlesStore } from './articles.svelte';
import { rangeBetween, stillListed } from './multi-select';
import { searchStore } from './search.svelte';

export type FocusZone = 'sidebar' | 'list';

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

    /**
     * Bumped to hand the keyboard to the reader body (Enter / → in the list).
     * The reader focuses its scroll body on this signal — once the selected
     * article has rendered, if it hasn't yet — so ↑/↓/Page keys scroll it.
     */
    readerFocusNonce: number = $state(0);

    /** Selected feed; `null` = the all-feeds views. */
    selectedFeedId: number | null = $state(null);

    /** The cursor row (see `selectedArticleId`). */
    #cursor: number | null = $state(null);

    /** The multi-selection; empty unless two or more rows are selected. */
    readonly #multi = new SvelteSet<number>();

    /** Where a Shift-extension starts (the last plainly selected/toggled row). */
    #anchor: number | null = null;

    /**
     * Whether the list on screen draws a multi-selection — only the article
     * rows do (`ArticleList` turns this on while mounted). Search results and
     * the YouTube / Reddit home layouts mark just the cursor, so there
     * Shift+J/K are plain j/k: a selection nobody can see must never be built
     * for the bulk keys to act on.
     */
    multiSelectable: boolean = $state(false);

    /**
     * Selected article row id (the cursor), if any. Setting it is a plain
     * single selection: it collapses any multi-selection and re-anchors.
     */
    get selectedArticleId(): number | null {
        return this.#cursor;
    }

    set selectedArticleId(id: number | null) {
        this.#cursor = id;
        this.#anchor = id;
        if (this.#multi.size > 0) {
            this.#multi.clear();
        }
    }

    /** Whether more than one row is selected (the bulk bar is showing). */
    get multiActive(): boolean {
        return this.#multi.size > 1;
    }

    /**
     * The rows a bulk action applies to, in list order: the multi-selection,
     * or just the cursor's row when only one is selected.
     */
    get selectedIds(): number[] {
        if (this.#multi.size > 1) {
            const multi = this.#multi;
            return articlesStore.activeItems
                .filter((item) => multi.has(item.id))
                .map((item) => item.id);
        }
        return this.#cursor === null ? [] : [this.#cursor];
    }

    /** Whether row `id` is part of the selection (`aria-selected`). */
    isSelected(id: number): boolean {
        return this.#multi.size > 1 ? this.#multi.has(id) : id === this.#cursor;
    }

    /** Commits a multi-selection; one row (or none) collapses to the cursor. */
    #setMulti(ids: readonly number[]): void {
        this.#multi.clear();
        for (const id of ids) {
            this.#multi.add(id);
        }
        if (this.#multi.size < 2) {
            this.#multi.clear();
        }
    }

    /**
     * Shift-click: selects the contiguous range from the anchor (else the
     * cursor) to `id`, in list order, and moves the cursor there. The anchor
     * stays put, so successive Shift-clicks pivot around it.
     */
    selectRange(id: number): void {
        const range = rangeBetween(
            articlesStore.activeItems,
            this.#anchor ?? this.#cursor ?? id,
            id,
        );
        if (range === null) {
            return;
        }
        this.#anchor = range.anchor;
        this.#cursor = id;
        this.lastPresentIndex = range.index;
        this.#setMulti(range.ids);
    }

    /**
     * ⌘/Ctrl-click: adds `id` to the selection, or takes it out. The first
     * toggle keeps the row already selected, so it builds a set of two. The
     * cursor follows an added row; removing the cursor's row moves it to the
     * most recently added row still selected.
     */
    toggleSelected(id: number): void {
        // Only a cursor still listed seeds the set: one archived out of the
        // view (`selectedIndex === -1`) must not ride along as a hidden member.
        const single = this.#cursor === null || this.selectedIndex === -1 ? [] : [this.#cursor];
        const current = this.#multi.size > 1 ? [...this.#multi] : single;
        const next = current.includes(id)
            ? current.filter((candidate) => candidate !== id)
            : [...current, id];
        if (next.includes(id)) {
            this.#cursor = id;
        } else if (this.#cursor === id) {
            this.#cursor = next.at(-1) ?? null;
        }
        this.#anchor = this.#cursor;
        this.#setMulti(next);
    }

    /**
     * Shift+J / Shift+K: moves the cursor one row and selects the range from
     * the anchor to it — growing or shrinking the selection like Shift+↓/↑ in
     * a file list. With no row selected — or on a list that draws only the
     * cursor ({@link multiSelectable}) — it is a plain j / k. Returns the
     * load-more signal, like j.
     */
    extendSelection(direction: 'down' | 'up'): boolean {
        const items = articlesStore.activeItems;
        const index = this.selectedIndex;
        if (index === -1 || !this.multiSelectable) {
            if (direction === 'up') {
                this.selectPreviousArticle();
                return false;
            }
            return this.selectNextArticle();
        }
        if (this.#multi.size < 2) {
            this.#anchor = this.#cursor;
        }
        const next =
            direction === 'down' ? Math.min(index + 1, items.length - 1) : Math.max(index - 1, 0);
        this.selectRange(items[next].id);
        return direction === 'down' && items.length - next <= LOAD_MORE_MARGIN;
    }

    /** Drops the multi-selection, keeping the cursor's row selected (Esc). */
    clearMulti(): void {
        this.#multi.clear();
        this.#anchor = this.#cursor;
    }

    /**
     * Drops selected ids that have left the list (archived out of the view,
     * a refetch). The list calls it whenever its window changes.
     */
    prune(present: readonly ArticleSummaryDto[]): void {
        if (this.#multi.size === 0) {
            return;
        }
        const kept = stillListed(this.#multi, present);
        if (kept.length !== this.#multi.size) {
            this.#setMulti(kept);
        }
    }

    /**
     * The last window index the selection sat at while present. When the
     * selected row leaves the window (archived out of the view — a row read
     * in an unread-only view is held in place for the session, see
     * `read-hold.ts`), `selectedIndex` goes to -1; j/k resume relative to this captured
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

    /** Asks the reader to take DOM focus on its scroll body (Enter / →). */
    focusReader(): void {
        this.focus = 'list';
        this.readerFocusNonce += 1;
    }

    /** Hands the keyboard to the sidebar tree (`g f`). */
    focusSidebar(): void {
        this.focus = 'sidebar';
        this.sidebarFocusNonce += 1;
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
        this.#multi.clear();
        this.#anchor = null;
        this.multiSelectable = false;
    }
}

/** The app-wide singleton. */
export const selectionStore = new SelectionStore();
