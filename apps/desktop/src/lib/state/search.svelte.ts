/**
 * Full-text search state. The query drives `search_articles` (FTS5,
 * phrase-escaped core-side — hostile input cannot break the MATCH grammar);
 * results are a flat, backend-ordered list the middle pane renders in place
 * of the filter list while a search is active. Debounced so a fast typist
 * fires one query, not one per keystroke, and generation-guarded so a slow
 * response never overwrites a newer one.
 */
import { type ArticleSummaryDto, type CommandError, commands } from '$lib/bindings';
import type { CommandResult } from './query-cache.svelte';

/** FTS result cap — enough to scan, small enough to render without paging. */
export const SEARCH_LIMIT = 100;

/** Idle time after the last keystroke before the query fires. */
export const SEARCH_DEBOUNCE_MS = 200;

function internalError(thrown: unknown): CommandResult<never> {
    return {
        status: 'error',
        error: {
            kind: 'internal',
            code: 'INTERNAL',
            message: thrown instanceof Error ? thrown.message : String(thrown),
            recoverable: false,
        },
    };
}

export class SearchStore {
    /** The live query text (bound to the search input). */
    query: string = $state('');

    /** The current FTS result window. */
    results: ArticleSummaryDto[] = $state([]);

    /** A search request is in flight. */
    loading: boolean = $state(false);

    /** The last search failure, if any. */
    error: CommandError | null = $state(null);

    /** Bumped to ask the search input to take focus (the `/` shortcut). */
    focusNonce: number = $state(0);

    #timer: ReturnType<typeof setTimeout> | null = null;
    #generation = 0;

    /** True while a non-empty query scopes the list to search results. */
    get active(): boolean {
        return this.query.trim().length > 0;
    }

    /** Asks the mounted search input to focus (drives the `/` shortcut). */
    requestFocus(): void {
        this.focusNonce += 1;
    }

    /**
     * Updates the query and (debounced) runs it. An empty query cancels any
     * pending/in-flight search and clears the results immediately.
     */
    setQuery(value: string, debounceMs: number = SEARCH_DEBOUNCE_MS): void {
        this.query = value;
        if (this.#timer !== null) {
            clearTimeout(this.#timer);
            this.#timer = null;
        }
        if (value.trim().length === 0) {
            this.#generation += 1; // cancel any in-flight response
            this.results = [];
            this.error = null;
            this.loading = false;
            return;
        }
        if (debounceMs <= 0) {
            void this.#run();
            return;
        }
        this.#timer = setTimeout(() => {
            this.#timer = null;
            void this.#run();
        }, debounceMs);
    }

    async #run(): Promise<void> {
        const query = this.query.trim();
        if (query.length === 0) {
            return;
        }
        this.#generation += 1;
        const generation = this.#generation;
        this.loading = true;
        let result: CommandResult<ArticleSummaryDto[]>;
        try {
            result = await commands.searchArticles(query, SEARCH_LIMIT);
        } catch (thrown) {
            result = internalError(thrown);
        }
        if (generation !== this.#generation) {
            return; // superseded by a newer query
        }
        this.loading = false;
        if (result.status === 'ok') {
            this.results = result.data;
            this.error = null;
        } else {
            this.error = result.error;
        }
    }

    /** Clears the query and results (Escape in the search box, feed switch). */
    clear(): void {
        if (this.#timer !== null) {
            clearTimeout(this.#timer);
            this.#timer = null;
        }
        this.#generation += 1;
        this.query = '';
        this.results = [];
        this.error = null;
        this.loading = false;
    }

    /** Test isolation. */
    reset(): void {
        this.clear();
        this.focusNonce = 0;
    }
}

/** The app-wide singleton. */
export const searchStore = new SearchStore();
