/**
 * Feed-tree state: the subscription list and the unread counts, both
 * backend-owned (`list_feeds`, `get_unread_counts` — SQL `GROUP BY`, never
 * client badge math) and cached in the query cache, which the Rust-emitted
 * events keep fresh. Refresh progress rides the `RefreshProgress` /
 * `RefreshFinished` events into reactive fields for the progress UI.
 */
import {
    events,
    type CommandError,
    type FeedDto,
    type FeedStatusDto,
    type NewFeedDto,
    type RefreshOutcomeDto,
    commands,
} from '$lib/bindings';
import { type CommandResult, type Query, ensureQuery, queryKeys } from './query-cache.svelte';

export class FeedsStore {
    /** True while a `refresh_all` sweep is in flight. */
    refreshing: boolean = $state(false);

    /** Per-feed outcomes of the current/last sweep, in refresh order. */
    refreshOutcomes: RefreshOutcomeDto[] = $state([]);

    get #feedsQuery(): Query<FeedDto[]> {
        return ensureQuery(queryKeys.feeds, commands.listFeeds);
    }

    get #countsQuery() {
        return ensureQuery(queryKeys.unreadCounts, commands.getUnreadCounts);
    }

    /**
     * Creates the feed + unread-count queries ahead of rendering. Svelte
     * excludes state created inside a reaction from that reaction's
     * dependencies, so a component must prime in init (or a $derived) and
     * only read from its template.
     */
    prime(): void {
        void this.#feedsQuery;
        void this.#countsQuery;
    }

    get feeds(): FeedDto[] {
        return this.#feedsQuery.data ?? [];
    }

    get loading(): boolean {
        return this.#feedsQuery.loading;
    }

    get loaded(): boolean {
        return this.#feedsQuery.loaded;
    }

    get error(): CommandError | null {
        return this.#feedsQuery.error;
    }

    /** Total unread across (and outside) feeds — backend-computed. */
    get unreadTotal(): number {
        return this.#countsQuery.data?.total ?? 0;
    }

    /** Backend-computed unread count for one feed. */
    unreadFor(feedId: number): number {
        const pair = this.#countsQuery.data?.by_feed.find(([id]) => id === feedId);
        return pair?.[1] ?? 0;
    }

    /** Subscribes to a feed. Invalidation rides the Rust-emitted event. */
    addFeed(newFeed: NewFeedDto): Promise<CommandResult<FeedDto>> {
        return commands.addFeed(newFeed);
    }

    /** Unsubscribes. Stored articles survive. */
    removeFeed(feedId: number): Promise<CommandResult<null>> {
        return commands.removeFeed(feedId);
    }

    /** Pause / resume / un-dead a feed. */
    setFeedStatus(feedId: number, status: FeedStatusDto): Promise<CommandResult<null>> {
        return commands.setFeedStatus(feedId, status);
    }

    /** Refreshes one feed; fetch/parse failures are outcomes, not errors. */
    refreshFeed(feedId: number): Promise<CommandResult<RefreshOutcomeDto>> {
        return commands.refreshFeed(feedId);
    }

    /** Kicks off the full sweep; progress arrives via events. */
    async refreshAll(): Promise<CommandResult<RefreshOutcomeDto[]>> {
        this.refreshing = true;
        this.refreshOutcomes = [];
        const result = await commands.refreshAll();
        // `RefreshFinished` normally clears the flag; this covers the
        // command erroring before any event fires.
        if (result.status === 'error') {
            this.refreshing = false;
        }
        return result;
    }

    /**
     * Subscribes the progress fields to the Rust-emitted refresh events via
     * the generated bindings. Returns an unsubscribe for teardown.
     */
    async wireRefreshEvents(): Promise<() => void> {
        const unlisteners = await Promise.all([
            events.refreshProgress.listen((event) => {
                this.refreshing = true;
                this.refreshOutcomes = [...this.refreshOutcomes, event.payload.outcome];
            }),
            events.refreshFinished.listen((event) => {
                this.refreshing = false;
                this.refreshOutcomes = event.payload.outcomes;
            }),
        ]);
        return () => {
            for (const unlisten of unlisteners) {
                unlisten();
            }
        };
    }
}

/** The app-wide singleton. Fetches lazily on first property access. */
export const feedsStore = new FeedsStore();
