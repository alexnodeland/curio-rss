/**
 * The keyed query cache — every read the UI renders lives in one of these
 * entries, and every entry is refreshed by Rust-emitted invalidation events
 * (the generated specta event bindings), never by client-side bookkeeping.
 * Rust emits after successful mutations only (idempotent no-ops emit
 * nothing), so an event always means real change. The cache never tails the
 * JSONL event log — that file belongs to the contract, not the UI.
 */
import { events, type ArticlesChanged, type CommandError } from '$lib/bindings';
import { untrack } from 'svelte';
import { SvelteMap, SvelteSet } from 'svelte/reactivity';

/** The result shape every generated command wrapper resolves to. */
export type CommandResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: CommandError };

/** Optional metadata used to scope invalidation (article-list queries). */
export interface QueryScope {
    /** The feed a list query is restricted to; `null` = across feeds. */
    feedId?: number | null;
}

/**
 * A detail entry a query's data carries whole — e.g. a list row's
 * `article-state:` flags — so the carrier lands it instead of a fetch.
 * `fetcher` is what the entry uses on its own when no carrier covers it.
 */
export interface CarriedEntry {
    readonly key: string;
    readonly fetcher: () => Promise<CommandResult<unknown>>;
    readonly data: unknown;
}

/** What a query's data carries for other entries (see [`CarriedEntry`]). */
export type Carries<T> = (data: T) => readonly CarriedEntry[];

/**
 * One cached query: reactive data/error/loading plus a refetch. Stale data
 * survives a refetch (stale-while-revalidate) so lists never flash empty.
 */
export class Query<T> {
    readonly key: string;
    readonly scope: QueryScope;

    data: T | undefined = $state(undefined);
    error: CommandError | null = $state(null);
    loading: boolean = $state(false);

    /** True once any fetch has settled (distinguishes empty from unloaded). */
    loaded: boolean = $state(false);

    /**
     * Whether anything has asked for this entry through [`ensureQuery`]. An
     * entry a carrier landed that nobody has read yet is unclaimed: when an
     * invalidation finds it uncovered, it is dropped rather than refetched
     * (a later reader recreates and fetches it), so landing a page's rows
     * never adds per-row refetches for rows nothing renders.
     */
    claimed = false;

    #fetcher: () => Promise<CommandResult<T>>;
    #carries: Carries<T> | undefined;
    #generation = 0;
    /** The newest refetch's promise (the one whose result will land). */
    #latest: Promise<void> | null = null;
    /**
     * Carried keys an invalidation left for this query's next landing to
     * refresh (see [`Query.defer`]).
     */
    #deferred = new SvelteSet<string>();

    /**
     * `carries` names the detail entries this query's data holds whole:
     * each fetch that lands (the newest only, so never a superseded result)
     * lands them too, and an invalidation that refetches this query leaves
     * them to it rather than refetching each one.
     */
    constructor(
        key: string,
        fetcher: () => Promise<CommandResult<T>>,
        scope: QueryScope = {},
        carries?: Carries<T>,
    ) {
        this.key = key;
        this.scope = scope;
        this.#fetcher = fetcher;
        this.#carries = carries;
    }

    /**
     * Lands `data` as if a fetch had returned it, without one: the entry is
     * loaded, and `start`/`whenLoaded` do not fetch. A fetch still in flight
     * is superseded — `data` is at least as new as anything it started from.
     * Later invalidation refetches it with its own fetcher like any other
     * entry.
     */
    land(data: T): void {
        this.#generation += 1;
        this.data = data;
        this.error = null;
        this.loading = false;
        this.loaded = true;
        this.#latest ??= Promise.resolve();
    }

    /**
     * The keys this query's current data carries, recorded as left to its
     * next landing: that landing lands each one it still carries and
     * settles the rest (refetched when claimed, dropped when not) — and a
     * failed refetch settles them all, so none is left stale.
     */
    defer(): string[] {
        if (this.#carries === undefined || this.data === undefined) {
            return [];
        }
        const keys = this.#carries(this.data).map((entry) => entry.key);
        for (const key of keys) {
            this.#deferred.add(key);
        }
        return keys;
    }

