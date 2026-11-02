/**
 * The keyed query cache — every read the UI renders lives in one of these
 * entries, and every entry is refreshed by Rust-emitted invalidation events
 * (the generated specta event bindings), never by client-side bookkeeping.
 * Rust emits after successful mutations only (idempotent no-ops emit
 * nothing), so an event always means real change. The cache never tails the
 * JSONL event log — that file belongs to the contract, not the UI.
 *
 * Three things keep one change from fanning out across the whole cache
 * (#135):
 * - **Patch in place.** A state-flag flip arrives as an `ArticlesChanged`
 *   naming its articles, flag and value. The cache writes the flag into those
 *   articles' cached state and drops them from lists that now exclude them;
 *   only the lists the flip can add rows to, and the unread counts when they
 *   move, go back to the backend.
 * - **Coalesced refetches.** Invalidations queue up and run once per entry
 *   per microtask batch, and an entry whose fetch is still in flight is not
 *   refetched again on top of it: it refetches once more after that fetch
 *   lands. So a burst of events — each delivered in its own task, like a
 *   folder's per-feed mark-all-read — costs an entry at most two fetches
 *   (the one running and one after it), not one per event.
 * - **Observed vs idle.** An entry knows whether anything reactive reads it.
 *   Once every reader has gone (its row scrolled away, its view closed), an
 *   invalidation only marks it stale; it refetches when it is read again.
 *   The keyed families (`articles:`, `article:`, …) also cap how many idle
 *   entries they keep, evicting the least recently used — so the registry
 *   stays bounded however long the session runs.
 *
 * A flag the user just pressed is **pinned** (`pinArticleFlag`) until its
 * write settles: the press shows at once, and nothing older — an earlier
 * press's event, a fetch that started before the write — can paint the
 * previous value back over it in the meantime.
 */
import {
    events,
    type ArticleStateDto,
    type ArticlesChanged,
    type CommandError,
    type FlagChange,
    type FlagField,
    type ReadProgressChanged,
} from '$lib/bindings';
import { untrack } from 'svelte';
import { SvelteMap, SvelteSet, createSubscriber } from 'svelte/reactivity';

/** The result shape every generated command wrapper resolves to. */
export type CommandResult<T> = { status: 'ok'; data: T } | { status: 'error'; error: CommandError };

/** Metadata used to scope invalidation (article-list queries). */
export interface QueryScope {
    /** The feed a list query is restricted to; `null` = across feeds. */
    feedId?: number | null;
    /**
     * The state flags a list query filters on (`null` = either value), so a
     * flag flip touches only the lists it can change.
     */
    flagFilter?: Readonly<Partial<Record<FlagField, boolean | null>>>;
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

    #data: T | undefined = $state(undefined);
    #error: CommandError | null = $state(null);
    #loading: boolean = $state(false);
    #loaded: boolean = $state(false);

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

    /** Whether a reactive reader (an effect, a template) currently reads it. */
    #observed = false;
    /** Whether one ever did — only such an entry can go idle. */
    #everObserved = false;
    /** Invalidated while idle: refetch once it is read again. */
    #stale = false;
    /** Invalidated while a fetch was in flight: refetch once that one lands. */
    #again = false;
    /** When its data was last written (`writeClock`): a fetch, a landing, a patch. */
    #writtenAt = 0;

