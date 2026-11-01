/**
 * Retry on sticky error toasts (#144): a Retry re-runs the action that
 * failed — the same article, the same absolute target, the caller's whole
 * flow — never whatever the UI points at by the time it is clicked; failures
 * from different actions never merge; a later success withdraws a stale
 * Retry; and input the backend rejected gets no Retry at all.
 */
import { events } from '$lib/bindings';
import {
    deleteFolder,
    loadFullArticle,
    promoteSelected,
    renameFeed,
    reorderFeedGroup,
    runCommand,
    toggleStar,
} from '$lib/state/actions';
import { articlesStore } from '$lib/state/articles.svelte';
import { destinationsStore } from '$lib/state/destinations.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache, wireInvalidation } from '$lib/state/query-cache.svelte';
import { selectionStore } from '$lib/state/selection.svelte';
import { SETTING_KEYS, settingsStore } from '$lib/state/settings.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    articleStateFixture,
    commandErrorFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
    saveOutcomeFixture,
    unreadCountsFixture,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

/** A failure retrying can fix (the drive came back, the lock was released). */
const transient = commandErrorFixture({ kind: 'user', code: 'IO', recoverable: true });

function retryToast(): { id: number; message: string } {
    const toast = toastStore.toasts.find((candidate) => candidate.action?.label === 'Retry');
    if (toast === undefined) throw new Error('no Retry toast');
    return toast;
}

