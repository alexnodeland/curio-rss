/**
 * Undo and Retry on destructive / bulk actions (#144): mark-all-read hands
 * back the ids it flipped and Undo restores exactly that set; a folder
 * mark-read is all-or-nothing; the sidebar's one-click unsubscribe waits out
 * an undo window before the backend removal; and error toasts offer a Retry
 * that re-invokes the command (only when the failure is recoverable).
 */
import Sidebar from '$components/sidebar/Sidebar.svelte';
import { events } from '$lib/bindings';
import { markAllRead, markFolderRead, runCommand } from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { buildFeedTree } from '$lib/state/feed-tree';
import { feedsStore } from '$lib/state/feeds.svelte';
import { menuStore } from '$lib/state/menu.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { sidebarTreeStore } from '$lib/state/sidebar-tree.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    commandErrorFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

/**
 * A tiny in-memory read-state backend: `mark_all_read` flips what is unread
 * (optionally one feed's) and returns those ids, `mark_read_many` sets the
 * flag on the given ids and returns how many changed — the Rust semantics.
 */
function readStateBackend(articles: Record<number, { feed: number; read: boolean }>) {
    const unread = (): number[] =>
        Object.entries(articles)
            .filter(([, article]) => !article.read)
            .map(([id]) => Number(id))
            .sort((a, b) => a - b);
    return {
        unread,
        responders: {
            mark_all_read: (args: Record<string, unknown>) => {
                const feedId = args.feedId as number | null;
                const flipped = unread().filter(
                    (id) => feedId === null || articles[id]?.feed === feedId,
                );
                for (const id of flipped) (articles[id] as { read: boolean }).read = true;
                return flipped;
            },
            mark_read_many: (args: Record<string, unknown>) => {
                let changed = 0;
                for (const id of args.articleIds as number[]) {
                    const article = articles[id];
                    if (article !== undefined && article.read !== args.read) {
                        article.read = args.read as boolean;
                        changed += 1;
                    }
                }
                return changed;
            },
        },
    };
}

function undoToastId(): number {
    const toast = toastStore.toasts.find((candidate) => candidate.action?.label === 'Undo');
    if (toast === undefined) throw new Error('no Undo toast');
    return toast.id;
}

describe('undo — mark read', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        resetUiStores();
        feedsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('mark-all-read → Undo restores exactly the previous unread set', async () => {
        const backend = readStateBackend({
            1: { feed: 1, read: false },
            2: { feed: 1, read: true }, // read before the sweep: must stay read
            3: { feed: 2, read: false },
            4: { feed: 2, read: false },
        });
        harness = installIpcHarness(backend.responders);
        const before = backend.unread();

        await markAllRead(null);
        expect(backend.unread()).toEqual([]);
        expect(toastStore.toasts[0]?.message).toBe('Marked 3 read');

        toastStore.runAction(undoToastId());
        await flushIpc();
        expect(backend.unread()).toEqual(before);
        expect(harness.callsFor('mark_read_many')).toEqual([
            { articleIds: [1, 3, 4], read: false },
        ]);
        expect(toastStore.toasts.at(-1)?.message).toBe('Marked 3 unread again');
    });

    it('a feed-scoped sweep undoes only that feed', async () => {
        const backend = readStateBackend({
            1: { feed: 1, read: false },
            3: { feed: 2, read: false },
        });
        harness = installIpcHarness(backend.responders);

        await markAllRead(1);
        expect(backend.unread()).toEqual([3]);
        toastStore.runAction(undoToastId());
        await flushIpc();
        expect(backend.unread()).toEqual([1, 3]);
    });

    it('nothing to mark → no Undo is offered', async () => {
        harness = installIpcHarness({ mark_all_read: [] });
        await markAllRead(null);
        expect(toastStore.toasts[0]?.message).toBe('Marked 0 read');
        expect(toastStore.toasts[0]?.action).toBeUndefined();
    });

    it('folder mark-read is all-or-nothing: a failure part-way rolls back', async () => {
        const backend = readStateBackend({
            1: { feed: 1, read: false },
            2: { feed: 2, read: false },
        });
        harness = installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1, tags: ['Tech'] }),
                feedFixture({ id: 2, tags: ['Tech/DB'] }),
            ],
            get_unread_counts: unreadCountsFixture({ total: 2, by_feed: [] }),
            ...backend.responders,
        });
        void feedsStore.feeds;
        await flushIpc();
        const folder = buildFeedTree(feedsStore.feeds).folders[0];
        if (folder === undefined) throw new Error('no folder');
        // The second feed's sweep fails after the first one succeeded.
        let calls = 0;
        harness.respond('mark_all_read', (args) => {
            calls += 1;
            if (calls === 2) {
                return Promise.reject(commandErrorFixture({ kind: 'internal', recoverable: true }));
            }
            return backend.responders.mark_all_read(args);
        });

        await markFolderRead(folder);
        expect(backend.unread()).toEqual([1, 2]); // the first feed was rolled back
        const error = toastStore.toasts.find((toast) => toast.tone === 'error');
        expect(error?.action?.label).toBe('Retry');

        // Retry runs the whole folder again; this time both succeed, with Undo.
        toastStore.runAction(error?.id ?? -1);
        await flushIpc();
        expect(backend.unread()).toEqual([]);
        toastStore.runAction(undoToastId());
        await flushIpc();
        expect(backend.unread()).toEqual([1, 2]);
    });
});

