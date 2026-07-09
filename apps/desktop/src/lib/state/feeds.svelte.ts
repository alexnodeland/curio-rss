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
import {
    feedInFolder,
    tagsForDelete,
    tagsForMove,
    tagsForRename,
    tagsForUngroup,
} from './folder-ops';
import { type CommandResult, type Query, ensureQuery, queryKeys } from './query-cache.svelte';
import { SETTING_KEYS, settingsStore } from './settings.svelte';

/** Replaces a reactive set's contents in place (keeps the reference stable). */
function reloadSet(set: SvelteSet<string>, values: string[]): void {
    set.clear();
    for (const value of values) set.add(value);
}

/** Guarded parse of a persisted JSON string array (WP6 pattern). */
function readStringArray(raw: string | undefined): string[] {
    if (raw === undefined) return [];
    try {
        const parsed: unknown = JSON.parse(raw);
        return Array.isArray(parsed)
            ? parsed.filter((item): item is string => typeof item === 'string')
            : [];
    } catch {
        return [];
    }
}

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

    /**
     * User-created folder paths not yet holding any feed. Folders are a tag
     * projection, so an empty folder can't exist in the feed data — this
     * overlay keeps it visible until a feed is dropped in. Persisted.
     */
    #pendingFolders = new SvelteSet<string>();

    /**
     * Feed ids (as strings) the user has muted for notifications. The Rust
     * scheduler reads the same `ui.notify.muted-feeds` JSON array to skip these
     * feeds when deciding what to notify. Persisted.
     */
    #mutedNotifyFeeds = new SvelteSet<string>();

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
     * Whether the most recent refresh of this feed errored — the signal behind
     * the sidebar's warning dot. Drawn from the last sweep's outcomes (in
     * memory, refreshed each sweep), so it clears as soon as a feed recovers and
     * is empty until the first refresh of the session completes.
     */
    lastErrored(feedId: number): boolean {
        return this.refreshOutcomes.some(
            (outcome) => outcome.feed_id === feedId && outcome.status === 'error',
        );
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

    /** Toggles a sidebar folder open/closed, persisting the collapse set. */
    toggleFolder(path: string): void {
        if (this.#collapsedFolders.has(path)) {
            this.#collapsedFolders.delete(path);
        } else {
            this.#collapsedFolders.add(path);
        }
        this.#persistCollapsed();
    }

    /** The empty user-created folders to overlay onto the derived tree. */
    get pendingPaths(): string[] {
        return [...this.#pendingFolders];
    }

    /** Whether notifications are muted for this feed. */
    isNotifyMuted(feedId: number): boolean {
        return this.#mutedNotifyFeeds.has(String(feedId));
    }

    /** Mutes/unmutes a feed's notifications, persisting the set. */
    setNotifyMuted(feedId: number, muted: boolean): void {
        if (muted) {
            this.#mutedNotifyFeeds.add(String(feedId));
        } else {
            this.#mutedNotifyFeeds.delete(String(feedId));
        }
        void settingsStore.set(
            SETTING_KEYS.notifyMutedFeeds,
            JSON.stringify([...this.#mutedNotifyFeeds]),
        );
    }

    /**
     * Adopts persisted collapse + pending-folder state at startup. Mutates the
     * existing sets in place (never reassigns them) so derivations that already
     * track these `SvelteSet`s keep reacting.
     */
    initSidebarState(): void {
        reloadSet(
            this.#collapsedFolders,
            readStringArray(settingsStore.get(SETTING_KEYS.collapsedFolders)),
        );
        reloadSet(
            this.#pendingFolders,
            readStringArray(settingsStore.get(SETTING_KEYS.pendingFolders)),
        );
        reloadSet(
            this.#mutedNotifyFeeds,
            readStringArray(settingsStore.get(SETTING_KEYS.notifyMutedFeeds)),
        );
    }

    #persistCollapsed(): void {
        void settingsStore.set(
            SETTING_KEYS.collapsedFolders,
            JSON.stringify([...this.#collapsedFolders]),
        );
    }

    #persistPending(): void {
        void settingsStore.set(
            SETTING_KEYS.pendingFolders,
            JSON.stringify([...this.#pendingFolders]),
        );
    }

    /** Creates an empty folder (persisted until a feed is dropped in). */
    createFolder(path: string): void {
        this.#pendingFolders.add(path);
        this.#persistPending();
    }

    /** Moves a feed into `folderPath` (retag), clearing the pending scaffold. */
    async moveFeedToFolder(feedId: number, folderPath: string): Promise<CommandResult<null>> {
        const feed = this.feeds.find((candidate) => candidate.id === feedId);
        if (feed === undefined) return { status: 'ok', data: null };
        if (this.#pendingFolders.delete(folderPath)) this.#persistPending();
        return this.setFeedTags(feedId, tagsForMove(feed.tags, folderPath));
    }

    /** Removes a feed from its folders (drop on "ungrouped"). */
    async ungroupFeed(feedId: number): Promise<CommandResult<null>> {
        const feed = this.feeds.find((candidate) => candidate.id === feedId);
        if (feed === undefined) return { status: 'ok', data: null };
        return this.setFeedTags(feedId, tagsForUngroup(feed.tags));
    }

    /** Renames folder `oldPath` → `newPath` across every feed in its subtree. */
    async renameFolder(oldPath: string, newPath: string): Promise<void> {
        for (const feed of this.feeds.filter((candidate) => feedInFolder(candidate, oldPath))) {
            await this.setFeedTags(feed.id, tagsForRename(feed.tags, oldPath, newPath));
        }
        this.#rewritePrefixedState(oldPath, newPath);
    }

    /** Deletes folder `path`, moving its feeds to the parent (never unsubscribes). */
    async deleteFolder(path: string): Promise<void> {
        for (const feed of this.feeds.filter((candidate) => feedInFolder(candidate, path))) {
            await this.setFeedTags(feed.id, tagsForDelete(feed.tags, path));
        }
        this.#dropPrefixedState(path);
    }

    /** Marks every feed in a folder subtree read. */
    async markFolderRead(folder: FeedFolder): Promise<void> {
        for (const id of subtreeFeedIds(folder)) {
            await commands.markAllRead(id);
        }
    }

    /** Rewrites collapse/pending paths under `oldPath` to `newPath` (folder rename). */
    #rewritePrefixedState(oldPath: string, newPath: string): void {
        const rewrite = (set: SvelteSet<string>): boolean => {
            let changed = false;
            for (const value of [...set]) {
                if (value === oldPath || value.startsWith(`${oldPath}/`)) {
                    set.delete(value);
                    set.add(`${newPath}${value.slice(oldPath.length)}`);
                    changed = true;
                }
            }
            return changed;
        };
        if (rewrite(this.#collapsedFolders)) this.#persistCollapsed();
        if (rewrite(this.#pendingFolders)) this.#persistPending();
    }

    /** Drops collapse/pending paths at or under `path` (folder delete). */
    #dropPrefixedState(path: string): void {
        const drop = (set: SvelteSet<string>): boolean => {
            let changed = false;
            for (const value of [...set]) {
                if (value === path || value.startsWith(`${path}/`)) {
                    set.delete(value);
                    changed = true;
                }
            }
            return changed;
        };
        if (drop(this.#collapsedFolders)) this.#persistCollapsed();
        if (drop(this.#pendingFolders)) this.#persistPending();
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

    /** Test isolation — drops sidebar state and any in-flight refresh flags. */
    reset(): void {
        this.#collapsedFolders.clear();
        this.#pendingFolders.clear();
        this.#mutedNotifyFeeds.clear();
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

    /**
     * Overwrites a feed's site URL and description (edit-feed modal). Unlike
     * the fetch-fill path these are unconditional edits; an empty value clears
     * the field. The tree refreshes off the command's `FeedsChanged`.
     */
    setFeedMetadata(
        feedId: number,
        siteUrl: string | null,
        description: string | null,
    ): Promise<CommandResult<null>> {
        return commands.setFeedMetadata(feedId, siteUrl, description);
    }

    /**
     * Persists a new global feed order (drag-to-reorder); `orderedIds` is the
     * complete sequence. The tree refreshes off the emitted `FeedsChanged`.
     */
    reorderFeeds(orderedIds: number[]): Promise<CommandResult<null>> {
        return commands.reorderFeeds(orderedIds);
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
