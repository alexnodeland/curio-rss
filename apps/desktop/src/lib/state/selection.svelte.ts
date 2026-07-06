/**
 * Selection state: which feed and article are selected, and which pane owns
 * keyboard focus. Navigation helpers walk the *backend-ordered* current list
 * (never a client re-sort) and report when the selection nears the end of
 * the loaded window so the shell can trigger `loadMore`.
 */
import type { ArticleSummaryDto } from '$lib/bindings';
import { articlesStore } from './articles.svelte';

export type FocusZone = 'sidebar' | 'list' | 'reader';

/** How close to the window's end selection may get before paging in more. */
export const LOAD_MORE_MARGIN = 5;

export class SelectionStore {
    /** The pane that owns j/k and friends right now. */
    focus: FocusZone = $state('list');

    /** Selected feed; `null` = the all-feeds views. */
    selectedFeedId: number | null = $state(null);

    /** Selected article row id, if any. */
    selectedArticleId: number | null = $state(null);

    /** The selected article's row, when it is inside the loaded window. */
    get selectedArticle(): ArticleSummaryDto | null {
        if (this.selectedArticleId === null) {
            return null;
        }
        const id = this.selectedArticleId;
        return articlesStore.current.items.find((item) => item.id === id) ?? null;
    }

    /** Index of the selection in the current window; -1 when absent. */
    get selectedIndex(): number {
        if (this.selectedArticleId === null) {
            return -1;
        }
        const id = this.selectedArticleId;
        return articlesStore.current.items.findIndex((item) => item.id === id);
    }

    /**
     * Moves selection down one row (j). Returns true when the shell should
     * page more rows in (selection entered the load-more margin).
     */
    selectNextArticle(): boolean {
        const items = articlesStore.current.items;
        if (items.length === 0) {
            return false;
        }
        const index = this.selectedIndex;
        const next = index === -1 ? 0 : Math.min(index + 1, items.length - 1);
        this.selectedArticleId = items[next].id;
        return items.length - next <= LOAD_MORE_MARGIN;
    }

    /** Moves selection up one row (k). */
    selectPreviousArticle(): void {
        const items = articlesStore.current.items;
        if (items.length === 0) {
            return;
        }
        const index = this.selectedIndex;
        const previous = index <= 0 ? 0 : index - 1;
        this.selectedArticleId = items[previous].id;
    }

    /** Selects a feed (or all feeds) and re-scopes the article list to it. */
    selectFeed(feedId: number | null): void {
        this.selectedFeedId = feedId;
        this.selectedArticleId = null;
        articlesStore.filters = { ...articlesStore.filters, feedId };
    }

    /** Test isolation. */
    reset(): void {
        this.focus = 'list';
        this.selectedFeedId = null;
        this.selectedArticleId = null;
    }
}

/** The app-wide singleton. */
export const selectionStore = new SelectionStore();
