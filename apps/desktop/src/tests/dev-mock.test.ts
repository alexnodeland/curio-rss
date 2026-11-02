/**
 * The dev mock IPC (`npm run dev` in a plain browser) must behave like the
 * real commands where the UI can tell: a mutating handler changes the
 * preview data AND emits the `FeedsChanged` / `ArticlesChanged` invalidation
 * the Rust command emits (only when something changed), or the visual-check
 * loop shows badges and lists that never react (#300).
 */
import { events, type ArticlesChanged, type ListArticlesDto, commands } from '$lib/bindings';
import { clearMocks } from '@tauri-apps/api/mocks';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { flushIpc } from './ipc-harness';

interface Recorded {
    articles: ArticlesChanged[];
    feeds: number;
}

let unlisteners: Array<() => void> = [];

/** Installs a fresh mock (its preview data restarts) and records its events. */
async function installMock(): Promise<Recorded> {
    // `installDevMock` installs once per module instance; a fresh instance
    // per test gives each test pristine preview data.
    vi.resetModules();
    const { installDevMock } = await import('$lib/dev/install-mock');
    await installDevMock();
    const recorded: Recorded = { articles: [], feeds: 0 };
    unlisteners = await Promise.all([
        events.articlesChanged.listen((event) => {
            recorded.articles.push(event.payload);
        }),
        events.feedsChanged.listen(() => {
            recorded.feeds += 1;
        }),
    ]);
    return recorded;
}

function ok<T>(result: { status: 'ok'; data: T } | { status: 'error'; error: unknown }): T {
    if (result.status !== 'ok') {
        throw new Error(`expected ok, got ${JSON.stringify(result.error)}`);
    }
    return result.data;
}

const listParams = (overrides: Partial<ListArticlesDto> = {}): ListArticlesDto => ({
    feed_id: null,
    before: null,
    limit: 500,
    read: null,
    starred: null,
    read_later: null,
    archived: null,
    tag: null,
    feed_tag: null,
    ...overrides,
});

async function unreadIn(feedId: number): Promise<number> {
    const counts = ok(await commands.getUnreadCounts());
    return counts.by_feed.find(([id]) => id === feedId)?.[1] ?? 0;
}

