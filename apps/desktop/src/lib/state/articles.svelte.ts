/**
 * Article-list state: keyset-paginated (`before`-cursor over `id DESC`),
 * with every filter and the sort order backend-owned — this module passes
 * `ListArticlesDto` through and never re-filters or re-sorts client-side.
 * Each filter combination is one query-cache entry holding the concatenated
 * loaded pages; invalidation (Rust-emitted events) reloads the whole loaded
 * range so the reading position survives a background refresh.
 */
import {
    type ArticleSummaryDto,
    type CommandError,
    type ListArticlesDto,
    commands,
} from '$lib/bindings';
import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { type CommandResult, type Query, ensureQuery, queryKeys } from './query-cache.svelte';

/** The UI-facing filter set — `ListArticlesDto` minus the cursor fields. */
export interface ArticleFilters {
    feedId: number | null;
    read: boolean | null;
    starred: boolean | null;
    readLater: boolean | null;
    archived: boolean | null;
    tag: string | null;
}

/** The unfiltered everything view. */
export const ALL_ARTICLES: ArticleFilters = {
    feedId: null,
    read: null,
    starred: null,
    readLater: null,
    archived: null,
    tag: null,
};

export const DEFAULT_PAGE_SIZE = 50;

/** Stable cache-key form of a filter set. */
export function filterKey(filters: ArticleFilters): string {
    const part = (value: boolean | number | string | null): string =>
        value === null ? '-' : String(value);
    return [
        `feed=${part(filters.feedId)}`,
        `read=${part(filters.read)}`,
        `starred=${part(filters.starred)}`,
        `later=${part(filters.readLater)}`,
        `archived=${part(filters.archived)}`,
        `tag=${part(filters.tag)}`,
    ].join(';');
}

function toDto(filters: ArticleFilters, before: number | null, limit: number): ListArticlesDto {
    return {
        feed_id: filters.feedId,
        before,
        limit,
        read: filters.read,
        starred: filters.starred,
        read_later: filters.readLater,
        archived: filters.archived,
        tag: filters.tag,
    };
}

/**
 * One filter combination's loaded window: a query-cache entry whose data is
 * the concatenated pages, plus the cursor bookkeeping around it.
 */
export class ArticleListState {
    readonly filters: ArticleFilters;
    readonly pageSize: number;

    /** No further page exists below the current window. */
    exhausted: boolean = $state(false);

    /** A `loadMore` append is in flight. */
    appending: boolean = $state(false);

    readonly query: Query<ArticleSummaryDto[]>;

    constructor(filters: ArticleFilters, pageSize: number = DEFAULT_PAGE_SIZE) {
        this.filters = filters;
        this.pageSize = pageSize;
        this.query = ensureQuery(
            queryKeys.articleList(filterKey(filters)),
            () => this.#fetchLoadedRange(),
            { feedId: filters.feedId },
        );
    }

    get items(): ArticleSummaryDto[] {
        return this.query.data ?? [];
    }

    get loading(): boolean {
        return this.query.loading;
    }

    get loaded(): boolean {
        return this.query.loaded;
    }

    get error(): CommandError | null {
        return this.query.error;
    }

    /** Fetches the next page below the current window and appends it. */
    async loadMore(): Promise<void> {
        if (this.exhausted || this.appending || this.query.loading) {
            return;
        }
        const window = this.items;
        const cursor = window.length > 0 ? window[window.length - 1].id : null;
        this.appending = true;
        try {
            const result = await commands.listArticles(toDto(this.filters, cursor, this.pageSize));
            if (result.status !== 'ok') {
                return; // the cache entry keeps its data; the next event retries
            }
            if (this.query.loading || this.items !== window) {
                return; // a refetch replaced the window while we appended
            }
            this.query.data = [...window, ...result.data];
            this.exhausted = result.data.length < this.pageSize;
        } finally {
            this.appending = false;
        }
    }

    /**
     * Reloads the whole loaded window page by page — the query-cache fetcher,
     * so event-driven invalidation refreshes what the user actually sees.
     */
    async #fetchLoadedRange(): Promise<CommandResult<ArticleSummaryDto[]>> {
        const target = Math.max(this.query.data?.length ?? 0, this.pageSize);
        const items: ArticleSummaryDto[] = [];
        let cursor: number | null = null;
        for (;;) {
            const result: CommandResult<ArticleSummaryDto[]> = await commands.listArticles(
                toDto(this.filters, cursor, this.pageSize),
            );
            if (result.status !== 'ok') {
                return result;
            }
            items.push(...result.data);
            if (result.data.length < this.pageSize) {
                this.exhausted = true;
                break;
            }
            if (items.length >= target) {
                this.exhausted = false;
                break;
            }
            cursor = items[items.length - 1].id;
        }
        return { status: 'ok', data: items };
    }
}

export class ArticlesStore {
    #lists = new SvelteMap<string, ArticleListState>();

    /** The filter set the main list currently shows. */
    filters: ArticleFilters = $state(ALL_ARTICLES);

    /**
     * The list state for an arbitrary filter combination (cached). Creation
     * is `untrack`ed so templates may reach for `current` directly —
     * registry bookkeeping is not a reactive dependency of the caller.
     */
    list(filters: ArticleFilters): ArticleListState {
        const key = filterKey(filters);
        const existing = this.#lists.get(key);
        if (existing !== undefined) {
            return existing;
        }
        return untrack(() => {
            const created = new ArticleListState(filters);
            this.#lists.set(key, created);
            return created;
        });
    }

    /** The list state for the current filters. */
    get current(): ArticleListState {
        return this.list(this.filters);
    }

    /** Test isolation — drops the list-state registry. */
    reset(): void {
        this.#lists.clear();
        this.filters = ALL_ARTICLES;
    }

    // Mutations are thin pass-throughs: Rust emits `ArticlesChanged` only
    // when state actually flipped (the idempotency `bool`), and the query
    // cache reacts to the event — no optimistic client bookkeeping.

    markRead(articleId: number, read: boolean): Promise<CommandResult<boolean>> {
        return commands.markRead(articleId, read);
    }

    setStarred(articleId: number, starred: boolean): Promise<CommandResult<boolean>> {
        return commands.setStarred(articleId, starred);
    }

    setReadLater(articleId: number, readLater: boolean): Promise<CommandResult<boolean>> {
        return commands.setReadLater(articleId, readLater);
    }

    setArchived(articleId: number, archived: boolean): Promise<CommandResult<boolean>> {
        return commands.setArchived(articleId, archived);
    }

    deleteArticle(articleId: number): Promise<CommandResult<null>> {
        return commands.deleteArticle(articleId);
    }

    recordOpened(articleId: number, dwellMs: number | null): Promise<CommandResult<null>> {
        return commands.recordOpened(articleId, dwellMs);
    }
}

/** The app-wide singleton. */
export const articlesStore = new ArticlesStore();