describe('retry on error toasts', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('a recoverable failure stays up with Retry, which re-invokes the command', async () => {
        vi.useFakeTimers();
        harness = installIpcHarness({
            refresh_feed: rejectWith(commandErrorFixture({ kind: 'internal', recoverable: true })),
        });
        await runCommand(() => feedsStore.refreshFeed(4));
        vi.advanceTimersByTime(30_000); // well past 8 s: errors are sticky
        vi.useRealTimers();
        const [toast] = toastStore.toasts;
        expect(toast?.tone).toBe('error');
        expect(toast?.action?.label).toBe('Retry');

        harness.respond('refresh_feed', null);
        toastStore.runAction(toast?.id ?? -1);
        await flushIpc();
        expect(harness.callsFor('refresh_feed')).toEqual([{ feedId: 4 }, { feedId: 4 }]);
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('a non-recoverable failure offers no Retry (it cannot help)', async () => {
        harness = installIpcHarness({
            refresh_feed: rejectWith(commandErrorFixture({ recoverable: false })),
        });
        await runCommand(() => feedsStore.refreshFeed(4));
        expect(toastStore.toasts[0]?.action).toBeUndefined();
    });
});

describe('undo — sidebar unsubscribe', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        sidebarTreeStore.reset();
        resetUiStores();
        feedsStore.reset();
        menuStore.reset();
        harness?.teardown();
        harness = null;
        vi.useRealTimers();
    });

    function feedsHarness(): IpcHarness {
        return installIpcHarness({
            list_feeds: [
                feedFixture({ id: 1, title: 'Alpha' }),
                feedFixture({ id: 2, title: 'Beta', url: 'https://beta.example/feed' }),
            ],
            get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
            remove_feed: null,
            stage_feed_removal: null,
            unstage_feed_removal: null,
            set_setting: null,
        });
    }

    /** Opens Alpha's context menu and picks Unsubscribe. */
    function unsubscribeAlpha(getByLabelText: (label: string) => HTMLElement): void {
        fireEvent.contextMenu(getByLabelText('Alpha'));
        const item = menuStore.current?.items.find((candidate) => candidate.id === 'unsub');
        item?.onSelect?.();
    }

    it('hides the feed at once and Undo brings it back without removing anything', async () => {
        harness = feedsHarness();
        const { getByLabelText, queryByLabelText } = render(Sidebar);
        await flushIpc();

        unsubscribeAlpha(getByLabelText);
        await flushIpc();
        expect(queryByLabelText('Alpha')).toBeNull();
        expect(toastStore.toasts[0]?.message).toBe('Unsubscribed “Alpha”');
        expect(harness.callsFor('remove_feed')).toHaveLength(0);

        toastStore.runAction(undoToastId());
        await flushIpc();
        expect(queryByLabelText('Alpha')).not.toBeNull();
        expect(harness.callsFor('remove_feed')).toHaveLength(0);
    });

    it('the removal goes ahead once the undo window runs out', async () => {
        harness = feedsHarness();
        const { getByLabelText, queryByLabelText } = render(Sidebar);
        await flushIpc();

        vi.useFakeTimers();
        unsubscribeAlpha(getByLabelText);
        vi.advanceTimersByTime(7000);
        expect(harness.callsFor('remove_feed')).toHaveLength(0); // still undoable
        harness.respond('list_feeds', [
            feedFixture({ id: 2, title: 'Beta', url: 'https://beta.example/feed' }),
        ]);
        vi.advanceTimersByTime(10_000);
        vi.useRealTimers();
        await flushIpc();
        await flushIpc();

        expect(harness.callsFor('remove_feed')).toEqual([{ feedId: 1 }]);
        expect(queryByLabelText('Alpha')).toBeNull();
    });

    it('a failed removal shows the feed again, with Retry', async () => {
        harness = feedsHarness();
        harness.respond(
            'remove_feed',
            rejectWith(commandErrorFixture({ kind: 'internal', recoverable: true })),
        );
        const { getByLabelText, queryByLabelText } = render(Sidebar);
        await flushIpc();

        unsubscribeAlpha(getByLabelText);
        toastStore.dismissToast(toastStore.toasts[0]?.id ?? -1); // closing = go ahead
        await flushIpc();
        await flushIpc();

        expect(harness.callsFor('remove_feed')).toHaveLength(1);
        expect(queryByLabelText('Alpha')).not.toBeNull();
        expect(toastStore.toasts.at(-1)?.action?.label).toBe('Retry');
    });

    it('stages the removal backend-side (for a quit), and Undo withdraws it', async () => {
        harness = feedsHarness();
        const { getByLabelText } = render(Sidebar);
        await flushIpc();

        unsubscribeAlpha(getByLabelText);
        await flushIpc();
        expect(harness.callsFor('stage_feed_removal')).toEqual([{ feedId: 1 }]);
        toastStore.runAction(undoToastId());
        await flushIpc();
        expect(harness.callsFor('unstage_feed_removal')).toEqual([{ feedId: 1 }]);
        expect(harness.callsFor('remove_feed')).toHaveLength(0);
    });

    it('Undo goes back to the unsubscribed feed when it was the one selected', async () => {
        harness = feedsHarness();
        const { getByLabelText } = render(Sidebar);
        await flushIpc();
        selectionStore.selectFeed(1);

        unsubscribeAlpha(getByLabelText);
        expect(selectionStore.selectedFeedId).toBeNull(); // the list left the feed
        toastStore.runAction(undoToastId());
        expect(selectionStore.selectedFeedId).toBe(1);
        expect(articlesStore.filters.feedId).toBe(1);
    });

    it('Undo leaves the selection alone once the user has moved on', async () => {
        harness = feedsHarness();
        const { getByLabelText } = render(Sidebar);
        await flushIpc();
        selectionStore.selectFeed(1);

        unsubscribeAlpha(getByLabelText);
        selectionStore.selectFeed(2);
        toastStore.runAction(undoToastId());
        expect(selectionStore.selectedFeedId).toBe(2);
    });

    it('a removed feed stays hidden while the list from before the removal is up', async () => {
        harness = feedsHarness();
        const { getByLabelText, queryByLabelText } = render(Sidebar);
        await flushIpc();

        unsubscribeAlpha(getByLabelText);
        // The list refetch after the removal fails: the stale list (still
        // naming Alpha) stays on screen, and Alpha must not flash back.
        harness.respond(
            'list_feeds',
            rejectWith(commandErrorFixture({ kind: 'internal', recoverable: true })),
        );
        toastStore.dismissToast(toastStore.toasts[0]?.id ?? -1);
        await flushIpc();
        await flushIpc();
        expect(harness.callsFor('remove_feed')).toEqual([{ feedId: 1 }]);
        expect(queryByLabelText('Alpha')).toBeNull();

        // The next list that lands is the truth: a feed under a reused id shows.
        harness.respond('list_feeds', [
            feedFixture({ id: 1, title: 'Alpha', url: 'https://alpha-again.example/feed' }),
        ]);
        const unwire = await wireInvalidation();
        await events.feedsChanged.emit();
        await flushIpc();
        unwire();
        expect(queryByLabelText('Alpha')).not.toBeNull();
    });
});
