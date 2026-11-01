/**
 * Article-list state: keyset-paginated (`before`-cursor over `id DESC`),
 * with every filter and the sort order backend-owned — this module passes
 * `ListArticlesDto` through and never re-filters or re-sorts client-side.
 * Each filter combination is one query-cache entry holding the concatenated
 * loaded pages; invalidation (Rust-emitted events) reloads the whole loaded
 * range so the reading position survives a background refresh.
 *
 * On top of the backend window sits the session read-hold (`read-hold.ts`):
 * rows read during a view session stay displayed until the view changes or
 * an explicit refresh, so an unread-only view never yanks the row being read.
 */
import {
    type ArticleSummaryDto,
    type CommandError,
    type ListArticlesDto,
    commands,
} from '$lib/bindings';
import { untrack } from 'svelte';
import { SvelteMap } from 'svelte/reactivity';
import { ListPositions } from './list-position';
import { type CommandResult, type Query, ensureQuery, queryKeys } from './query-cache.svelte';
import { type HeldRow, captureHeld, mergeHeld } from './read-hold';
import { searchStore } from './search.svelte';

/** The UI-facing filter set — `ListArticlesDto` minus the cursor fields. */
export interface ArticleFilters {
    feedId: number | null;
    read: boolean | null;
    starred: boolean | null;
    readLater: boolean | null;
    archived: boolean | null;
    tag: string | null;
    /** Folder scope: the feed's `/`-path tag (or a parent of it). */
    feedTag: string | null;
}

/**
 * The default everything view. `archived: false` keeps archived items out of
 * All / Starred / Read-Later / per-feed — archiving takes an article out of
 * the reading flow; the dedicated Archived view (`archived: true`) is the only
 * place they resurface.
 */
export const ALL_ARTICLES: ArticleFilters = {
    feedId: null,
    read: null,
    starred: null,
    readLater: null,
    archived: false,
    tag: null,
    feedTag: null,
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
        `feedTag=${part(filters.feedTag)}`,
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
        feed_tag: filters.feedTag,
    };
}

/**
 * A limit-1 `list_articles` request for the next unread article in a scope
 * strictly below `before` — the backend-owned query behind next-unread
 * navigation. Forces `read: false` over whatever the scope's read filter was.
 */
export function unreadScopeDto(filters: ArticleFilters, before: number | null): ListArticlesDto {
    return toDto({ ...filters, read: false }, before, 1);
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

    /** The session read-hold: rows read this view session, by id. */
    readonly #held = new SvelteMap<number, HeldRow<ArticleSummaryDto>>();

    /** The displayed rows: the backend window plus any held rows it dropped. */
    #items = $derived.by(() => mergeHeld(this.query.data ?? [], this.#held));

    constructor(filters: ArticleFilters, pageSize: number = DEFAULT_PAGE_SIZE) {
        this.filters = filters;
        this.pageSize = pageSize;
        this.query = ensureQuery(
            queryKeys.articleList(filterKey(filters)),
            () => this.#fetchLoadedRange(),
            { feedId: filters.feedId },
        );
    }

    /**
     * The rows the list displays: the backend window verbatim, plus the held
     * rows (read this session) it no longer returns, each kept in its slot.
     */
    get items(): readonly ArticleSummaryDto[] {
        return this.#items;
    }

    /** Whether `articleId` is held in this view session. */
    isHeld(articleId: number): boolean {
        return this.#held.has(articleId);
    }

    /**
     * Holds a displayed row for the rest of this view session: if a refetch
     * drops it (it was read in an unread-only view), it stays in its slot.
     * A no-op for a row that isn't displayed or is already held.
     */
    hold(articleId: number): void {
        untrack(() => {
            if (this.#held.has(articleId)) {
                return;
            }
            const slot = captureHeld(this.#items, articleId);
            if (slot !== null) {
                this.#held.set(articleId, slot);
            }
        });
    }

    /** Stops holding one row (it was archived — it should leave the list). */
    release(articleId: number): void {
        untrack(() => {
            if (this.#held.has(articleId)) {
                this.#held.delete(articleId);
            }
        });
    }

    /** Ends the view session's hold: the list shows the backend window again. */
    releaseAll(): void {
        untrack(() => {
            if (this.#held.size > 0) {
                this.#held.clear();
            }
        });
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
        // The backend window, not the displayed rows: held rows are not part
        // of the keyset and must never become the cursor.
        const window = this.query.data ?? [];
        const cursor = window.length > 0 ? window[window.length - 1].id : null;
        this.appending = true;
        try {
            const result = await commands.listArticles(toDto(this.filters, cursor, this.pageSize));
            if (result.status !== 'ok') {
                // Surface the page failure through the list's stale-while-
                // revalidate banner (rows are retained; the next refetch clears
                // it) rather than swallowing it into a silent scroll dead stop.
                this.query.error = result.error;
                return;
            }
            if (this.query.loading || (this.query.data ?? []) !== window) {
                return; // a refetch replaced the window while we appended
            }
            this.query.data = [...window, ...result.data];
            this.query.error = null; // a good page clears any prior append banner
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

    #filters: ArticleFilters = $state(ALL_ARTICLES);

    /** Where each view (by `filterKey`) was left — the list restores it on return. */
    readonly positions = new ListPositions();

    /** The filter set the main list currently shows. */
    get filters(): ArticleFilters {
        return this.#filters;
    }

    /**
     * Switching to a different filter combination ends the outgoing view's
     * session: its read-hold is released, so returning to it later starts
     * fresh from the backend window.
     */
    set filters(next: ArticleFilters) {
        untrack(() => {
            const previous = filterKey(this.#filters);
            if (filterKey(next) !== previous) {
                this.#lists.get(previous)?.releaseAll();
            }
        });
        this.#filters = next;
    }

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

    /** Whether the current view is restricted to unread articles. */
    get unreadOnly(): boolean {
        return this.filters.read === false;
    }

    /**
     * Flips the unread-only filter on whatever the list currently shows,
     * preserving every other dimension (feed, starred, tag, …). `read: false`
     * keeps only unread; `null` shows read and unread alike. Filtering is
     * backend-owned — the query-cache window switches via `filterKey`, with no
     * client-side re-filter.
     */
    toggleUnreadOnly(): void {
        this.filters = { ...this.filters, read: this.filters.read === false ? null : false };
    }

    /**
     * The rows the middle pane and selection navigation act on: search
     * results while a search is active, otherwise the current filter
     * window. Short-circuits before touching `current` during search so a
     * search-only context never kicks off a `list_articles` fetch.
     */
    get activeItems(): readonly ArticleSummaryDto[] {
        return searchStore.active ? searchStore.results : this.current.items;
    }

    /** The displayed list for the current filters, without creating one. */
    #shown(): ArticleListState | undefined {
        return untrack(() => this.#lists.get(filterKey(this.#filters)));
    }

    /**
     * Holds a row of the current view for the rest of its session (it is
     * being read: opened in the reader, scrolled past, or toggled).
     */
    holdRead(articleId: number): void {
        this.#shown()?.hold(articleId);
    }

    /** Lets one held row of the current view go (archived out of it). */
    releaseHeld(articleId: number): void {
        this.#shown()?.release(articleId);
    }

    /** Ends the current view's read-hold (an explicit refresh or mark-all-read). */
    releaseHolds(): void {
        this.#shown()?.releaseAll();
    }

    /** Test isolation — drops the list-state registry and scroll memory. */
    reset(): void {
        this.#lists.clear();
        this.positions.reset();
        this.#filters = ALL_ARTICLES;
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
