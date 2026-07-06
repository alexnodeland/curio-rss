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
import { SvelteMap } from 'svelte/reactivity';

/** The result shape every generated command wrapper resolves to. */
export type CommandResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: CommandError };

/** Optional metadata used to scope invalidation (article-list queries). */
export interface QueryScope {
    /** The feed a list query is restricted to; `null` = across feeds. */
    feedId?: number | null;
}

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

    #fetcher: () => Promise<CommandResult<T>>;
    #generation = 0;

    constructor(key: string, fetcher: () => Promise<CommandResult<T>>, scope: QueryScope = {}) {
        this.key = key;
        this.scope = scope;
        this.#fetcher = fetcher;
    }

    /** Runs the fetcher; a newer concurrent refetch wins (last-write-wins). */
    async refetch(): Promise<void> {
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
): Query<T> {
    const existing = registry.get(key);
    if (existing !== undefined) {
        return existing as Query<T>;
    }
    return untrack(() => {
        const created = new Query(key, fetcher, scope);
        registry.set(key, created as Query<unknown>);
        // Deferred a microtask so a fetcher may close over state its owner
        // finishes constructing after `ensureQuery` returns.
        queueMicrotask(() => void created.refetch());
        return created;
    });
}

/** Drops one entry (e.g. when a filter combination goes out of use). */
export function dropQuery(key: string): void {
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

/** Applies one [`ArticlesChanged`] payload to the cache. */
export function applyArticlesChanged(payload: ArticlesChanged): void {
    invalidatePrefix(queryKeys.unreadCounts);
    invalidatePrefix(queryKeys.tags);
    // Detail queries (article:/article-state:/article-tags:) share the
    // 'article' prefix with 'articles:' lists — handled by scope below.
    invalidateWhere((query) => {
        if (!query.key.startsWith('article')) {
            return false;
        }
        if (!query.key.startsWith(queryKeys.articleList(''))) {
            return true; // detail namespaces: always refresh, they are cheap
        }
        // List queries: a feed-scoped event touches that feed's lists and
        // every unscoped list; an unscoped event touches all of them.
        if (payload.feed_id === null) {
            return true;
        }
        return query.scope.feedId === payload.feed_id || query.scope.feedId === null;
    });
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