    /** Registers the calling reaction as a reader (see `createSubscriber`). */
    readonly #track = createSubscriber(() => {
        this.#observed = true;
        this.#everObserved = true;
        noteObserved(this as Query<unknown>);
        return () => {
            this.#observed = false;
            noteIdle(this as Query<unknown>);
        };
    });

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
     * loaded (and no longer stale), and `start`/`whenLoaded` do not fetch. A
     * fetch still in flight is superseded — `data` is at least as new as
     * anything it started from. Later invalidation refetches it with its own
     * fetcher like any other entry.
     */
    land(data: T): void {
        this.#generation += 1;
        this.#writtenAt = nextWrite();
        this.#data = withPins(this.key, data);
        this.#error = null;
        this.#loading = false;
        this.#loaded = true;
        this.#stale = false;
        this.#latest ??= Promise.resolve();
        this.#followUp();
    }

    /**
     * The keys this query's current data carries, recorded as left to its
     * next landing: that landing lands each one it still carries and
     * settles the rest (refetched when claimed, dropped when not) — and a
     * failed refetch settles them all, so none is left stale.
     */
    defer(): string[] {
        if (this.#carries === undefined || this.#data === undefined) {
            return [];
        }
        const keys = this.#carries(this.#data).map((entry) => entry.key);
        for (const key of keys) {
            this.#deferred.add(key);
        }
        return keys;
    }

    /**
     * Settles the keys left to a landing that is no longer coming (the entry
     * went idle, or was evicted, before its refetch ran), so none stays
     * stale.
     */
    releaseDeferred(): void {
        const deferred = this.#deferred;
        this.#deferred = new SvelteSet();
        for (const key of deferred) {
            settleUncovered(key);
        }
    }

    get data(): T | undefined {
        this.#track();
        return this.#data;
    }

    set data(value: T | undefined) {
        this.#data = value;
    }

    get error(): CommandError | null {
        this.#track();
        return this.#error;
    }

    set error(value: CommandError | null) {
        this.#error = value;
    }

    get loading(): boolean {
        this.#track();
        return this.#loading;
    }

    /** True once any fetch has settled (distinguishes empty from unloaded). */
    get loaded(): boolean {
        this.#track();
        return this.#loaded;
    }

    /** Whether anything reactive reads this entry right now (untracked). */
    get observed(): boolean {
        return this.#observed;
    }

    /** Read once, unread since, and invalidated meanwhile (untracked). */
    get stale(): boolean {
        return this.#stale;
    }

    /** Whether it was read once and nothing reads it now. */
    get idle(): boolean {
        return this.#everObserved && !this.#observed;
    }

    /** Runs the fetcher; a newer concurrent refetch wins (last-write-wins). */
    refetch(): Promise<void> {
        this.#stale = false;
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
        if (this.#stale) {
            // Invalidated while idle: what it holds is known to be outdated.
            await this.refetch();
        }
        while (!this.#loaded) {
            await this.start();
        }
    }

    /**
     * The invalidation step: refetch now, or — while idle — only mark the
     * entry stale, so it refetches when something reads it again. While a
     * fetch is in flight, it refetches once more after that one lands.
     */
    invalidate(): void {
        if (this.idle) {
            this.#stale = true;
            this.releaseDeferred();
            return;
        }
        if (this.#loading) {
            // A fetch is running; one more after it lands covers this
            // invalidation and every other one that arrives meanwhile.
            this.#again = true;
            return;
        }
        void this.refetch();
    }

    /** Runs the refetch an invalidation left for after the last landing. */
    #followUp(): void {
        if (this.#again) {
            this.#again = false;
            scheduleRefetch(this as Query<unknown>);
        }
    }

    /**
     * Rewrites loaded data in place (no IPC). Returns whether there was data
     * to patch; a fetch still in flight may land older data over the patch,
     * so callers reconciling a committed change refetch then as well.
     */
    patch(update: (data: T) => T): boolean {
        const current = this.#data;
        if (current === undefined) {
            return false;
        }
        this.#data = update(current);
        this.#writtenAt = nextWrite();
        return true;
    }

    /** When its data was last written, on the cache's write clock (untracked). */
    get writtenAt(): number {
        return this.#writtenAt;
    }

    /** A fetch has not settled yet (untracked — for bookkeeping). */
    get inFlight(): boolean {
        return this.#loading;
    }

    async #run(): Promise<void> {
        this.#generation += 1;
        const generation = this.#generation;
        const startedAt = nextWrite();
        this.#loading = true;
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
        this.#loading = false;
        this.#loaded = true;
        if (result.status === 'ok') {
            this.#data = withPins(this.key, result.data);
            this.#writtenAt = nextWrite();
            this.#error = null;
        } else {
            this.#error = result.error;
        }
        this.#settleCarried(result, startedAt);
        this.#followUp();
    }

    /**
     * Lands what a landed result carries, then settles every deferred key
     * it no longer carries (all of them when the fetch failed). An entry
     * written since the fetch started (a flag flip patched in meanwhile) keeps
     * its newer data.
     */
    #settleCarried(result: CommandResult<T>, startedAt: number): void {
        const deferred = this.#deferred;
        this.#deferred = new SvelteSet();
        const landed =
            result.status === 'ok' && this.#carries !== undefined
                ? landCarried(this.#carries(result.data), startedAt)
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
 * The cache's write clock: orders an entry's last write against the moment a
 * carrier's fetch started, so a list fetch that began before a flag flip
 * cannot land its older flags over the patch.
 */
let writeClock = 0;

function nextWrite(): number {
    writeClock += 1;
    return writeClock;
}

/**
 * A keyed family whose idle entries are kept in least-recently-used order
 * and evicted beyond `cap`. Entries no reactive reader has picked up yet
 * count as idle too, so a fetch-only entry (a prefetched neighbour) cannot
 * pile up either. Singletons (feeds, unread counts) are never evicted.
 */
interface IdleFamily {
    readonly prefix: string;
    readonly cap: number;
    /** Idle entries by key, oldest first. */
    readonly idle: Map<string, Query<unknown>>;
}

/**
 * The evictable families and how many idle entries each keeps: a handful of
 * list windows to return to, the bodies of the last few articles read, and
 * the tiny flag/tag entries of the rows loaded most recently. List rows carry
 * those last two (`row-details.ts`), so their caps hold a deep scroll's worth
 * of rows: scrolling back over a loaded window costs no call per row.
 */
export const IDLE_CAPS = {
    articles: 16,
    article: 16,
    articleState: 1000,
    articleTags: 1000,
} as const;

const families: readonly IdleFamily[] = [
    { prefix: 'articles:', cap: IDLE_CAPS.articles, idle: new Map() },
    { prefix: 'article:', cap: IDLE_CAPS.article, idle: new Map() },
    { prefix: 'article-state:', cap: IDLE_CAPS.articleState, idle: new Map() },
    { prefix: 'article-tags:', cap: IDLE_CAPS.articleTags, idle: new Map() },
];

function familyOf(key: string): IdleFamily | undefined {
    return families.find((family) => key.startsWith(family.prefix));
}

/** Moves an entry to the most-recently-used end of its family's idle set. */
function touchIdle(query: Query<unknown>): void {
    const family = familyOf(query.key);
    if (family === undefined) {
        return;
    }
    family.idle.delete(query.key);
    family.idle.set(query.key, query);
    evictIdle(family);
}

/** Drops a family's least recently used idle entries beyond its cap. */
function evictIdle(family: IdleFamily): void {
    for (const [key, query] of family.idle) {
        if (family.idle.size <= family.cap) {
            return;
        }
        family.idle.delete(key);
        if (registry.get(key) === query) {
            untrack(() => registry.delete(key));
            query.releaseDeferred();
        }
    }
}

/** A reader picked the entry up: it is no longer evictable, and a stale one refetches. */
function noteObserved(query: Query<unknown>): void {
    familyOf(query.key)?.idle.delete(query.key);
    if (query.stale) {
        // Not from inside the reading reaction: the refetch writes state.
        scheduleRefetch(query);
    }
}

/** Its last reader went away: it may be evicted from now on. */
function noteIdle(query: Query<unknown>): void {
    if (registry.get(query.key) === query) {
        touchIdle(query);
    }
}

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
        if (!existing.observed) {
            untrack(() => touchIdle(existing));
        }
        return existing as Query<T>;
    }
    return untrack(() => {
        const created = new Query(key, fetcher, scope, carries);
        created.claimed = true;
        registry.set(key, created as Query<unknown>);
        touchIdle(created as Query<unknown>);
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
 * costs no call per row. With `readAt` (when the carrier's fetch started on
 * the write clock), an entry written after that keeps its newer data. Returns
 * the keys landed (or kept newer).
 */
export function landCarried(
    entries: readonly CarriedEntry[],
    readAt = Number.POSITIVE_INFINITY,
): ReadonlySet<string> {
    const landed = new SvelteSet<string>();
    untrack(() => {
        for (const entry of entries) {
            landOne(entry, readAt);
            landed.add(entry.key);
        }
    });
    return landed;
}

/** Lands one carried entry, unless it was written after `readAt` (it is newer). */
function landOne({ key, fetcher, data }: CarriedEntry, readAt: number): void {
    const existing = registry.get(key);
    if (existing !== undefined && existing.writtenAt > readAt) {
        return; // already newer than the carrier's read
    }
    const query = existing ?? new Query(key, fetcher);
    if (existing === undefined) {
        registry.set(key, query);
    }
    query.land(data);
    if (!query.observed) {
        touchIdle(query);
    }
}

/**
 * Brings one detail entry current without a carrier: refetched (in the next
 * batch; only marked stale while idle) when something has asked for it,
 * dropped when nothing has (nobody is shown stale data, and the next reader
 * fetches it fresh).
 */
function settleUncovered(key: string): void {
    const query = untrack(() => registry.get(key));
    if (query === undefined) {
        return;
    }
    if (query.claimed) {
        scheduleRefetch(query);
    } else {
        dropQuery(key);
    }
}

/** Whether an entry exists under `key` (read untracked by callers that only
 *  need bookkeeping, e.g. the reader's neighbour prefetch). */
export function hasQuery(key: string): boolean {
    return registry.has(key);
}

/** The entry under `key`, if any, without creating one (untracked). */
export function peekQuery<T>(key: string): Query<T> | undefined {
    return untrack(() => registry.get(key)) as Query<T> | undefined;
}

/** Whether `query` is still the registered entry for its key (untracked). */
export function isCached(query: Query<unknown>): boolean {
    return peekQuery(query.key) === query;
}

/** How many entries are registered whose key starts with `prefix` (tests). */
export function queryCount(prefix = ''): number {
    let count = 0;
    for (const key of untrack(() => [...registry.keys()])) {
        if (key.startsWith(prefix)) {
            count += 1;
        }
    }
    return count;
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
    familyOf(key)?.idle.delete(key);
    registry.delete(key);
}

/** Drops every entry — test isolation. */
export function resetQueryCache(): void {
    registry.clear();
    pending.clear();
    pins.clear();
    for (const family of families) {
        family.idle.clear();
    }
}

/**
 * Entries waiting for the next coalesced refetch batch, each once, in the
 * order queued (a plain set: bookkeeping, never rendered; the registry
 * bounds its size).
 */
// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping touched from inside reactions; reactive reads here would become their dependencies
const pending = new Set<Query<unknown>>();

/**
 * Queues one entry for the next batch. However many events invalidate it
 * before the batch runs, it refetches once.
 */
function scheduleRefetch(query: Query<unknown>): void {
    if (pending.has(query)) {
        return;
    }
    pending.add(query);
    if (pending.size === 1) {
        queueMicrotask(flushRefetches);
    }
}

function flushRefetches(): void {
    const batch = [...pending];
    pending.clear();
    for (const query of batch) {
        // An entry evicted or dropped since it was queued has no reader left;
        // the rows it was left to refresh are settled without it.
        if (isCached(query)) {
            query.invalidate();
        } else {
            query.releaseDeferred();
        }
    }
}

/** Refetches (in the next batch) every query the predicate selects. */
export function invalidateWhere(predicate: (query: Query<unknown>) => boolean): void {
    for (const query of untrack(() => [...registry.values()])) {
        if (predicate(query)) {
            scheduleRefetch(query);
        }
    }
}

/** Refetches (in the next batch) every query whose key starts with `prefix`. */
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

const LIST_PREFIX = queryKeys.articleList('');

/** Whether a list query can hold rows of `feedId` (`null` = any feed). */
function listCoversFeed(query: Query<unknown>, feedId: number | null): boolean {
    return feedId === null || query.scope.feedId === feedId || query.scope.feedId === null;
}

/**
 * Flags the user pressed whose writes are still pending, by state-entry key:
 * the value every landing and every committed event for that entry shows
 * instead of its own until the pin is lifted (plain map: bookkeeping, never
 * rendered).
 */
// eslint-disable-next-line svelte/prefer-svelte-reactivity -- bookkeeping read while landing data; never rendered, so it must not be a reactive dependency
const pins = new Map<string, Partial<Record<FlagField, boolean>>>();

/** `data` with the entry's pinned flags written over it (state entries only). */
function withPins<T>(key: string, data: T): T {
    const pinned = pins.get(key);
    if (pinned === undefined || data === null || typeof data !== 'object') {
        return data;
    }
    return { ...data, ...pinned };
}

/**
 * Pins `field` of an article to the value the user just pressed and shows it
 * (see `patchArticleFlag` for the returned undo). Until `unpinArticleFlag`,
 * an older value arriving — an earlier write's event, a fetch that predates
 * the press — leaves the shown flag alone.
 */
export function pinArticleFlag(
    articleId: number,
    field: FlagField,
    value: boolean,
): (() => void) | null {
    const key = queryKeys.articleState(articleId);
    pins.set(key, { ...pins.get(key), [field]: value });
    return patchArticleFlag(articleId, field, value);
}

/** Lifts a pin once the flag's writes have settled. */
export function unpinArticleFlag(articleId: number, field: FlagField): void {
    const key = queryKeys.articleState(articleId);
    const pinned = pins.get(key);
    if (pinned === undefined) {
        return;
    }
    delete pinned[field];
    if (Object.keys(pinned).length === 0) {
        pins.delete(key);
    }
}

/**
 * Writes one flag into an article's cached state, if it is cached (and not
 * stale). Returns an undo that puts the previous value back — unless
 * something newer (a later flip, an event, a refetch) has rewritten the flag
 * since — or `null` when nothing was cached to patch.
 */
export function patchArticleFlag(
    articleId: number,
    field: FlagField,
    value: boolean,
): (() => void) | null {
    const query = peekQuery<ArticleStateDto>(queryKeys.articleState(articleId));
    const before = flagOf(query, field);
    if (query === undefined || before === undefined) {
        return null;
    }
    setFlagOf(query, field, value);
    return () => {
        if (flagOf(query, field) === value) {
            setFlagOf(query, field, before);
        }
    };
}

/**
 * One flag of a cached state entry, untracked; `undefined` when not loaded —
 * or stale (invalidated while idle), whose value is known to be outdated.
 */
function flagOf(query: Query<ArticleStateDto> | undefined, field: FlagField): boolean | undefined {
    if (query === undefined || query.stale) {
        return undefined;
    }
    // `data?.`: the read for an article purged meanwhile can come back empty.
    return untrack(() => query.data?.[field]);
}

function setFlagOf(query: Query<ArticleStateDto>, field: FlagField, value: boolean): void {
    query.patch((state) => ({ ...state, [field]: value }));
}

/** The cached value of one article flag, or `undefined` when not loaded or stale. */
export function cachedArticleFlag(articleId: number, field: FlagField): boolean | undefined {
    return flagOf(peekQuery<ArticleStateDto>(queryKeys.articleState(articleId)), field);
}

/**
 * A committed flag flip, reconciled into a cached state entry: patched in
 * place, and refetched too when a fetch that may predate the write is still
 * in flight (or nothing has loaded to patch yet). A pinned flag keeps the
 * pressed value: the event is an earlier write's, and the pending write's
 * own event follows.
 */
function reconcileState(articleId: number, change: FlagChange): void {
    const key = queryKeys.articleState(articleId);
    const query = peekQuery<ArticleStateDto>(key);
    if (query === undefined) {
        return;
    }
    const loaded = flagOf(query, change.field) !== undefined;
    if (loaded) {
        setFlagOf(query, change.field, pins.get(key)?.[change.field] ?? change.value);
    }
    if (!loaded || query.inFlight) {
        scheduleRefetch(query as Query<unknown>);
    }
}

/**
 * A committed flag flip, applied to one list window: a list filtering on the
 * other value loses the rows in place; a list filtering on this value may
 * gain rows it never loaded, so it refetches (unless it holds them all); a
 * list that ignores the flag is untouched.
 */
function reconcileList(query: Query<unknown>, change: FlagChange, ids: ReadonlySet<number>): void {
    const wanted = query.scope.flagFilter?.[change.field] ?? null;
    if (wanted === null) {
        return;
    }
    const rows = untrack(() => query.data) as readonly { id: number }[] | undefined;
    if (wanted !== change.value) {
        if (rows?.some((row) => ids.has(row.id))) {
            query.patch((data) => (data as { id: number }[]).filter((row) => !ids.has(row.id)));
        }
        if (query.inFlight) {
            scheduleRefetch(query);
        }
        return;
    }
    const held = new Set(rows?.map((row) => row.id));
    if (rows === undefined || query.inFlight || [...ids].some((id) => !held.has(id))) {
        scheduleRefetch(query);
    }
}

/** Applies a pure state-flag flip (see [`FlagChange`]) to the cache. */
function applyFlagChange(change: FlagChange, feedId: number | null): void {
    const ids = new Set(change.article_ids);
    for (const id of ids) {
        reconcileState(id, change);
    }
    // Unread counts skip read and archived rows; stars and read-later don't move them.
    if (change.field === 'read' || change.field === 'archived') {
        invalidatePrefix(queryKeys.unreadCounts);
    }
    for (const query of untrack(() => [...registry.values()])) {
        if (query.key.startsWith(LIST_PREFIX) && listCoversFeed(query, feedId)) {
            reconcileList(query, change, ids);
        }
    }
}

/** Applies one [`ArticlesChanged`] payload to the cache. */
export function applyArticlesChanged(payload: ArticlesChanged): void {
    if (payload.flags !== null) {
        applyFlagChange(payload.flags, payload.feed_id);
        return;
    }
    invalidatePrefix(queryKeys.unreadCounts);
    invalidatePrefix(queryKeys.tags);
    // Detail queries (article:/article-state:/article-tags:) share the
    // 'article' prefix with 'articles:' lists. A feed-scoped event touches
    // that feed's lists and every unscoped list; an unscoped event touches
    // all of them.
    const all = untrack(() => [...registry.values()]);
    const isList = (query: Query<unknown>) => query.key.startsWith(LIST_PREFIX);
    const lists = all.filter((query) => isList(query) && listCoversFeed(query, payload.feed_id));
    const details = all.filter((query) => query.key.startsWith('article') && !isList(query));
    // A detail entry a refetching list carries (its rows' state and tags) is
    // left to that list: its landing refreshes the entry with no call of its
    // own. An idle list only goes stale, so it covers nothing. Every other
    // detail entry is settled now.
    const covered = new SvelteSet(
        lists.filter((list) => !list.idle).flatMap((list) => list.defer()),
    );
    for (const detail of details) {
        if (!covered.has(detail.key)) {
            settleUncovered(detail.key);
        }
    }
    for (const list of lists) {
        scheduleRefetch(list);
    }
}

/**
 * Applies a `ReadProgressChanged` payload: that one article's state entry
 * refetches — a position save touches no list, count or tag.
 */
export function applyReadProgressChanged(payload: ReadProgressChanged): void {
    invalidateWhere((query) => query.key === queryKeys.articleState(payload.article_id));
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
        events.readProgressChanged.listen((event) => {
            applyReadProgressChanged(event.payload);
        }),
    ]);
    return () => {
        for (const unlisten of unlisteners) {
            unlisten();
        }
    };
}
