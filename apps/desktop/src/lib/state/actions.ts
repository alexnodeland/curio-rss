/**
 * The action layer between input surfaces (keyboard shortcuts, toolbar
 * buttons, sidebar clicks) and the backend: every mutation goes through the
 * generated wrappers, failures become toasts, and list/count updates arrive
 * via the Rust-emitted invalidation events — never client-side bookkeeping.
 * Toggles read the authoritative flag first (`get_article_state`), then
 * write the flip; core's idempotency `bool` makes double-fires harmless.
 */
import {
    events,
    type ArticleSummaryDto,
    type CommandError,
    type ImportSourceDto,
    commands,
} from '$lib/bindings';
import { t } from '$lib/i18n';
import { SHORTCUTS, type ShortcutId } from '$lib/keyboard/registry';
import { commandErrorMessage } from '$lib/utils/errors';
import { isOpenableUrl, openExternal } from '$lib/utils/external';
import {
    ALL_ARTICLES,
    type ArticleFilters,
    articlesStore,
    filterKey,
    unreadScopeDto,
} from './articles.svelte';
import { destinationsStore } from './destinations.svelte';
import { rebuildGlobalOrder } from './feed-dnd.svelte';
import { type FeedFolder, subtreeFeedIds } from './feed-tree';
import { feedsStore } from './feeds.svelte';
import { feedInFolder } from './folder-ops';
import { layoutStore } from './layout.svelte';
import type { CommandResult } from './query-cache.svelte';
import { searchStore } from './search.svelte';
import { selectionStore } from './selection.svelte';
import { type ToastOptions, toastStore } from './toasts.svelte';

/** Re-runs the action that failed (the error toast's Retry button). */
export type Retry = () => void;

/**
 * How a failed action is retried. `run` re-runs the whole user action — the
 * caller's full flow, with the absolute target it asked for (never a
 * relative flip, which would mean the opposite once anything changed).
 * `key` names what the action writes (`refresh:12`, `starred:7`): a repeat
 * failure under it replaces the showing toast instead of stacking, a
 * different action's failure never merges into it, and a later success under
 * it withdraws the toast, whose Retry would replay a stale request over the
 * newer outcome.
 */
export interface RetryPlan {
    readonly run: Retry;
    readonly key?: string;
}

/** An error toast's options: keyed, with a Retry button when `retry` is set. */
function errorOptions(plan: RetryPlan | undefined, retry: boolean): ToastOptions {
    return {
        key: plan?.key,
        action:
            plan !== undefined && retry ? { label: t('toast.retry'), run: plan.run } : undefined,
    };
}

/**
 * Whether retrying the same action can help. The backend's `recoverable`
 * also covers input the user can fix (an invalid folder name), but a Retry
 * replays the same input, which fails the same validation every time.
 */
function retryable(error: CommandError): boolean {
    return error.recoverable && error.code !== 'INVALID_INPUT';
}

/**
 * Surfaces a command failure as an error toast (sticky until dismissed).
 * With `retry`, a failure retrying can fix (see {@link retryable}) offers a
 * Retry button that runs the plan; any other (bad input, a row that's gone)
 * doesn't.
 */
export function toastCommandError(error: CommandError, retry?: RetryPlan): void {
    toastStore.showToast(
        commandErrorMessage(error),
        'error',
        errorOptions(retry, retryable(error)),
    );
}

/**
 * Unwraps a command result; failures (including thrown) become error toasts.
 * `retry`, when given, is what the toast's Retry button runs (see
 * {@link RetryPlan}); a success under its key withdraws a showing failure.
 */
async function run<T>(
    invoke: () => Promise<CommandResult<T>>,
    retry?: RetryPlan,
): Promise<T | undefined> {
    let result: CommandResult<T>;
    try {
        result = await invoke();
    } catch {
        // A thrown (non-contract) failure is internal; retrying may help.
        toastStore.showToast(t('app.error.internal'), 'error', errorOptions(retry, true));
        return undefined;
    }
    if (result.status === 'error') {
        toastCommandError(result.error, retry);
        return undefined;
    }
    if (retry?.key !== undefined) {
        toastStore.withdraw(retry.key);
    }
    return result.data;
}

/** {@link runCommand}'s options. */
export interface RunCommandOptions {
    /** What the command writes (see {@link RetryPlan.key}). */
    readonly key?: string;
    /**
     * The caller's whole flow, for Retry — when it does more than the
     * command (a follow-up toast). Default: invoke the command again.
     */
    readonly retry?: Retry;
}

