/**
 * The mockIPC test harness — every frontend test talks to the generated
 * bindings through this. It installs `@tauri-apps/api/mocks` with typed
 * fixture responders keyed by wire (snake_case) command name, records every
 * call for assertions, and enables the mock event bus so tests can fire the
 * Rust-emitted invalidation events through the generated event bindings
 * (`events.articlesChanged.emit(...)` etc.).
 *
 * Unmocked commands throw (a real `Error`, which the generated wrappers
 * re-throw) so a test can never silently exercise a command it did not
 * declare.
 */
import type {
    AppInfoDto,
    ArticleDto,
    ArticleStateDto,
    ArticleSummaryDto,
    CommandError,
    DbStatsDto,
    DestinationDto,
    DiscoveryDto,
    FeedDto,
    FetchRecordDto,
    ImportOutcomeDto,
    OpmlImportOutcomeDto,
    PathTokenDto,
    RefreshOutcomeDto,
    SaveOutcomeDto,
    UnreadCountsDto,
} from '$lib/bindings';
import { clearMocks, mockIPC } from '@tauri-apps/api/mocks';

/** A recorded IPC call. */
export interface IpcCall {
    cmd: string;
    args: Record<string, unknown>;
}

/** Computes a fixture response from the invoke args. */
export type CommandResponder = (args: Record<string, unknown>) => unknown;

/**
 * A static fixture value or a responder function. (The union is spelled out
 * — not `unknown` — so responder lambdas get a contextual parameter type.)
 */
export type Responder = CommandResponder | string | number | boolean | null | object;

export interface IpcHarness {
    /** Every non-event invoke, in order. */
    readonly calls: IpcCall[];
    /** The recorded args of every call to one command. */
    callsFor(cmd: string): Record<string, unknown>[];
    /** Adds or replaces a responder after installation. */
    respond(cmd: string, responder: Responder): void;
    /** Uninstalls the mock (also wired into `afterEach` by callers). */
    teardown(): void;
}

/**
 * Installs the harness. `responders` maps wire command names (snake_case,
 * as the Rust side registers them) to fixture values or functions.
 */
export function installIpcHarness(responders: Record<string, Responder> = {}): IpcHarness {
    const table = new Map<string, Responder>(Object.entries(responders));
    const calls: IpcCall[] = [];

    mockIPC(
        (cmd, args) => {
            const record = (args ?? {}) as Record<string, unknown>;
            calls.push({ cmd, args: record });
            if (!table.has(cmd)) {
                throw new Error(`ipc-harness: no responder for command '${cmd}'`);
            }
            const responder = table.get(cmd);
            if (typeof responder === 'function') {
                return (responder as CommandResponder)(record);
            }
            return structuredClone(responder);
        },
        { shouldMockEvents: true },
    );

    return {
        calls,
        callsFor(cmd: string): Record<string, unknown>[] {
            return calls.filter((call) => call.cmd === cmd).map((call) => call.args);
        },
        respond(cmd: string, responder: Responder): void {
            table.set(cmd, responder);
        },
        teardown(): void {
            clearMocks();
        },
    };
}

/**
 * A responder that rejects the way real Tauri does: with the serialized
 * `CommandError` value (not a JS `Error`), which the generated wrappers
 * fold into `{ status: 'error', error }`.
 */
export function rejectWith(error: CommandError): CommandResponder {
    return () => Promise.reject(error);
}

/** Lets queued microtasks and the mock's promise chains settle. */
export async function flushIpc(): Promise<void> {
    await new Promise((resolve) => {
        setTimeout(resolve, 0);
    });
}

// ─── Typed fixture factories ────────────────────────────────────────────────

export function commandErrorFixture(overrides: Partial<CommandError> = {}): CommandError {
    return {
        kind: 'user',
        code: 'NOT_FOUND',
        message: 'no such row',
        recoverable: false,
        ...overrides,
    };
}

export function feedFixture(overrides: Partial<FeedDto> = {}): FeedDto {
    return {
        id: 1,
        url: 'https://example.org/feed.xml',
        title: 'Example Feed',
        site_url: 'https://example.org',
        description: null,
        status: 'active',
        added_at: '2026-07-01T10:00:00.000Z',
        last_fetched_at: null,
        tags: [],
        ...overrides,
    };
}

