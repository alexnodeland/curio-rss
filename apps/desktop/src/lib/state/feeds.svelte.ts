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
import { SvelteMap, SvelteSet } from 'svelte/reactivity';
import { type FeedFolder, subtreeFeedIds } from './feed-tree';
import {
    feedInFolder,
    folderPathsForTags,
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

/**
 * How long the refresh flag may stay set with no progress before the watchdog
 * force-clears it. Re-armed on every progress event — a full-text page that
 * finishes reports one too, so a sweep loading pages past its last feed stays
 * alive — so it only trips after a genuine stall: the `RefreshFinished` event
 * failing to arrive must never wedge the refresh button disabled forever.
 */
const REFRESH_WATCHDOG_MS = 120_000;

/** The DOM id of the sidebar's refresh control — where focus goes back to. */
export const REFRESH_CONTROL_ID = 'refresh-all';

/** A refresh sweep's running count — "12 of 300", then its full-text pages. */
export interface RefreshSweepProgress {
    /** The sweep the count belongs to (counts up per app run). */
    sweepId: number;
    /** Feeds finished so far. */
    done: number;
    /** Feeds the sweep set out to refresh. */
    total: number;
    /** Full-text pages finished so far. */
    pagesDone: number;
    /** Full-text pages queued so far — final once `done` reaches `total`. */
    pagesTotal: number;
}

export class FeedsStore {
    /** True while a `refresh_all` sweep is in flight. */
    refreshing: boolean = $state(false);

    /** Per-feed outcomes of the current/last sweep, in refresh order. */
    refreshOutcomes: RefreshOutcomeDto[] = $state([]);

    /**
     * The sweep in flight's running count, from its latest `RefreshProgress`
     * — `null` until its first feed finishes, and once it is over.
     */
    refreshProgress: RefreshSweepProgress | null = $state(null);

    /** Whether the last sweep was stopped early by `cancelRefresh`. */
    refreshCancelled: boolean = $state(false);

    /** True between asking to stop the sweep and its `RefreshFinished`. */
    cancellingRefresh: boolean = $state(false);

    /** Backstop timer that clears `refreshing` if no finish/progress arrives. */
    #refreshWatchdog: ReturnType<typeof setTimeout> | undefined;

    /**
     * The sweep the outcomes belong to. Kept past a settle, so a late event
     * of the same sweep (after the watchdog gave up on it) adds to its
     * outcomes rather than wiping them as a new sweep would.
     */
    #outcomesSweepId: number | null = null;

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

    /**
     * Feeds unsubscribed in the UI whose backend removal is still waiting out
     * its undo window: hidden from every view until the removal lands (or
     * the undo brings them back).
     */
    #hiddenFeeds = new SvelteSet<number>();

    /**
     * Feeds whose removal landed, each with the feed list that was showing
     * then (still listing it): hidden only while that same list is on screen.
     */
    #removedFeeds = new SvelteMap<number, FeedDto[] | undefined>();

    /** The subscriptions as the UI shows them (pending unsubscribes hidden). */
    get feeds(): FeedDto[] {
        const all = this.#feedsQuery.data ?? [];
        if (this.#hiddenFeeds.size === 0 && this.#removedFeeds.size === 0) {
            return all;
        }
        const shown = this.#feedsQuery.data;
        return all.filter(
            (feed) =>
                !this.#hiddenFeeds.has(feed.id) &&
                !(this.#removedFeeds.has(feed.id) && this.#removedFeeds.get(feed.id) === shown),
        );
    }

    /** Forgets removed feeds whose stale list has since been replaced. */
    #pruneRemovedFeeds(): void {
        const shown = this.#feedsQuery.data;
        for (const [feedId, list] of this.#removedFeeds) {
            if (list !== shown) {
                this.#removedFeeds.delete(feedId);
            }
        }
    }

    /** Hides a feed pending its deferred unsubscribe. */
    hideFeed(feedId: number): void {
        this.#hiddenFeeds.add(feedId);
    }

    /** Shows a hidden feed again (undo, or the removal failed). */
    unhideFeed(feedId: number): void {
        this.#hiddenFeeds.delete(feedId);
    }

    /** Re-runs the subscription list's load (the sidebar error's Retry). */
    reload(): Promise<void> {
        return this.#feedsQuery.refetch();
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
        // Relocating to another folder — an emptied source folder is expected
        // to fold away (this isn't the "feed left the folder system" case).
        return this.setFeedTags(feedId, tagsForMove(feed.tags, folderPath));
    }

    /** Removes a feed from its folders (drop on "ungrouped"). */
    async ungroupFeed(feedId: number): Promise<CommandResult<null>> {
        const feed = this.feeds.find((candidate) => candidate.id === feedId);
        if (feed === undefined) return { status: 'ok', data: null };
        const nextTags = tagsForUngroup(feed.tags);
        const result = await this.setFeedTags(feedId, nextTags);
        if (result.status === 'ok') this.#preserveEmptiedFolders(feed.tags, nextTags, feedId);
        return result;
    }

    /**
     * Keeps a user-made folder from silently vanishing the moment its last
     * feed is dragged out: any folder path the feed vacated that no other feed
     * still occupies is re-scaffolded as pending (so it stays visible until the
     * user explicitly deletes it). Reads the pre-refetch `feeds` — the moved
     * feed is excluded by id, every other feed's tags are still authoritative.
     */
    #preserveEmptiedFolders(oldTags: string[], newTags: string[], movedFeedId: number): void {
        const kept = folderPathsForTags(newTags);
        const vacated = folderPathsForTags(oldTags).filter((path) => !kept.includes(path));
        let changed = false;
        for (const path of vacated) {
            const stillUsed = this.feeds.some(
                (candidate) => candidate.id !== movedFeedId && feedInFolder(candidate, path),
            );
            if (!stillUsed && !this.#pendingFolders.has(path)) {
                this.#pendingFolders.add(path);
                changed = true;
            }
        }
        if (changed) this.#persistPending();
    }

    /**
     * Renames folder `oldPath` → `newPath` across every feed in its subtree.
     * Returns the first per-feed failure so the caller surfaces it instead of
     * falsely reporting success; the local path rewrite runs only on full
     * success (a partial retag leaves the backend authoritative, no bad local
     * state). A single atomic backend retag is the follow-up.
     */
    async renameFolder(oldPath: string, newPath: string): Promise<CommandResult<null>> {
        for (const feed of this.feeds.filter((candidate) => feedInFolder(candidate, oldPath))) {
            const result = await this.setFeedTags(
                feed.id,
                tagsForRename(feed.tags, oldPath, newPath),
            );
            if (result.status === 'error') return result;
        }
        this.#rewritePrefixedState(oldPath, newPath);
        return { status: 'ok', data: null };
    }

    /** Deletes folder `path`, moving its feeds to the parent (never unsubscribes). */
    async deleteFolder(path: string): Promise<CommandResult<null>> {
        for (const feed of this.feeds.filter((candidate) => feedInFolder(candidate, path))) {
            const result = await this.setFeedTags(feed.id, tagsForDelete(feed.tags, path));
            if (result.status === 'error') return result;
        }
        this.#dropPrefixedState(path);
        return { status: 'ok', data: null };
    }

    /**
     * Marks every feed in a folder subtree read, all or nothing: returns the
     * ids it flipped (the undo snapshot), or — on the first per-feed failure —
     * puts the feeds already marked back to unread and returns that failure,
     * so a folder is never left half-marked.
     */
    async markFolderRead(folder: FeedFolder): Promise<CommandResult<number[]>> {
        const flipped: number[] = [];
        for (const id of subtreeFeedIds(folder)) {
            const result = await commands.markAllRead(id);
            if (result.status === 'error') {
                if (flipped.length > 0) {
                    // Best effort: if the rollback fails too, the original
                    // error is still what the user needs to see.
                    await commands.markReadMany(flipped, false);
                }
                return result;
            }
            flipped.push(...result.data);
        }
        return { status: 'ok', data: flipped };
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
        this.#hiddenFeeds.clear();
        this.#removedFeeds.clear();
        this.#clearRefreshWatchdog();
        this.#outcomesSweepId = null;
        this.refreshing = false;
        this.refreshOutcomes = [];
        this.refreshProgress = null;
        this.refreshCancelled = false;
        this.cancellingRefresh = false;
    }

    /** Subscribes to a feed. Invalidation rides the Rust-emitted event. */
    addFeed(newFeed: NewFeedDto): Promise<CommandResult<FeedDto>> {
        return commands.addFeed(newFeed);
    }

    /** Unsubscribes. Stored articles survive. Any folder the feed emptied by
     *  leaving stays visible (re-scaffolded) rather than vanishing with it. */
    async removeFeed(feedId: number): Promise<CommandResult<null>> {
        // Read the unfiltered list: a feed in its undo window is hidden from `feeds`.
        const vacatedTags =
            (this.#feedsQuery.data ?? []).find((candidate) => candidate.id === feedId)?.tags ?? [];
        const result = await commands.removeFeed(feedId);
        if (result.status === 'ok') {
            this.#preserveEmptiedFolders(vacatedTags, [], feedId);
            if (this.#hiddenFeeds.has(feedId)) {
                // Keep it out of view while the list on screen is the one
                // fetched before the removal: it ends with the next list that
                // lands, which no longer has the feed. Waiting on a refetch
                // instead isn't enough — one a newer refetch superseded lands
                // nothing, and a failed one leaves this stale list up — and
                // tying the hide to the stale list means a reused id is never
                // hidden.
                this.#pruneRemovedFeeds();
                this.#removedFeeds.set(feedId, this.#feedsQuery.data);
                this.#hiddenFeeds.delete(feedId);
                void this.#feedsQuery.refetch();
            }
        }
        return result;
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
     * Flips a feed's full-text mode: when on, refreshes hydrate new
     * articles from their source pages (for feeds that ship excerpts).
     */
    setFeedFullText(feedId: number, enabled: boolean): Promise<CommandResult<null>> {
        return commands.setFeedFullText(feedId, enabled);
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

    /** Re-arms the stall watchdog: clears `refreshing` after a quiet interval. */
    #armRefreshWatchdog(): void {
        this.#clearRefreshWatchdog();
        this.#refreshWatchdog = setTimeout(() => {
            this.#refreshWatchdog = undefined;
            this.#settleRefresh();
        }, REFRESH_WATCHDOG_MS);
    }

    #clearRefreshWatchdog(): void {
        if (this.#refreshWatchdog !== undefined) {
            clearTimeout(this.#refreshWatchdog);
            this.#refreshWatchdog = undefined;
        }
    }

    /** Kicks off the full sweep; progress arrives via events. */
    async refreshAll(): Promise<CommandResult<RefreshOutcomeDto[]>> {
        // A request while a sweep runs joins it (the core runs one at a
        // time) — keep that sweep's progress rather than wiping it.
        if (!this.refreshing) {
            this.refreshing = true;
            this.refreshOutcomes = [];
            this.refreshProgress = null;
            this.refreshCancelled = false;
            this.cancellingRefresh = false;
        }
        this.#armRefreshWatchdog();
        const result = await commands.refreshAll();
        // `RefreshFinished` normally clears the flag; this covers the
        // command erroring before any event fires.
        if (result.status === 'error') {
            this.#settleRefresh();
        }
        return result;
    }

    /**
     * Asks the core to stop the sweep in flight: no further feed starts and
     * the fetches in flight are dropped; `RefreshFinished` (marked cancelled)
     * then settles the progress fields. `cancellingRefresh` shows the stop is
     * under way until it lands.
     */
    async cancelRefresh(): Promise<CommandResult<boolean>> {
        this.cancellingRefresh = true;
        const result = await commands.cancelRefresh();
        // Nothing was running (the sweep just ended) or the call failed:
        // no finish event is coming for this stop.
        if (result.status === 'error' || !result.data) {
            this.cancellingRefresh = false;
        }
        return result;
    }

    /** Clears the in-flight refresh fields (the outcomes stay for the summary). */
    #settleRefresh(): void {
        this.refreshing = false;
        this.refreshProgress = null;
        this.cancellingRefresh = false;
        this.#clearRefreshWatchdog();
    }

    /**
     * Subscribes the progress fields to the Rust-emitted refresh events via
     * the generated bindings. Returns an unsubscribe for teardown.
     */
    async wireRefreshEvents(): Promise<() => void> {
        const unlisteners = await Promise.all([
            events.refreshProgress.listen((event) => {
                const { sweep_id, done, total, pages_done, pages_total, outcome } = event.payload;
                // A new sweep's progress (the background scheduler starts
                // sweeps too) never adds to the last sweep's outcomes.
                if (this.#outcomesSweepId !== sweep_id) {
                    this.#outcomesSweepId = sweep_id;
                    this.refreshOutcomes = [];
                    this.refreshCancelled = false;
                }
                this.refreshing = true;
                this.refreshProgress = {
                    sweepId: sweep_id,
                    done,
                    total,
                    pagesDone: pages_done,
                    pagesTotal: pages_total,
                };
                // A full-text page's progress carries no feed outcome.
                if (outcome !== null) {
                    this.refreshOutcomes = [...this.refreshOutcomes, outcome];
                }
                this.#armRefreshWatchdog(); // still alive — push the deadline out
            }),
            events.refreshFinished.listen((event) => {
                this.#outcomesSweepId = event.payload.sweep_id;
                this.refreshOutcomes = event.payload.outcomes;
                this.refreshCancelled = event.payload.cancelled;
                this.#settleRefresh();
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