/**
 * Public unwrap for fire-and-forget mutation call sites that would otherwise
 * `void` the `CommandResult` and swallow failures (sidebar/folder ops). A
 * failure toasts with a Retry (the caller's `retry`, or the same `invoke`
 * again — so an invoke should compute its arguments from current state, or
 * carry the absolute target the user asked for); success is silent. Route
 * every such mutation through this so no failure is lost and none falsely
 * reports success.
 */
export function runCommand<T>(
    invoke: () => Promise<CommandResult<T>>,
    options: RunCommandOptions = {},
): Promise<T | undefined> {
    return run(invoke, {
        key: options.key,
        run: options.retry ?? (() => void runCommand(invoke, options)),
    });
}

type ToggleField = 'starred' | 'read' | 'read_later' | 'archived';

/**
 * Per-article-field toggle serialization. A naive toggle reads the backend flag
 * then writes its negation; two fast presses both read the *pre-write* value
 * concurrently and both write the same negation, so the second press is
 * silently dropped. Serializing per key (chained promise) makes the second
 * press run only after the first settles, and `#toggleDesired` carries the
 * last-written value so the follow-up alternates without a redundant read.
 */
const toggleChain = new Map<string, Promise<unknown>>();
const toggleDesired = new Map<string, boolean>();

type FlagWrite = (id: number, value: boolean) => Promise<CommandResult<boolean>>;

/** Runs `step` after every earlier step on the same flag has settled. */
async function onFlagChain<T>(key: string, step: () => Promise<T>): Promise<T> {
    const previous = toggleChain.get(key) ?? Promise.resolve();
    const task = previous.then(step);
    toggleChain.set(key, task);
    const outcome = await task;
    // Drain once this press is the tail of the chain, so the maps don't grow.
    if (toggleChain.get(key) === task) {
        toggleChain.delete(key);
        toggleDesired.delete(key);
    }
    return outcome;
}

/**
 * Writes an absolute flag value; the value written, or `undefined` when the
 * write failed (toasted). A failed write's Retry writes that same value —
 * never a fresh flip, which would mean the opposite once the flag changed in
 * between (the toast stays until dismissed) — and a later successful write
 * of the flag withdraws the failure (same key).
 */
async function writeFlag(
    articleId: number,
    field: ToggleField,
    write: FlagWrite,
    value: boolean,
): Promise<boolean | undefined> {
    const key = `${field}:${articleId}`;
    toggleDesired.set(key, value);
    const ok = await run(() => write(articleId, value), {
        key,
        run: () => void onFlagChain(key, () => writeFlag(articleId, field, write, value)),
    });
    if (ok === undefined) {
        // The write failed (toast already fired) — forget the optimistic
        // value so the next press reads the real backend flag again.
        toggleDesired.delete(key);
        return undefined;
    }
    return value;
}

/** Flips a flag; the value written, or `undefined` when nothing was (toasted). */
async function toggleFlag(
    articleId: number,
    field: ToggleField,
    write: FlagWrite,
): Promise<boolean | undefined> {
    const key = `${field}:${articleId}`;
    return onFlagChain(key, async (): Promise<boolean | undefined> => {
        let current: boolean;
        if (toggleDesired.has(key)) {
            current = toggleDesired.get(key) as boolean;
        } else {
            // A failed read decided nothing, so its Retry is the whole toggle.
            const state = await run(() => commands.getArticleState(articleId), {
                key,
                run: () => void toggleFlag(articleId, field, write),
            });
            if (state === undefined) {
                return undefined; // read failed (a toast fired); nothing to flip
            }
            current = state[field];
        }
        return writeFlag(articleId, field, write, !current);
    });
}

/** Stars / unstars against the current backend flag. */
export async function toggleStar(articleId: number): Promise<void> {
    await toggleFlag(articleId, 'starred', (id, value) => commands.setStarred(id, value));
}

/** Adds to / removes from the read-later queue. */
export async function toggleReadLater(articleId: number): Promise<void> {
    await toggleFlag(articleId, 'read_later', (id, value) => commands.setReadLater(id, value));
}

/**
 * Marks read / unread against the current backend flag. The row is held for
 * the view session first, so marking it read in an unread-only view keeps it
 * in place (dimmed) instead of yanking it out from under the cursor.
 */
export async function toggleRead(articleId: number): Promise<void> {
    articlesStore.holdRead(articleId);
    await toggleFlag(articleId, 'read', (id, value) => commands.markRead(id, value));
}

/**
 * Archives / unarchives against the current backend flag. Archiving takes the
 * row out of the reading flow, so once the archive is written any session
 * read-hold on it is let go; a failed write (toasted) leaves the row held.
 */