export function articleSummaryFixture(
    overrides: Partial<ArticleSummaryDto> = {},
): ArticleSummaryDto {
    return {
        id: 100,
        feed_id: 1,
        title: 'An article',
        source_url: 'https://example.org/post/100',
        author: null,
        published_at: '2026-07-01T09:00:00.000Z',
        saved_at: '2026-07-01T10:00:00.000Z',
        word_count: 250,
        lang: 'en',
        image: null,
        snippet: 'A short preview of the article body.',
        ...overrides,
    };
}

export function articleFixture(overrides: Partial<ArticleDto> = {}): ArticleDto {
    return {
        id: 100,
        curio_id: '019780f0-0000-7000-8000-000000000001',
        feed_id: 1,
        title: 'An article',
        source_url: 'https://example.org/post/100',
        author: null,
        published_at: '2026-07-01T09:00:00.000Z',
        content_html: '<p>Body</p>',
        content_text: 'Body',
        lang: 'en',
        word_count: 250,
        saved_at: '2026-07-01T10:00:00.000Z',
        source_updated_at: null,
        ...overrides,
    };
}

export function articleStateFixture(overrides: Partial<ArticleStateDto> = {}): ArticleStateDto {
    return {
        read: false,
        starred: false,
        read_later: false,
        archived: false,
        ...overrides,
    };
}

export function unreadCountsFixture(overrides: Partial<UnreadCountsDto> = {}): UnreadCountsDto {
    return {
        total: 3,
        by_feed: [
            [1, 2],
            [2, 1],
        ],
        ...overrides,
    };
}

export function refreshOutcomeFixture(
    overrides: Partial<RefreshOutcomeDto> = {},
): RefreshOutcomeDto {
    return {
        feed_id: 1,
        status: 'ok',
        http_status: 200,
        new_articles: 1,
        updated_articles: 0,
        error: null,
        ...overrides,
    };
}

export function fetchRecordFixture(overrides: Partial<FetchRecordDto> = {}): FetchRecordDto {
    return {
        feed_id: 1,
        fetched_at: '2026-07-01T10:00:00.000Z',
        status: 'ok',
        http_status: 200,
        error: null,
        articles_new: 1,
        duration_ms: 120,
        ...overrides,
    };
}

export function destinationFixture(overrides: Partial<DestinationDto> = {}): DestinationDto {
    return {
        name: 'notes',
        root: '/home/user/notes',
        ...overrides,
    };
}

export function saveOutcomeFixture(overrides: Partial<SaveOutcomeDto> = {}): SaveOutcomeDto {
    return {
        destination: 'notes',
        path: 'curio/an-article.md',
        checksum: `sha256:${'0'.repeat(64)}`,
        disposition: 'created',
        ...overrides,
    };
}

export function opmlImportOutcomeFixture(
    overrides: Partial<OpmlImportOutcomeDto> = {},
): OpmlImportOutcomeDto {
    return {
        added: 2,
        skipped: 1,
        ...overrides,
    };
}

export function importOutcomeFixture(overrides: Partial<ImportOutcomeDto> = {}): ImportOutcomeDto {
    return {
        feeds_added: 2,
        feeds_skipped: 1,
        articles_added: 0,
        articles_skipped: 0,
        ...overrides,
    };
}

export function appInfoFixture(overrides: Partial<AppInfoDto> = {}): AppInfoDto {
    return {
        version: '0.1.0',
        profile_dir: '/home/user/.local/share/curio',
        db_schema_version: 1,
        schema_supported: 1,
        ...overrides,
    };
}

export function dbStatsFixture(overrides: Partial<DbStatsDto> = {}): DbStatsDto {
    return {
        article_count: 42,
        schema_version: 1,
        db_path: '/home/user/.local/share/curio/curio.db',
        ...overrides,
    };
}

export function pathTokenFixture(overrides: Partial<PathTokenDto> = {}): PathTokenDto {
    return {
        token: '01JZ0000000000000000000000',
        path: '/home/user/Downloads/subs.opml',
        ...overrides,
    };
}

export function discoveryFixture(overrides: Partial<DiscoveryDto> = {}): DiscoveryDto {
    return {
        feeds: [{ url: 'https://example.org/feed.xml', title: 'Example Feed' }],
        favicon: 'https://example.org/favicon.ico',
        ...overrides,
    };
}