describe('dev mock IPC', () => {
    beforeEach(() => {
        unlisteners = [];
    });

    afterEach(() => {
        for (const unlisten of unlisteners) unlisten();
        clearMocks();
    });

    describe('article state → ArticlesChanged', () => {
        it('mark_all_read emits once with its feed, and the counts follow', async () => {
            const recorded = await installMock();
            const before = ok(await commands.getUnreadCounts());
            const feedUnread = await unreadIn(1);
            expect(feedUnread).toBeGreaterThan(0);

            const flipped = ok(await commands.markAllRead(1));
            await flushIpc();

            expect(flipped).toHaveLength(feedUnread);
            expect(recorded.articles).toEqual([{ feed_id: 1 }]);
            expect(await unreadIn(1)).toBe(0);
            expect(ok(await commands.getUnreadCounts()).total).toBe(before.total - feedUnread);

            // Nothing left to change: the command says so and stays silent.
            expect(ok(await commands.markAllRead(1))).toEqual([]);
            await flushIpc();
            expect(recorded.articles).toHaveLength(1);

            // The toast's Undo restores the badge and the All count.
            expect(ok(await commands.markReadMany(flipped, false))).toBe(feedUnread);
            await flushIpc();
            expect(recorded.articles).toEqual([{ feed_id: 1 }, { feed_id: null }]);
            expect(await unreadIn(1)).toBe(feedUnread);
            expect(ok(await commands.getUnreadCounts()).total).toBe(before.total);
        });

        it('a library-wide mark_all_read carries no feed', async () => {
            const recorded = await installMock();
            ok(await commands.markAllRead(null));
            await flushIpc();
            expect(recorded.articles).toEqual([{ feed_id: null }]);
            expect(ok(await commands.getUnreadCounts()).total).toBe(0);
        });

        it('the per-article flags emit only when they change something', async () => {
            const recorded = await installMock();
            const [first] = ok(await commands.listArticles(listParams({ read: false })));

            expect(ok(await commands.markRead(first.id, true))).toBe(true);
            expect(ok(await commands.markRead(first.id, true))).toBe(false);
            expect(ok(await commands.setStarred(first.id, true))).toBe(true);
            expect(ok(await commands.setReadLater(first.id, true))).toBe(true);
            expect(ok(await commands.setArchived(first.id, true))).toBe(true);
            expect(ok(await commands.setArchived(first.id, true))).toBe(false);
            expect(ok(await commands.markReadMany([first.id], true))).toBe(0);
            await flushIpc();

            expect(recorded.articles).toHaveLength(4);
            expect(recorded.articles.every((event) => event.feed_id === null)).toBe(true);
            expect(recorded.feeds).toBe(0);
        });

        it('tagging emits only when the tag set changes', async () => {
            const recorded = await installMock();
            const [first] = ok(await commands.listArticles(listParams()));

            expect(ok(await commands.tagArticle(first.id, 'dev-mock'))).toBe(true);
            expect(ok(await commands.tagArticle(first.id, 'dev-mock'))).toBe(false);
            expect(ok(await commands.untagArticle(first.id, 'dev-mock'))).toBe(true);
            expect(ok(await commands.untagArticle(first.id, 'dev-mock'))).toBe(false);
            await flushIpc();

            expect(recorded.articles).toHaveLength(2);
        });

        it('unread counts leave archived articles out, as core does', async () => {
            await installMock();
            const [first] = ok(await commands.listArticles(listParams({ read: false })));
            const feedId = first.feed_id ?? -1;
            const before = ok(await commands.getUnreadCounts());
            const feedBefore = await unreadIn(feedId);

            ok(await commands.setArchived(first.id, true));

            expect(ok(await commands.getUnreadCounts()).total).toBe(before.total - 1);
            expect(await unreadIn(feedId)).toBe(feedBefore - 1);
        });
    });

    describe('feed writes → FeedsChanged', () => {
        it('remove_feed drops the feed for good and orphans its articles', async () => {
            const recorded = await installMock();
            const before = ok(await commands.getUnreadCounts());
            const ownArticles = ok(await commands.listArticles(listParams({ feed_id: 1 })));
            expect(ownArticles.length).toBeGreaterThan(0);

            ok(await commands.stageFeedRemoval(1));
            ok(await commands.removeFeed(1));
            await flushIpc();

            expect(recorded.feeds).toBe(1);
            expect(recorded.articles).toEqual([{ feed_id: null }]);
            const feeds = ok(await commands.listFeeds());
            expect(feeds.some((feed) => feed.id === 1)).toBe(false);
            expect(ok(await commands.listArticles(listParams({ feed_id: 1 })))).toEqual([]);
            const counts = ok(await commands.getUnreadCounts());
            expect(counts.by_feed.some(([id]) => id === 1)).toBe(false);
            // Core's `ON DELETE SET NULL`: the articles stay in the library.
            expect(counts.total).toBe(before.total);
            const article = ok(await commands.getArticle(ownArticles[0].id));
            expect(article?.feed_id).toBeNull();

            // Gone for good: a second removal finds nothing, and says nothing.
            const again = await commands.removeFeed(1);
            expect(again.status).toBe('error');
            expect(again.status === 'error' && again.error.code).toBe('NOT_FOUND');
            await flushIpc();
            expect(recorded.feeds).toBe(1);
        });

        it('the feed-row writes change the feed and emit', async () => {
            const recorded = await installMock();

            ok(await commands.setFeedTitle(2, 'Renamed'));
            ok(await commands.setFeedTags(2, ['Moved']));
            ok(await commands.setFeedStatus(2, 'paused'));
            ok(await commands.setFeedMetadata(2, 'https://example.com', 'About'));
            ok(await commands.setFeedFullText(2, true));
            ok(await commands.updateFeedMetadata(2, 'Retitled', null, null));
            await flushIpc();

            expect(recorded.feeds).toBe(6);
            expect(recorded.articles).toEqual([]);
            const feed = ok(await commands.listFeeds()).find((candidate) => candidate.id === 2);
            expect(feed).toMatchObject({
                title: 'Retitled',
                tags: ['Moved'],
                status: 'paused',
                site_url: null,
                description: null,
                fetch_full_text: true,
            });

            const missing = await commands.setFeedTitle(9999, 'Nope');
            expect(missing.status === 'error' && missing.error.code).toBe('NOT_FOUND');
            await flushIpc();
            expect(recorded.feeds).toBe(6);
        });

        it('reorder_feeds lists the feeds in the new order', async () => {
            const recorded = await installMock();
            const ids = ok(await commands.listFeeds()).map((feed) => feed.id);
            const reversed = [...ids].reverse();

            ok(await commands.reorderFeeds(reversed));
            await flushIpc();

            expect(recorded.feeds).toBe(1);
            expect(ok(await commands.listFeeds()).map((feed) => feed.id)).toEqual(reversed);

            // A partial order moves only the listed feeds' positions.
            ok(await commands.reorderFeeds([ids[0]]));
            const partial = ok(await commands.listFeeds()).map((feed) => feed.id);
            expect(partial[0]).toBe(ids[0]);
            expect(partial).toHaveLength(ids.length);
        });

        it('add_feed subscribes a new feed once', async () => {
            const recorded = await installMock();
            const before = ok(await commands.listFeeds());
            const newFeed = {
                url: 'https://example.com/feed.xml',
                title: 'Example',
                tags: ['Dev'],
                validate: true,
            };

            const added = ok(await commands.addFeed(newFeed));
            await flushIpc();

            expect(recorded.feeds).toBe(1);
            expect(added).toMatchObject({ url: newFeed.url, title: 'Example', tags: ['Dev'] });
            expect(before.some((feed) => feed.id === added.id)).toBe(false);
            const after = ok(await commands.listFeeds());
            expect(after.at(-1)?.id).toBe(added.id);

            const duplicate = await commands.addFeed(newFeed);
            expect(duplicate.status === 'error' && duplicate.error.code).toBe('ALREADY_SUBSCRIBED');
            await flushIpc();
            expect(recorded.feeds).toBe(1);
        });

        it('refresh_feed announces both changes, scoped to its feed', async () => {
            const recorded = await installMock();
            ok(await commands.refreshFeed(3));
            await flushIpc();
            expect(recorded.feeds).toBe(1);
            expect(recorded.articles).toEqual([{ feed_id: 3 }]);
        });
    });
});