export async function toggleArchived(articleId: number): Promise<void> {
    const archived = await toggleFlag(articleId, 'archived', (id, value) =>
        commands.setArchived(id, value),
    );
    if (archived === true) {
        articlesStore.releaseHeld(articleId);
    }
}

/**
 * Displaying an article in the reader marks it read (idempotent — core
 * emits nothing when it already was). The `article.opened` contract event
 * is reserved for a real open-in-browser, so the log stays truthful under
 * j/k scanning.
 */
export async function markReadOnOpen(articleId: number): Promise<void> {
    // Held first: in an unread-only view the open must not drop the row (and
    // the selection with it) — it stays until the view changes.
    articlesStore.holdRead(articleId);
    await run(() => commands.markRead(articleId, true), {
        key: `read:${articleId}`,
        run: () => void markReadOnOpen(articleId),
    });
}

/**
 * Mark-on-scroll: a row that scrolled up out of the list marks itself read.
 * Silent by design — this is a background sweep that can fire once per row,
 * so a transient failure must never raise a toast. Core no-ops when the row
 * was already read, so re-scrolling the same rows costs nothing.
 */
export async function markReadOnScroll(articleId: number): Promise<void> {
    // Held so the scrolled-past rows don't vanish and shift the list up.
    articlesStore.holdRead(articleId);
    try {
        await commands.markRead(articleId, true);
    } catch {
        // A background read-mark that fails is not worth interrupting for.
    }
}

/**
 * Fetches the article's source page and replaces its stored content with the
 * readability-extracted full article; the reader + list refetch off the
 * emitted `ArticlesChanged`. Returns whether it succeeded (for a loading UI).
 */
export async function loadFullArticle(articleId: number): Promise<boolean> {
    return (
        (await run(() => commands.loadFullArticle(articleId), {
            key: `fullArticle:${articleId}`,
            run: () => void loadFullArticle(articleId),
        })) !== undefined
    );
}

/**
 * Puts a bulk mark-read back: exactly the snapshot of ids it flipped return
 * to unread (one `mark_read_many` transaction — ids purged since are
 * skipped), and the toast says how many came back.
 */
export async function undoMarkRead(articleIds: readonly number[]): Promise<void> {
    const restored = await run(() => commands.markReadMany([...articleIds], false), {
        run: () => void undoMarkRead(articleIds),
    });
    if (restored !== undefined) {
        toastStore.showToast(t('toast.markReadUndone', { count: restored }), 'info');
    }
}

/** The "Marked N read" toast, with Undo when anything changed. */
function toastMarkedRead(articleIds: readonly number[]): void {
    const message = t('toast.markAllRead', { count: articleIds.length });
    if (articleIds.length === 0) {
        toastStore.showToast(message, 'success');
        return;
    }
    toastStore.showToast(message, 'success', {
        action: { label: t('toast.undo'), run: () => undoMarkRead(articleIds) },
    });
}

/**
 * Marks every unread article in a feed (`feedId`) — or the whole library
 * (`null`) — read, and toasts how many changed, with an Undo that restores
 * exactly the articles this sweep flipped (the backend returns their ids).
 * The sidebar badges and the list refresh off the Rust-emitted
 * `ArticlesChanged`.
 */
export async function markAllRead(feedId: number | null): Promise<void> {
    const changed = await run(() => commands.markAllRead(feedId), {
        key: `markAllRead:${feedId ?? 'all'}`,
        run: () => void markAllRead(feedId),
    });
    if (changed !== undefined) {
        // An explicit sweep: the swept feed's held stragglers go with the rest
        // (the whole hold for the library-wide sweep); other feeds' stay put.
        articlesStore.releaseHolds(feedId);
        toastMarkedRead(changed);
    }
}

/**
 * Marks a whole folder subtree read, all or nothing: a failure part-way rolls
 * back the feeds already marked (the store does it) and toasts with Retry;
 * success toasts the count with Undo.
 */
export async function markFolderRead(folder: FeedFolder): Promise<void> {
    const changed = await run(() => feedsStore.markFolderRead(folder), {
        key: `markFolderRead:${folder.path}`,
        run: () => void markFolderRead(folder),
    });
    if (changed !== undefined) {
        // Like a feed sweep: the swept feeds' held rows go, others' stay put.
        for (const feedId of subtreeFeedIds(folder)) {
            articlesStore.releaseHolds(feedId);
        }
        toastMarkedRead(changed);
    }
}

