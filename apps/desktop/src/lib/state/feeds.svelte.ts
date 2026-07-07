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
import { SvelteSet } from 'svelte/reactivity';
import { type FeedFolder, subtreeFeedIds } from './feed-tree';
import { type CommandResult, type Query, ensureQuery, queryKeys } from './query-cache.svelte';

export class FeedsStore {
    /** True while a `refresh_all` sweep is in flight. */
    refreshing: boolean = $state(false);

    /** Per-feed outcomes of the current/last sweep, in refresh order. */
    refreshOutcomes: RefreshOutcomeDto[] = $state([]);

    /**
     * Folder paths the user has collapsed in the sidebar tree. In-memory and
     * expanded-by-default (absence = open); a `SvelteSet` so the tree reacts
     * to a toggle.
     */
    #collapsedFolders = new SvelteSet<string>();

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

    /**
     * The next feed (in sidebar order, wrapping) after `afterFeedId` that has
     * unread articles — the cross-feed hop for next-unread triage. Never
     * returns `afterFeedId` itself; `null` if no other feed has unread.
     */
    nextFeedWithUnread(afterFeedId: number | null): number | null {
        const feeds = this.feeds;
        const start =
            afterFeedId === null ? -1 : feeds.findIndex((feed) => feed.id === afterFeedId);
        for (let step = 1; step <= feeds.length; step += 1) {
            const feed = feeds[(((start + step) % feeds.length) + feeds.length) % feeds.length];
            if (feed.id !== afterFeedId && this.unreadFor(feed.id) > 0) {
                return feed.id;
            }
        }
        return null;
    }

    /** Whether a sidebar folder is collapsed (expanded by default). */
    isFolderCollapsed(path: string): boolean {
        return this.#collapsedFolders.has(path);
    }

    /** Toggles a sidebar folder open/closed. */
    toggleFolder(path: string): void {
        if (this.#collapsedFolders.has(path)) {
            this.#collapsedFolders.delete(path);
        } else {
            this.#collapsedFolders.add(path);
        }
    }

    /**
     * Roll-up unread for a folder: the sum of the backend-owned per-feed
     * counts across its subtree, deduped by feed id (a feed under two
     * subfolders is counted once). Aggregation of authoritative counts, not
     * client badge math.
     */
    folderUnread(folder: FeedFolder): number {
        let total = 0;
        for (const id of subtreeFeedIds(folder)) {
            total += this.unreadFor(id);
        }
        return total;
    }

    /** Test isolation — drops collapse state and any in-flight refresh flags. */
    reset(): void {
        this.#collapsedFolders.clear();
        this.refreshing = false;
        this.refreshOutcomes = [];
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

    /**
     * Replaces a feed's tags (move-to-folder / re-tag). Tags are `/`-path
     * strings the sidebar renders as folders. Wholesale overwrite; the tree
     * refreshes off the Rust-emitted `FeedsChanged`.
     */
    setFeedTags(feedId: number, tags: string[]): Promise<CommandResult<null>> {
        return commands.setFeedTags(feedId, tags);
    }

    /** Renames a feed (an empty title clears it → the URL shows instead). */
    setFeedTitle(feedId: number, title: string | null): Promise<CommandResult<null>> {
        return commands.setFeedTitle(feedId, title);
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