describe('retry plans', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        resetQueryCache();
        articlesStore.reset();
        selectionStore.reset();
        settingsStore.reset();
        destinationsStore.reset();
        feedsStore.reset();
        resetUiStores();
        harness?.teardown();
        harness = null;
    });

    it('a promote Retry promotes the article that failed, not the one selected now', async () => {
        harness = installIpcHarness({
            list_destinations: [
                { name: 'vault', root: '/v' },
                { name: 'inbox', root: '/i' },
            ],
            get_setting: (args) => (args.key === SETTING_KEYS.promoteDestination ? 'vault' : null),
            set_setting: null,
            promote_article: rejectWith(transient),
        });
        await settingsStore.load();
        selectionStore.selectedArticleId = 100;

        await promoteSelected();
        await flushIpc();
        expect(harness.callsFor('promote_article')).toEqual([
            { articleId: 100, destination: 'vault' },
        ]);

        // The user moves on (j/k) and changes the default destination before
        // clicking the sticky toast's Retry.
        selectionStore.selectedArticleId = 200;
        await settingsStore.set(SETTING_KEYS.promoteDestination, 'inbox');
        expect(destinationsStore.promoteTarget).toBe('inbox');
        harness.respond(
            'promote_article',
            saveOutcomeFixture({ destination: 'vault', disposition: 'created' }),
        );
        toastStore.runAction(retryToast().id);
        await flushIpc();

        expect(harness.callsFor('promote_article')).toEqual([
            { articleId: 100, destination: 'vault' },
            { articleId: 100, destination: 'vault' },
        ]);
        expect(toastStore.toasts.at(-1)?.message).toBe('Saved to vault');
    });

    it('a failed star write retries the value it tried to write, not a fresh flip', async () => {
        harness = installIpcHarness({
            get_article_state: articleStateFixture({ starred: false }),
            set_starred: rejectWith(transient),
        });
        await toggleStar(7);
        expect(harness.callsFor('set_starred')).toEqual([{ articleId: 7, starred: true }]);

        // Meanwhile the flag changed (another surface starred it): a relative
        // retry would read `true` and unstar — the opposite of the request.
        harness.respond('get_article_state', articleStateFixture({ starred: true }));
        harness.respond('set_starred', true);
        toastStore.runAction(retryToast().id);
        await flushIpc();
        expect(harness.callsFor('set_starred').at(-1)).toEqual({ articleId: 7, starred: true });
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('a later success on the same flag withdraws the stale Retry', async () => {
        harness = installIpcHarness({
            get_article_state: articleStateFixture({ starred: false }),
            set_starred: rejectWith(transient),
        });
        await toggleStar(7);
        expect(retryToast()).toBeDefined();

        // The user toggles again and this time it lands: replaying the old
        // write would overwrite that newer choice, so its toast goes away.
        harness.respond('set_starred', true);
        await toggleStar(7);
        expect(toastStore.toasts.some((toast) => toast.action?.label === 'Retry')).toBe(false);
    });

    it('two different actions failing with the same text keep separate Retries', async () => {
        const internal = commandErrorFixture({ kind: 'internal', recoverable: true });
        harness = installIpcHarness({
            get_article_state: articleStateFixture({ starred: false }),
            set_starred: rejectWith(internal),
            refresh_feed: rejectWith(internal),
        });
        await toggleStar(7);
        await runCommand(() => feedsStore.refreshFeed(3), { key: 'refresh:3' });
        const retries = toastStore.toasts.filter((toast) => toast.action?.label === 'Retry');
        expect(retries).toHaveLength(2);
        expect(retries[0]?.message).toBe(retries[1]?.message);

        // Each Retry re-runs its own action.
        harness.respond('set_starred', true);
        harness.respond('refresh_feed', null);
        for (const toast of retries) toastStore.runAction(toast.id);
        await flushIpc();
        expect(harness.callsFor('set_starred')).toHaveLength(2);
        expect(harness.callsFor('refresh_feed')).toEqual([{ feedId: 3 }, { feedId: 3 }]);
    });

    it('a repeat of the same failure merges into one toast', async () => {
        harness = installIpcHarness({ refresh_feed: rejectWith(transient) });
        await runCommand(() => feedsStore.refreshFeed(3), { key: 'refresh:3' });
        await runCommand(() => feedsStore.refreshFeed(3), { key: 'refresh:3' });
        expect(toastStore.toasts).toHaveLength(1);
    });

    it.each(['INVALID_INPUT', 'INVALID_URL', 'PATH_TOKEN_EXPIRED'] as const)(
        '%s input gets no Retry even when the backend marks it recoverable',
        async (code) => {
            harness = installIpcHarness({
                set_feed_title: rejectWith(
                    commandErrorFixture({
                        kind: 'user',
                        code,
                        message: 'bad title',
                        recoverable: true,
                    }),
                ),
            });
            await renameFeed(1, '   ');
            expect(toastStore.toasts).toHaveLength(1);
            expect(toastStore.toasts[0]?.action).toBeUndefined();
        },
    );

    it('a Reddit auth refusal offers Retry only when a retry can get a token', async () => {
        const refusal = (reason: string, recoverable: boolean) =>
            rejectWith(
                commandErrorFixture({
                    kind: 'user',
                    code: 'REDDIT_AUTH',
                    params: { reason, status: '403' },
                    message: 'reddit refused',
                    recoverable,
                }),
            );
        harness = installIpcHarness({ load_full_article: refusal('credentials_missing', false) });
        expect(await loadFullArticle(7)).toBe(false);
        expect(toastStore.toasts).toHaveLength(1);
        expect(toastStore.toasts[0]?.message).toContain('Settings → Reddit API');
        expect(toastStore.toasts[0]?.action).toBeUndefined();
        harness.teardown();
        resetUiStores();

        harness = installIpcHarness({ load_full_article: refusal('token_unavailable', true) });
        expect(await loadFullArticle(7)).toBe(false);
        expect(retryToast().message).toContain('access token');
    });

    it("a folder delete's Retry runs the whole flow, announcing the delete", async () => {
        harness = installIpcHarness({
            list_feeds: [feedFixture({ id: 1, tags: ['Tech'] })],
            get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
            set_feed_tags: rejectWith(transient),
            set_setting: null,
        });
        void feedsStore.feeds;
        await flushIpc();

        await deleteFolder('Tech', 'Tech');
        expect(toastStore.toasts.map((toast) => toast.message)).not.toContain('Deleted “Tech”');

        harness.respond('set_feed_tags', null);
        toastStore.runAction(retryToast().id);
        await flushIpc();
        expect(harness.callsFor('set_feed_tags')).toHaveLength(2);
        expect(toastStore.toasts.at(-1)?.tone).toBe('info');
        expect(toastStore.toasts.some((toast) => toast.tone === 'error')).toBe(false);
    });

    it('a reorder Retry rebuilds the order from the feeds as they are now', async () => {
        harness = installIpcHarness({
            list_feeds: [feedFixture({ id: 1 }), feedFixture({ id: 2, url: 'https://b.test/f' })],
            get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
            reorder_feeds: rejectWith(transient),
        });
        void feedsStore.feeds;
        await flushIpc();

        await reorderFeedGroup([2, 1]);
        expect(harness.callsFor('reorder_feeds')).toEqual([{ feedIds: [2, 1] }]);

        // A feed subscribed since the failure is kept, not dropped by a replay.
        harness.respond('list_feeds', [
            feedFixture({ id: 1 }),
            feedFixture({ id: 2, url: 'https://b.test/f' }),
            feedFixture({ id: 3, url: 'https://c.test/f' }),
        ]);
        const unwire = await wireInvalidation();
        await events.feedsChanged.emit();
        await flushIpc();
        unwire();
        expect(feedsStore.feeds).toHaveLength(3);
        harness.respond('reorder_feeds', null);
        toastStore.runAction(retryToast().id);
        await flushIpc();
        expect(harness.callsFor('reorder_feeds').at(-1)).toEqual({ feedIds: [2, 1, 3] });
    });
});