/**
 * Unsubscribes with an undo window instead of a confirm: the feed leaves the
 * sidebar at once, and the toast's Undo brings it back untouched. The backend
 * removal (which deletes the subscription row — its id, health and history
 * can't be recreated) runs only once the toast closes any other way — its
 * timer, the close button, eviction, or the window unloading — so an undo
 * never has anything to restore. The removal is also staged backend-side,
 * which commits it if the app quits inside the window (the webview need not
 * deliver `pagehide` on a quit).
 */
export function unsubscribeWithUndo(feedId: number, name: string): void {
    const wasSelected = selectionStore.selectedFeedId === feedId;
    const previous = {
        filters: articlesStore.filters,
        articleId: selectionStore.selectedArticleId,
    };
    if (wasSelected) {
        selectView('all'); // don't leave the list showing a feed that's going away
    }
    feedsStore.hideFeed(feedId);
    void stageRemoval(feedId, true);
    const undo = (): void => {
        void stageRemoval(feedId, false);
        feedsStore.unhideFeed(feedId);
        // Back to the feed, unless the user has moved on from the All view
        // the unsubscribe switched to.
        const untouched =
            activeView(articlesStore.filters) === 'all' &&
            selectionStore.selectedFeedId === null &&
            !searchStore.active;
        if (wasSelected && untouched) {
            selectionStore.selectedFeedId = feedId;
            selectionStore.selectedArticleId = previous.articleId;
            articlesStore.filters = previous.filters;
        }
    };
    toastStore.showToast(t('feed.unsubscribed', { name }), 'info', {
        action: { label: t('toast.undo'), run: undo },
        onClose: (reason) => {
            if (reason !== 'action') {
                void finishUnsubscribe(feedId);
            }
        },
    });
}

/**
 * Stages (or withdraws) an unsubscribe's exit-time commit. Best effort: if
 * it fails, only that safety net is lost — the toast still commits on close.
 */
async function stageRemoval(feedId: number, staged: boolean): Promise<void> {
    try {
        await (staged ? commands.stageFeedRemoval(feedId) : commands.unstageFeedRemoval(feedId));
    } catch {
        // No IPC (the window is going away): nothing else to do.
    }
}

/** The deferred half of {@link unsubscribeWithUndo}: the backend removal. */
async function finishUnsubscribe(feedId: number): Promise<void> {
    const removed = await run(() => feedsStore.removeFeed(feedId), {
        key: `unsubscribe:${feedId}`,
        run: () => void finishUnsubscribe(feedId),
    });
    if (removed === undefined) {
        // Still subscribed after all: show it again (the error toast has Retry).
        feedsStore.unhideFeed(feedId);
    }
}

// ─── sidebar mutations ──────────────────────────────────────────────────────
// Each is keyed by what it writes, so a later success withdraws a stale
// failure's Retry, and each Retry carries the absolute target (or recomputes
// from current state) rather than replaying a snapshot over newer changes.

/** Retitles a feed (`null` = back to the feed's own title). */
export function renameFeed(feedId: number, title: string | null): Promise<null | undefined> {
    return runCommand(() => feedsStore.setFeedTitle(feedId, title), {
        key: `feed:${feedId}:title`,
    });
}

/** Moves a feed into folder `path` (the move reads the feed's current tags). */
export function moveFeed(feedId: number, path: string): Promise<null | undefined> {
    return runCommand(() => feedsStore.moveFeedToFolder(feedId, path), {
        key: `feed:${feedId}:folder`,
    });
}

/** Takes a feed out of its folders. */
export function ungroupFeed(feedId: number): Promise<null | undefined> {
    return runCommand(() => feedsStore.ungroupFeed(feedId), { key: `feed:${feedId}:folder` });
}

/** Pauses or resumes a feed's refreshes. */
export function setFeedPaused(feedId: number, paused: boolean): Promise<null | undefined> {
    return runCommand(() => feedsStore.setFeedStatus(feedId, paused ? 'paused' : 'active'), {
        key: `feed:${feedId}:status`,
    });
}

/**
 * Applies a within-group reorder (`group`: the group's ids in their new
 * order). The global order is rebuilt from the current feed list at invoke
 * time, so a Retry applies the same group order without dropping feeds added
 * (or resurrecting ones removed) since the failure.
 */
export function reorderFeedGroup(group: readonly number[]): Promise<null | undefined> {
    return runCommand(
        () => {
            const all = feedsStore.feeds.map((candidate) => candidate.id);
            const present = group.filter((id) => all.includes(id));
            return feedsStore.reorderFeeds(rebuildGlobalOrder(all, present));
        },
        { key: 'feeds:order' },
    );
}

/** Renames folder `oldPath` to `newPath` (retagging the feeds in it as they are now). */
export function renameFolder(oldPath: string, newPath: string): Promise<null | undefined> {
    return runCommand(() => feedsStore.renameFolder(oldPath, newPath), {
        key: `folder:${oldPath}`,
    });
}

