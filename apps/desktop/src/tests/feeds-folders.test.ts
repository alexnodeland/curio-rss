/**
 * The sidebar folder operations on the feeds store: move-to-folder / rename /
 * delete are per-feed `set_feed_tags` rewrites; collapse + pending-folder state
 * persists through the settings table. Feeds are a set — every op preserves
 * unrelated tags.
 */
import { FeedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    feedFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';

async function storeWithFeeds(
    feeds: ReturnType<typeof feedFixture>[],
    extra: Record<string, Responder> = {},
): Promise<{ store: FeedsStore; harness: IpcHarness }> {
    const harness = installIpcHarness({
        list_feeds: feeds,
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        set_feed_tags: null,
        set_setting: null,
        mark_all_read: null,
        ...extra,
    });
    const store = new FeedsStore();
    void store.feeds; // kick the fetch
    await flushIpc();
    return { store, harness };
}

describe('feeds store — folder operations', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('keeps a folder visible when its last feed is ungrouped out of it', async () => {
        const setup = await storeWithFeeds([feedFixture({ id: 1, tags: ['Solo'] })]);
        harness = setup.harness;

        await setup.store.ungroupFeed(1);

        // The now-empty 'Solo' folder is re-scaffolded as pending so it doesn't
        // silently vanish with its last feed.
        const pendingWrites = harness
            .callsFor('set_setting')
            .filter((call) => call.key === 'ui.sidebar.pending-folders');
        expect(pendingWrites.at(-1)?.value).toBe(JSON.stringify(['Solo']));
    });

    it('does not re-scaffold a folder another feed still occupies', async () => {
        const setup = await storeWithFeeds([
            feedFixture({ id: 1, tags: ['Shared'] }),
            feedFixture({ id: 2, tags: ['Shared'], url: 'https://b.example/feed' }),
        ]);
        harness = setup.harness;

        await setup.store.ungroupFeed(1);

        const pendingWrites = harness
            .callsFor('set_setting')
            .filter((call) => call.key === 'ui.sidebar.pending-folders');
        // 'Shared' still holds feed 2, so nothing is re-scaffolded.
        expect(pendingWrites).toHaveLength(0);
    });

    it('moveFeedToFolder retags the feed and clears the pending scaffold', async () => {
        const setup = await storeWithFeeds([feedFixture({ id: 1, tags: ['Old', 'Keep'] })]);
        harness = setup.harness;
        setup.store.createFolder('Target');

        await setup.store.moveFeedToFolder(1, 'Target');

        expect(harness.callsFor('set_feed_tags')).toEqual([
            { feedId: 1, tags: ['Target', 'Keep'] },
        ]);
        // The pending scaffold is dropped once the folder holds a feed.
        const pendingWrites = harness
            .callsFor('set_setting')
            .filter((call) => call.key === 'ui.sidebar.pending-folders');
        expect(pendingWrites.at(-1)?.value).toBe('[]');
    });

    it('renameFolder rewrites the tag prefix across the subtree', async () => {
        const setup = await storeWithFeeds([
            feedFixture({ id: 1, tags: ['Tech'] }),
            feedFixture({ id: 2, tags: ['Tech/DB'] }),
            feedFixture({ id: 3, tags: ['News'] }),
        ]);
        harness = setup.harness;

        await setup.store.renameFolder('Tech', 'Eng');

        expect(harness.callsFor('set_feed_tags')).toEqual([
            { feedId: 1, tags: ['Eng'] },
            { feedId: 2, tags: ['Eng/DB'] },
        ]);
    });

    it('deleteFolder moves feeds to the parent, never unsubscribing', async () => {
        const setup = await storeWithFeeds([feedFixture({ id: 1, tags: ['A/B', 'A/B/C'] })]);
        harness = setup.harness;

        await setup.store.deleteFolder('A/B');

        expect(harness.callsFor('set_feed_tags')).toEqual([{ feedId: 1, tags: ['A', 'A/C'] }]);
        expect(harness.callsFor('remove_feed')).toHaveLength(0);
    });

    it('markFolderRead marks every feed in the subtree read', async () => {
        const setup = await storeWithFeeds([
            feedFixture({ id: 1, tags: ['Tech'] }),
            feedFixture({ id: 2, tags: ['Tech/DB'] }),
        ]);
        harness = setup.harness;
        const tree = (await import('$lib/state/feed-tree')).buildFeedTree(setup.store.feeds);

        await setup.store.markFolderRead(tree.folders[0]);

        expect(
            harness
                .callsFor('mark_all_read')
                .map((call) => call.feedId)
                .sort(),
        ).toEqual([1, 2]);
    });
});

describe('feeds store — sidebar persistence', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('toggleFolder persists the collapse set', async () => {
        harness = installIpcHarness({ set_setting: null });
        const store = new FeedsStore();

        store.toggleFolder('Tech');

        const writes = harness
            .callsFor('set_setting')
            .filter((call) => call.key === 'ui.sidebar.collapsed-folders');
        expect(writes.at(-1)?.value).toBe('["Tech"]');
    });

    it('initSidebarState adopts persisted collapse + pending folders', async () => {
        harness = installIpcHarness({
            get_setting: (args) => {
                if (args.key === 'ui.sidebar.collapsed-folders') return JSON.stringify(['Tech']);
                if (args.key === 'ui.sidebar.pending-folders') return JSON.stringify(['Ideas']);
                return null;
            },
        });
        await settingsStore.load();
        const store = new FeedsStore();

        store.initSidebarState();

        expect(store.isFolderCollapsed('Tech')).toBe(true);
        expect(store.pendingPaths).toEqual(['Ideas']);
    });
});
