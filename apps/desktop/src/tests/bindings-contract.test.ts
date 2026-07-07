/**
 * The bindings contract test — imports EVERY generated command wrapper and
 * invokes it against mockIPC. The `cases` map is typed over
 * `keyof typeof commands`, so regenerating bindings with a new command
 * fails typecheck until a case is added, and each case's `response` must
 * satisfy the wrapper's real return type. This makes the class of bug that
 * broke the old sketch at runtime (snake_case/camelCase argument drift and
 * a drifted error shape under 139 green tests) unrepresentable.
 */
import { commands } from '$lib/bindings';
import { afterEach, describe, expect, it } from 'vitest';
import {
    type IpcHarness,
    appInfoFixture,
    articleFixture,
    articleStateFixture,
    articleSummaryFixture,
    commandErrorFixture,
    dbStatsFixture,
    destinationFixture,
    discoveryFixture,
    feedFixture,
    fetchRecordFixture,
    installIpcHarness,
    opmlImportOutcomeFixture,
    pathTokenFixture,
    refreshOutcomeFixture,
    rejectWith,
    saveOutcomeFixture,
    unreadCountsFixture,
} from './ipc-harness';

type Commands = typeof commands;
type CommandName = keyof Commands;

/** The `data` payload of a wrapper's `{ status: 'ok' }` arm. */
type OkData<K extends CommandName> = Extract<
    Awaited<ReturnType<Commands[K]>>,
    { status: 'ok' }
>['data'];

interface ContractCase<K extends CommandName> {
    args: Parameters<Commands[K]>;
    response: OkData<K>;
}

/** Exhaustive by construction: one case per generated wrapper. */
const cases: { [K in CommandName]: ContractCase<K> } = {
    discoverFeeds: { args: ['https://example.org'], response: discoveryFixture() },
    addFeed: {
        args: [{ url: 'https://example.org/feed.xml', title: null, tags: [] }],
        response: feedFixture(),
    },
    removeFeed: { args: [1], response: null },
    listFeeds: { args: [], response: [feedFixture()] },
    getFeed: { args: [1], response: feedFixture() },
    getFeedByUrl: { args: ['https://example.org/feed.xml'], response: feedFixture() },
    setFeedStatus: { args: [1, 'paused'], response: null },
    updateFeedMetadata: { args: [1, 'New title', null, null], response: null },
    setFeedTags: { args: [1, ['Tech/Databases']], response: null },
    setFeedTitle: { args: [1, 'New name'], response: null },
    refreshFeed: { args: [1], response: refreshOutcomeFixture() },
    refreshAll: { args: [], response: [refreshOutcomeFixture()] },
    recentFetches: { args: [1, 10], response: [fetchRecordFixture()] },
    listArticles: {
        args: [
            {
                feed_id: null,
                before: null,
                limit: 50,
                read: null,
                starred: null,
                read_later: null,
                archived: null,
                tag: null,
                feed_tag: null,
            },
        ],
        response: [articleSummaryFixture()],
    },
    getArticle: { args: [100], response: articleFixture() },
    loadFullArticle: { args: [100], response: articleFixture() },
    getArticleState: { args: [100], response: articleStateFixture() },
    getArticleTags: { args: [100], response: ['rust'] },
    getUnreadCounts: { args: [], response: unreadCountsFixture() },
    countArticles: { args: [], response: 42 },
    deleteArticle: { args: [100], response: null },
    searchArticles: { args: ['rust', 20], response: [articleSummaryFixture()] },
    markRead: { args: [100, true], response: true },
    markAllRead: { args: [1], response: 3 },
    recordOpened: { args: [100, 1500], response: null },
    setArchived: { args: [100, true], response: true },
    setStarred: { args: [100, true], response: true },
    setReadLater: { args: [100, true], response: true },
    tagArticle: { args: [100, 'rust'], response: true },
    untagArticle: { args: [100, 'rust'], response: true },
    listTags: { args: [], response: ['rust'] },
    addDestination: { args: ['notes', '01JZ0000000000000000000000'], response: null },
    listDestinations: { args: [], response: [destinationFixture()] },
    removeDestination: { args: ['notes'], response: null },
    promoteArticle: { args: [100, 'notes'], response: saveOutcomeFixture() },
    importOpml: { args: ['01JZ0000000000000000000000'], response: opmlImportOutcomeFixture() },
    exportOpml: { args: ['01JZ0000000000000000000000'], response: null },
    getSetting: { args: ['ui.theme'], response: 'dark' },
    setSetting: { args: ['ui.theme', 'dark'], response: null },
    getAppInfo: { args: [], response: appInfoFixture() },
    integrityCheck: { args: [], response: null },
    ftsIntegrityCheck: { args: [], response: null },
    vacuumDb: { args: [], response: null },
    dbStats: { args: [], response: dbStatsFixture() },
    sweepEventRetention: { args: [], response: ['2026-03-01.jsonl'] },
    getCachedImage: {
        args: ['https://example.org/img.png'],
        response: '/cache/curio/images/abc123',
    },
    copyDiagnosticsBundle: { args: [], response: pathTokenFixture() },
    pickImportFile: { args: [], response: pathTokenFixture() },
    pickExportPath: { args: [], response: pathTokenFixture() },
    pickDestinationRoot: { args: [], response: pathTokenFixture() },
};

/** The wire name every wrapper must invoke: strict snake_case of its own. */
function wireName(name: string): string {
    return name.replace(/[A-Z]/g, (upper) => `_${upper.toLowerCase()}`);
}

const names = Object.keys(cases) as CommandName[];

describe('generated bindings contract', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        harness?.teardown();
        harness = null;
    });

    it('covers every generated wrapper (runtime belt over the type-level braces)', () => {
        expect(names.sort()).toEqual(Object.keys(commands).sort());
    });

    it.each(names)('%s round-trips through mockIPC', async (name) => {
        const testCase = cases[name];
        const wire = wireName(name);
        harness = installIpcHarness({ [wire]: testCase.response });

        const wrapper = commands[name] as (...args: unknown[]) => Promise<unknown>;
        const result = (await wrapper(...(testCase.args as unknown[]))) as {
            status: string;
            data?: unknown;
        };

        // The wrapper resolved through the generated result envelope.
        expect(result.status).toBe('ok');
        expect(result.data).toEqual(testCase.response);

        // Exactly one invoke, addressed to the snake_case wire name — an
        // unmocked (drifted) name would have thrown inside the harness.
        expect(harness.calls).toHaveLength(1);
        expect(harness.calls[0].cmd).toBe(wire);

        // Argument keys cross IPC in camelCase (tauri's argument casing);
        // snake_case leaking into the payload is the sketch's fatal bug.
        for (const key of Object.keys(harness.calls[0].args)) {
            expect(key).not.toMatch(/_/);
        }
    });

    it('folds a Rust CommandError rejection into the typed error arm', async () => {
        const error = commandErrorFixture({
            kind: 'user',
            code: 'INVALID_INPUT',
            message: 'tags cannot be empty',
            recoverable: false,
        });
        harness = installIpcHarness({ tag_article: rejectWith(error) });

        const result = await commands.tagArticle(100, '');
        expect(result).toEqual({ status: 'error', error });
    });

    it('re-throws real JS errors instead of masking them as command errors', async () => {
        harness = installIpcHarness({});
        await expect(commands.listFeeds()).rejects.toThrow(
            "ipc-harness: no responder for command 'list_feeds'",
        );
    });
});