/**
 * Deletes folder `path` (its feeds move to the parent) and says so — only
 * once it actually succeeded. A failure's Retry runs this whole flow, so a
 * retried delete that succeeds announces itself too.
 */
export async function deleteFolder(path: string, name: string): Promise<void> {
    const result = await runCommand(() => feedsStore.deleteFolder(path), {
        key: `folder:${path}`,
        retry: () => void deleteFolder(path, name),
    });
    if (result !== undefined) {
        toastStore.showToast(t('folder.deleted', { name }), 'info');
    }
}

/** How many unread rows one mark-view-read listing page asks for. */
const MARK_VIEW_PAGE = 200;

/** Marks each id read, one command apiece; stops at (and toasts) the first failure. */
async function markEachRead(articleIds: readonly number[]): Promise<number | undefined> {
    let changed = 0;
    for (const articleId of articleIds) {
        const flipped = await run(() => commands.markRead(articleId, true));
        if (flipped === undefined) {
            return undefined;
        }
        if (flipped) {
            changed += 1;
        }
    }
    return changed;
}

/** One page of a view's unread rows strictly below `before`. */
function unreadPage(
    filters: ArticleFilters,
    before: number | null,
): Promise<ArticleSummaryDto[] | undefined> {
    return run(() => commands.listArticles(unreadScopeDto(filters, before, MARK_VIEW_PAGE)));
}

/** Every unread article id in a filtered view, paged backend-side. */
async function unreadIdsInView(filters: ArticleFilters): Promise<number[] | undefined> {
    const ids: number[] = [];
    let before: number | null = null;
    for (;;) {
        const page = await unreadPage(filters, before);
        if (page === undefined) {
            return undefined;
        }
        ids.push(...page.map((article) => article.id));
        const last = page.at(-1);
        if (page.length < MARK_VIEW_PAGE || last === undefined) {
            return ids;
        }
        before = last.id;
    }
}

/**
 * Whether the backend's bulk command covers a view: it scopes by feed or the
 * whole library, so any narrowing flag (starred, read later, a tag, the
 * Archived view) has no bulk form.
 */
function hasBulkScope(filters: ArticleFilters): boolean {
    return (
        filters.starred === null &&
        filters.readLater === null &&
        filters.tag === null &&
        filters.archived !== true
    );
}

/** Bulk-marks every feed in folder `path` read; the summed count. */
async function markFolderFeedsRead(path: string): Promise<number | undefined> {
    let changed = 0;
    for (const feed of feedsStore.feeds.filter((candidate) => feedInFolder(candidate, path))) {
        const flipped = await run(() => commands.markAllRead(feed.id));
        if (flipped === undefined) {
            return undefined;
        }
        changed += flipped.length;
    }
    return changed;
}

/** The count of articles a view-wide mark flipped, or `undefined` on failure. */
async function markViewReadCount(): Promise<number | undefined> {
    if (searchStore.active) {
        // Search results are a fixed FTS window: mark exactly what is listed.
        return markEachRead(articlesStore.activeItems.map((article) => article.id));
    }
    const filters = articlesStore.filters;
    if (filters.read === true) {
        return 0; // a read-only view holds nothing unread
    }
    if (hasBulkScope(filters) && filters.feedTag === null) {
        const flipped = await run(() => commands.markAllRead(filters.feedId));
        return flipped?.length;
    }
    if (hasBulkScope(filters) && filters.feedTag !== null && filters.feedId === null) {
        return markFolderFeedsRead(filters.feedTag);
    }
    const ids = await unreadIdsInView(filters);
    return ids === undefined ? undefined : markEachRead(ids);
}

/**
 * Marks everything unread in the current view read (`Shift+A`) and toasts how
 * many changed. A feed or the whole library rides the backend's bulk
 * command, a folder its feeds' bulk marks; every narrower view (Starred,
 * Read later, Archived, a tag, search results) lists its own unread and
 * marks exactly those. Known gap: the bulk command ignores `archived`, so `A`
 * from All, a feed or a folder also marks that scope's hidden archived rows
 * read (#278). Like mark-all-read, the sweep ends the view's read-hold: the
 * rows read earlier this session go with the rest.
 */
export async function markViewRead(): Promise<void> {
    const changed = await markViewReadCount();
    if (changed !== undefined) {
        articlesStore.releaseHolds();
        toastStore.showToast(t('toast.markAllRead', { count: changed }), 'success');
    }
}

/** The reader's scroll body while one is mounted (registered by ReaderPane). */
let readerBody: HTMLElement | null = null;