    /** Runs the fetcher; a newer concurrent refetch wins (last-write-wins). */
    refetch(): Promise<void> {
        const run = this.#run();
        this.#latest = run;
        return run;
    }

    /**
     * Starts the first fetch unless one has already begun, and returns the
     * newest fetch's promise — so the creation load and an early `whenLoaded`
     * share one fetch instead of racing two.
     */
    start(): Promise<void> {
        return this.#latest ?? this.refetch();
    }

    /**
     * Resolves once a fetch has settled (`loaded`), starting one if none is in
     * flight. A superseded refetch resolves without landing anything, so this
     * keeps awaiting the newest one rather than trusting the first to finish.
     */
    async whenLoaded(): Promise<void> {
        while (!this.loaded) {
            await this.start();
        }
    }

    async #run(): Promise<void> {
        this.#generation += 1;
        const generation = this.#generation;
        this.loading = true;
        let result: CommandResult<T>;
        try {
            result = await this.#fetcher();
        } catch (thrown) {
            // The generated wrappers re-throw real `Error`s (e.g. no Tauri
            // runtime); surface them as an internal error, never a crash.
            result = {
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
        if (generation !== this.#generation) {
            return; // superseded by a newer refetch
        }
        this.loading = false;
        this.loaded = true;
        if (result.status === 'ok') {
            this.data = result.data;
            this.error = null;
        } else {
            this.error = result.error;
        }
        this.#settleCarried(result);
    }

    /**
     * Lands what a landed result carries, then settles every deferred key
     * it no longer carries (all of them when the fetch failed).
     */
    #settleCarried(result: CommandResult<T>): void {
        const deferred = this.#deferred;
        this.#deferred = new SvelteSet();
        const landed =
            result.status === 'ok' && this.#carries !== undefined
                ? landCarried(this.#carries(result.data))
                : new SvelteSet<string>();
        for (const key of deferred) {
            if (!landed.has(key)) {
                settleUncovered(key);
            }
        }
    }
}

const registry = new SvelteMap<string, Query<unknown>>();

/**
 * Returns the cached query under `key`, creating (and immediately loading)
 * it on first use. The fetcher and scope are fixed at creation. Creation is
 * `untrack`ed so components may call this from template expressions —
 * registry bookkeeping is not a reactive dependency of the caller; the
 * reactivity lives in the returned query's fields.
 */
export function ensureQuery<T>(
    key: string,
    fetcher: () => Promise<CommandResult<T>>,
    scope: QueryScope = {},
    carries?: Carries<T>,
): Query<T> {
    const existing = registry.get(key);
    if (existing !== undefined) {
        existing.claimed = true;
        return existing as Query<T>;
    }
    return untrack(() => {
        const created = new Query(key, fetcher, scope, carries);
        created.claimed = true;
        registry.set(key, created as Query<unknown>);
        // Deferred a microtask so a fetcher may close over state its owner
        // finishes constructing after `ensureQuery` returns. `start`, not
        // `refetch`: a `whenLoaded` that already kicked the first fetch off is
        // reused rather than superseded by a second IPC round-trip.
        queueMicrotask(() => void created.start());
        return created;
    });
}

/**
 * Lands each entry's `data` under its key without a fetch: an existing entry
 * takes it (it is the newest read), and a missing one is created loaded with
 * it, unclaimed until someone reads it. This is how a list page that carries
 * each row's details fills those rows' detail entries, so rendering the page
 * costs no call per row. Returns the keys landed.
 */
export function landCarried(entries: readonly CarriedEntry[]): ReadonlySet<string> {
    const landed = new SvelteSet<string>();
    untrack(() => {
        for (const { key, fetcher, data } of entries) {
            let query = registry.get(key);
            if (query === undefined) {
                query = new Query(key, fetcher);
                registry.set(key, query);
            }
            query.land(data);
            landed.add(key);
        }
    });
    return landed;
}

/**
 * Brings one detail entry current without a carrier: refetched when
 * something reads it, dropped when nothing has (nobody is shown stale data,
 * and the next reader fetches it fresh).
 */
function settleUncovered(key: string): void {
    const query = registry.get(key);
    if (query === undefined) {
        return;
    }
    if (query.claimed) {
        void query.refetch();
    } else {
        registry.delete(key);
    }
}

/** Whether an entry exists under `key` (read untracked by callers that only
 *  need bookkeeping, e.g. the reader's neighbour prefetch). */
export function hasQuery(key: string): boolean {
    return registry.has(key);
}

/**
 * Drops one entry (e.g. when a filter combination goes out of use). With
 * `only`, drops it only while the entry is still that exact instance — so an
 * owner evicting what it created never drops a successor someone else made.
 */
export function dropQuery(key: string, only?: Query<unknown>): void {
    if (only !== undefined && registry.get(key) !== only) {
        return;
    }
    registry.delete(key);
}

/** Drops every entry — test isolation. */
export function resetQueryCache(): void {
    registry.clear();
}

/** Refetches every loaded query the predicate selects. */
export function invalidateWhere(predicate: (query: Query<unknown>) => boolean): void {
    for (const query of registry.values()) {
        if (predicate(query)) {
            void query.refetch();
        }
    }
}

/** Refetches every loaded query whose key starts with `prefix`. */
export function invalidatePrefix(prefix: string): void {
    invalidateWhere((query) => query.key.startsWith(prefix));
}

/**
 * Key namespaces — the shared vocabulary between stores and invalidation.
 * Stores mint keys through these so the event→key mapping cannot drift.
 */
export const queryKeys = {
    feeds: 'feeds',
    unreadCounts: 'unread-counts',
    tags: 'tags',
    destinations: 'destinations',
    articleList: (filterKey: string) => `articles:${filterKey}`,
    article: (id: number) => `article:${id}`,
    articleState: (id: number) => `article-state:${id}`,
    articleTags: (id: number) => `article-tags:${id}`,
} as const;

/**
 * Whether an event touches a list query: a feed-scoped event touches that
 * feed's lists and every unscoped list; an unscoped event touches all of
 * them.
 */
function listTouched(query: Query<unknown>, payload: ArticlesChanged): boolean {
    return (
        payload.feed_id === null ||
        query.scope.feedId === payload.feed_id ||
        query.scope.feedId === null
    );
}

/** Applies one [`ArticlesChanged`] payload to the cache. */
export function applyArticlesChanged(payload: ArticlesChanged): void {
    invalidatePrefix(queryKeys.unreadCounts);
    invalidatePrefix(queryKeys.tags);
    // Detail queries (article:/article-state:/article-tags:) share the
    // 'article' prefix with 'articles:' lists.
    const all = [...registry.values()];
    const isList = (query: Query<unknown>) => query.key.startsWith(queryKeys.articleList(''));
    const lists = all.filter((query) => isList(query) && listTouched(query, payload));
    const details = all.filter((query) => query.key.startsWith('article') && !isList(query));
    // A detail entry a refetching list carries (its rows' state and tags) is
    // left to that list: its landing refreshes the entry with no call of its
    // own. Every other detail entry is settled now.
    const covered = new SvelteSet(lists.flatMap((list) => list.defer()));
    for (const detail of details) {
        if (!covered.has(detail.key)) {
            settleUncovered(detail.key);
        }
    }
    for (const list of lists) {
        void list.refetch();
    }
}

/** Applies a `FeedsChanged` signal to the cache. */
export function applyFeedsChanged(): void {
    invalidatePrefix(queryKeys.feeds);
}

/**
 * Subscribes the cache to the Rust-emitted invalidation events via the
 * generated event bindings. Returns an unsubscribe for teardown. Refresh
 * progress events deliberately do not touch the cache — Rust emits
 * `FeedsChanged`/`ArticlesChanged` alongside them when data changed.
 */
export async function wireInvalidation(): Promise<() => void> {
    const unlisteners = await Promise.all([
        events.feedsChanged.listen(() => {
            applyFeedsChanged();
        }),
        events.articlesChanged.listen((event) => {
            applyArticlesChanged(event.payload);
        }),
    ]);
    return () => {
        for (const unlisten of unlisteners) {
            unlisten();
        }
    };
}
