/**
 * Full-text search state. The query drives `search_articles` (Curio's search
 * syntax, compiled to escaped FTS5 core-side — hostile input cannot break the
 * MATCH grammar); results are a flat, backend-ranked list the middle pane
 * renders in place of the filter list while a search is active. Debounced so
 * a fast typist fires one query, not one per keystroke, and
 * generation-guarded so a slow response never overwrites a newer one.
 *
 * A search runs in the current view by default — the feed, folder, tag or
 * built-in view the list shows (see `scope`) — and `setEverywhere(true)`
 * widens it to the whole library, archived included.
 */
import {
    type ArticleSummaryDto,
    type CommandError,
    type SearchArticlesDto,
    commands,
} from '$lib/bindings';
import type { ArticleFilters } from './articles.svelte';
import type { CommandResult } from './query-cache.svelte';

/** FTS result cap — enough to scan, small enough to render without paging. */
export const SEARCH_LIMIT = 100;

/** Idle time after the last keystroke before the query fires. */
export const SEARCH_DEBOUNCE_MS = 200;

/** No filter at all: the whole library, archived articles included. */
export const EVERYWHERE: ArticleFilters = {
    feedId: null,
    read: null,
    starred: null,
    readLater: null,
    archived: null,
    tag: null,
    feedTag: null,
};

/** The `search_articles` request for `query` within `scope`. */
export function searchParams(
    query: string,
    scope: ArticleFilters,
    limit: number = SEARCH_LIMIT,
): SearchArticlesDto {
    return {
        query,
        limit,
        feed_id: scope.feedId,
        read: scope.read,
        starred: scope.starred,
        read_later: scope.readLater,
        archived: scope.archived,
        tag: scope.tag,
        feed_tag: scope.feedTag,
    };
}

function internalError(thrown: unknown): CommandResult<never> {
    return {
        status: 'error',
        error: {
            kind: 'internal',
            code: 'INTERNAL',
            params: {},
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

    /**
     * True between a keystroke and its debounced query actually firing. The
     * gap is ~200ms where `active` is already true but `loading` is not yet —
     * without this the empty-results view flashes "No matches" before the
     * first query even runs.
     */
    pending: boolean = $state(false);

    /** The last search failure, if any. */
    error: CommandError | null = $state(null);

    /** Bumped to ask the search input to take focus (the `/` shortcut). */
    focusNonce: number = $state(0);

    /** The search ignores the current view and covers the whole library. */
    everywhere: boolean = $state(false);

    #timer: ReturnType<typeof setTimeout> | null = null;
    #generation = 0;
    #view: () => ArticleFilters = () => EVERYWHERE;

    /** True while a non-empty query scopes the list to search results. */
    get active(): boolean {
        return this.query.trim().length > 0;
    }

    /**
     * The results window is settled: nothing is pending or in flight, so an
     * empty `results` genuinely means "no matches" rather than "hasn't run
     * yet". Gates the empty state so it never flashes during the debounce gap.
     */
    get settled(): boolean {
        return !this.loading && !this.pending;
    }

    /**
     * The filters a search runs in: the current view's (feed, folder, tag,
     * starred/read-later/archived) — minus its unread-only toggle, which the
     * header hides while searching — or none at all when `everywhere`.
     */
    get scope(): ArticleFilters {
        return this.everywhere ? EVERYWHERE : { ...this.#view(), read: null };
    }

    /**
     * Wires the "current view" a search is scoped to (the article list's
     * filters — `articles.svelte.ts` binds it, so this module needs no
     * runtime import of that one).
     */
    scopeTo(view: () => ArticleFilters): void {
        this.#view = view;
    }

    /** Widens the search to the whole library (or back to the view) and re-runs it. */
    setEverywhere(everywhere: boolean): void {
        if (everywhere === this.everywhere) {
            return;
        }
        this.everywhere = everywhere;
        if (this.active) {
            this.setQuery(this.query, 0);
        }
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
            this.pending = false;
            return;
        }
        this.pending = true;
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
        this.pending = false;
        this.loading = true;
        let result: CommandResult<ArticleSummaryDto[]>;
        try {
            result = await commands.searchArticles(searchParams(query, this.scope));
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

    /**
     * Clears the query and results (Escape in the search box, feed switch).
     * The next search starts scoped to the view again.
     */
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
        this.pending = false;
        this.everywhere = false;
    }

    /** Test isolation. */
    reset(): void {
        this.clear();
        this.focusNonce = 0;
    }
}

/** The app-wide singleton. */
export const searchStore = new SearchStore();