/**
 * Registers the mounted reader scroll body as the Space-paging target.
 * Returns the unregister (a no-op once a newer body took over).
 */
export function registerReaderBody(element: HTMLElement): () => void {
    readerBody = element;
    return () => {
        if (readerBody === element) {
            readerBody = null;
        }
    };
}

/** A Space page is 7/8 of the viewport — the browser's own step, keeping a line of context. */
const PAGE_FRACTION = 0.875;

/** Sub-pixel slack when deciding the reader sits at the article's end. */
const END_SLACK_PX = 2;

/**
 * Space / Shift+Space: pages the reader body down / up. Space at the end of
 * the article — or with nothing selected — advances to the next unread
 * (Reeder / NetNewsWire). While the selected article is still loading there
 * is no body to page, and the press is dropped rather than skipping an
 * article the reader never saw.
 */
export function pageReader(direction: 'down' | 'up'): void {
    if (selectionStore.selectedArticleId === null) {
        if (direction === 'down') {
            void goToNextUnread();
        }
        return;
    }
    const body = readerBody;
    if (body === null) {
        return;
    }
    const atEnd = body.scrollTop + body.clientHeight >= body.scrollHeight - END_SLACK_PX;
    if (direction === 'down' && atEnd) {
        void goToNextUnread();
        return;
    }
    const step = Math.round(body.clientHeight * PAGE_FRACTION);
    // A plain scrollTop write honours the body's CSS `scroll-behavior`
    // (smooth unless the user asked for reduced motion).
    body.scrollTop = Math.max(0, body.scrollTop + (direction === 'down' ? step : -step));
}

/**
 * Opens the article's source in the OS browser and records the open.
 * Non-http(s) URLs are refused outright — no open, no `article.opened`.
 */
export async function openInBrowser(articleId: number, url: string): Promise<void> {
    if (!isOpenableUrl(url)) {
        return;
    }
    try {
        await openExternal(url);
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
        return;
    }
    await run(() => commands.recordOpened(articleId, null));
}

/**
 * Runs the native import flow (OPML by default): a Rust-side file picker
 * returns an opaque PathToken, the chosen source's parser ingests it, and
 * the outcome is toasted. Invalidation rides the Rust-emitted events, so the
 * list and sidebar refresh on their own. Shared by the Settings import panel
 * and the new-user empty state.
 */
export async function importFromFile(source: ImportSourceDto = 'opml'): Promise<void> {
    try {
        const picked = await commands.pickImportFile();
        if (picked.status === 'error') {
            toastCommandError(picked.error);
            return;
        }
        if (picked.data === null) {
            toastStore.showToast(t('opml.cancelled'), 'info');
            return;
        }
        const result = await commands.importFile(picked.data.token, source);
        if (result.status === 'error') {
            toastCommandError(result.error);
            return;
        }
        const {
            feeds_added,
            articles_added,
            feeds_skipped,
            articles_skipped,
            feeds_invalid,
            articles_invalid,
        } = result.data;
        const counts = {
            feeds: feeds_added,
            articles: articles_added,
            skipped: feeds_skipped + articles_skipped,
        };
        // Unusable rows (a broken outline, a webcal: or junk URL) never fail
        // the import, but they are not silent either: the toast says how many
        // were left out, as a warning rather than a success.
        const invalid = feeds_invalid + articles_invalid;
        if (invalid > 0) {
            toastStore.showToast(t('import.doneWithInvalid', { ...counts, invalid }), 'warning');
        } else {
            toastStore.showToast(t('import.done', counts), 'success');
        }
    } catch {
        toastStore.showToast(t('app.error.internal'), 'error');
    }
}

/** The sidebar's built-in views over the backend-owned filters. */
export type ViewId = 'all' | 'starred' | 'readLater' | 'archived';

const VIEW_FILTERS: Record<ViewId, ArticleFilters> = {
    all: ALL_ARTICLES,
    starred: { ...ALL_ARTICLES, starred: true },
    readLater: { ...ALL_ARTICLES, readLater: true },
    // The one view that shows archived items (everything else hides them).
    archived: { ...ALL_ARTICLES, archived: true },
};

/** Switches the article list to a built-in view (clears feed + search). */
export function selectView(view: ViewId): void {
    searchStore.clear(); // a view switch leaves search mode
    selectionStore.selectedFeedId = null;
    selectionStore.selectedArticleId = null;
    articlesStore.filters = VIEW_FILTERS[view];
}

/**
 * Scopes the article list to a folder — every article whose feed carries this
 * `/`-path tag or a tag nested beneath it (backend-owned via `feed_tag`).
 * Clears the feed scope + search so the folder is the active selection.
 */
export function selectFolder(path: string): void {
    searchStore.clear();
    selectionStore.selectedFeedId = null;
    selectionStore.selectedArticleId = null;
    articlesStore.filters = { ...ALL_ARTICLES, feedTag: path };
}

/**
 * Jumps to the next unread article, crossing feed boundaries. The next unread
 * strictly below the selection is chosen *within the current view* — the
 * displayed filter is left untouched (an unread row is present in an
 * all/unread view, so no silent switch to unread-only is needed). When the
 * current scope is out of unread, it hops to the next feed with unread and
 * takes its first; when nothing unread remains anywhere, it says so.
 */
export async function goToNextUnread(): Promise<void> {
    const before = selectionStore.selectedArticleId;
    const next = await run(() =>
        commands.listArticles(unreadScopeDto(articlesStore.filters, before)),
    );
    if (next !== undefined && next.length > 0) {
        selectionStore.selectedArticleId = next[0].id;
        return;
    }
    const feed = feedsStore.nextFeedWithUnread(selectionStore.selectedFeedId);
    if (feed === null) {
        toastStore.showToast(t('nav.noMoreUnread'), 'info');
        return;
    }
    searchStore.clear();
    selectionStore.selectedFeedId = feed;
    articlesStore.filters = { ...ALL_ARTICLES, feedId: feed };
    const first = await run(() =>
        commands.listArticles(unreadScopeDto({ ...ALL_ARTICLES, feedId: feed }, null)),
    );
    if (first !== undefined && first.length > 0) {
        selectionStore.selectedArticleId = first[0].id;
    }
}

/** The built-in view a filter set corresponds to, if any (highlighting). */
export function activeView(filters: ArticleFilters): ViewId | null {
    const key = filterKey(filters);
    for (const view of Object.keys(VIEW_FILTERS) as ViewId[]) {
        if (filterKey(VIEW_FILTERS[view]) === key) {
            return view;
        }
    }
    return null;
}

function selectNext(): void {
    const nearEnd = selectionStore.selectNextArticle();
    // Search results are a fixed FTS window — only the filter list pages.
    if (nearEnd && !searchStore.active) {
        void articlesStore.current.loadMore();
    }
}

/**
 * Promotes the selected article to a destination by NAME. Uses the chosen
 * default (or the sole destination); with no clear target it opens the
 * destinations panel to pick one. A raw path never crosses IPC — only the
 * registry name. The outcome disposition drives the toast copy.
 */
export async function promoteSelected(): Promise<void> {
    const articleId = selectionStore.selectedArticleId;
    if (articleId === null) {
        return;
    }
    await destinationsStore.ensureLoaded();
    const target = destinationsStore.promoteTarget;
    if (target === null) {
        layoutStore.openModal('destinations');
        return;
    }
    await promoteArticle(articleId, target);
}

/**
 * Promotes one article to the destination named `target` and toasts the
 * outcome. A failure's Retry promotes that same article to that same
 * destination — never whatever is selected (or the default) by the time the
 * sticky toast's Retry is clicked.
 */
export async function promoteArticle(articleId: number, target: string): Promise<void> {
    const outcome = await run(() => destinationsStore.promote(articleId, target), {
        key: `promote:${articleId}:${target}`,
        run: () => void promoteArticle(articleId, target),
    });
    if (outcome === undefined) {
        return;
    }
    const key =
        outcome.disposition === 'unchanged' ? 'toast.promote.unchanged' : 'toast.promote.saved';
    toastStore.showToast(t(key, { name: target }), 'success');
}

/** Focuses the search input (the `/` shortcut). */
function focusSearch(): void {
    searchStore.requestFocus();
}

/**
 * Refreshes one feed (the `r` shortcut, the feed's context menu). An explicit
 * refresh ends that feed's share of the current view's read-hold: its rows
 * show the backend window again once it reloads. Held rows of other feeds
 * stay put — refreshing feed B never reshuffles a view of feed A.
 */
export function refreshFeed(feedId: number): void {
    articlesStore.releaseHolds(feedId);
    void run(() => feedsStore.refreshFeed(feedId), {
        key: `refresh:${feedId}`,
        run: () => refreshFeed(feedId),
    });
}

/** Refreshes the selected feed, if any (the `r` shortcut). */
function refreshSelectedFeed(): void {
    const feedId = selectionStore.selectedFeedId;
    if (feedId === null) {
        return;
    }
    refreshFeed(feedId);
}

/**
 * Kicks off a full refresh sweep (the `Shift+R` shortcut / toolbar); like a
 * feed refresh, it ends the current view's read-hold.
 */
export function refreshAll(): void {
    articlesStore.releaseHolds();
    void run(() => feedsStore.refreshAll(), { key: 'refresh:all', run: () => refreshAll() });
}

function openSelected(): void {
    const selected = selectionStore.selectedArticle;
    if (selected !== null) {
        void openInBrowser(selected.id, selected.source_url);
    }
}

function withSelected(action: (articleId: number) => Promise<void>): void {
    const articleId = selectionStore.selectedArticleId;
    if (articleId !== null) {
        void action(articleId);
    }
}

/**
 * Enter / → from the list: hands the keyboard to the reader body. With
 * nothing selected it selects first (the row j would pick), so the key is
 * never a dead end.
 */
function focusReader(): void {
    if (selectionStore.selectedArticleId === null) {
        selectNext();
    }
    if (selectionStore.selectedArticleId !== null) {
        selectionStore.focusReader();
    }
}

function toggleHelp(): void {
    if (layoutStore.activeModal === 'help') {
        layoutStore.closeModal();
    } else {
        layoutStore.openModal('help');
    }
}

/**
 * Executes one shortcut id. The core set (j/k, o, s, l, m, ?) landed in
 * WP3; WP4 wires the reader-breadth surfaces: `p` promote, `/` search
 * focus, `r`/`Shift+R` refresh, and the `g`-chord view switches. The pane
 * keys (listbox ↑/↓/Enter/→, reader ←/Esc) land here too, through the
 * panes' own handlers.
 */
export function handleShortcut(id: ShortcutId): void {
    switch (id) {
        case 'nav.nextArticle':
            selectNext();
            break;
        case 'nav.previousArticle':
            selectionStore.selectPreviousArticle();
            break;
        case 'nav.nextUnread':
            void goToNextUnread();
            break;
        case 'nav.focusReader':
            focusReader();
            break;
        case 'nav.focusList':
            selectionStore.focusList();
            break;
        case 'reader.pageDown':
            pageReader('down');
            break;
        case 'reader.pageUp':
            pageReader('up');
            break;
        case 'article.open':
            openSelected();
            break;
        case 'article.toggleStar':
            withSelected(toggleStar);
            break;
        case 'article.toggleReadLater':
            withSelected(toggleReadLater);
            break;
        case 'article.toggleRead':
            withSelected(toggleRead);
            break;
        case 'article.toggleArchived':
            withSelected(toggleArchived);
            break;
        case 'view.markAllRead':
            void markViewRead();
            break;
        case 'article.promote':
            void promoteSelected();
            break;
        case 'feed.refresh':
            refreshSelectedFeed();
            break;
        case 'app.refreshAll':
            refreshAll();
            break;
        case 'app.addFeed':
            layoutStore.openModal('add-feed');
            break;
        case 'app.settings':
            layoutStore.openModal('settings');
            break;
        case 'search.focus':
            focusSearch();
            break;
        case 'view.all':
            selectView('all');
            break;
        case 'view.starred':
            selectView('starred');
            break;
        case 'view.readLater':
            selectView('readLater');
            break;
        case 'view.archived':
            selectView('archived');
            break;
        case 'view.feeds':
            searchStore.clear();
            selectionStore.focusSidebar();
            break;
        case 'help.toggle':
            toggleHelp();
            break;
        default:
            break;
    }
}

/** The public docs site + issue tracker, opened from the Help menu. */
const DOCS_URL = 'https://alexnodeland.github.io/curio-rss/';
const ISSUES_URL = 'https://github.com/alexnodeland/curio-rss/issues/new';
const SHORTCUT_IDS: ReadonlySet<string> = new Set(SHORTCUTS.map((shortcut) => shortcut.id));

/**
 * Runs a native-menu item id through the same action layer as a keyboard
 * shortcut: a `ShortcutId` runs `handleShortcut`; the two menu-only ids open
 * the docs / issue tracker; anything else (predefined native items like
 * copy/paste) is ignored.
 */
export function routeMenuAction(id: string): void {
    if (id === 'menu.docs') {
        void openExternal(DOCS_URL);
    } else if (id === 'menu.reportIssue') {
        void openExternal(ISSUES_URL);
    } else if (SHORTCUT_IDS.has(id)) {
        handleShortcut(id as ShortcutId);
    }
}

/**
 * Subscribes to the Rust `MenuAction` event and routes each click through
 * {@link routeMenuAction}. Returns an unsubscribe for teardown.
 */
export async function wireMenuActions(): Promise<() => void> {
    return events.menuAction.listen((event) => routeMenuAction(event.payload.id));
}
