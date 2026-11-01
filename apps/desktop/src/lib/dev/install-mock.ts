/**
 * DEV-ONLY in-browser IPC mock. When `npm run dev` runs the app in a plain
 * browser (no Tauri webview), this installs `@tauri-apps/api/mocks` with a
 * stateful responder over the preview dataset so the whole UI renders and
 * reacts — the substrate for visual design review. Guarded by
 * `import.meta.env.DEV` + a no-real-Tauri check and dynamic-imported, so it is
 * absent from the production bundle and never runs inside the packaged app.
 */
import type { ArticleStateDto, StartupFailureClass, StartupFailureDto } from '$lib/bindings';

let installed = false;

const DEV_PROFILE = '/Users/alex/Library/Application Support/io.github.alexnodeland.curio';

const DEV_STARTUP_MESSAGES: Record<StartupFailureClass, string> = {
    corrupt: `${DEV_PROFILE}/curio.db is not a readable Curio database (corrupt or not SQLite): file is not a database`,
    too_new:
        'database schema version 8 is newer than this build supports (7) — update Curio to open it',
    locked: `${DEV_PROFILE}/curio.db is locked by another process — close other copies of Curio and retry`,
    io: `cannot open ${DEV_PROFILE}/curio.db: unable to open database file`,
    profile: `profile io at ${DEV_PROFILE}: Not a directory (os error 20)`,
    network_fs: `${DEV_PROFILE}/curio.db is on iCloud Drive, where the database can be corrupted (SQLite's locking is unsafe there and sync clients copy the file mid-write) — move the profile to a local disk, or set CURIO_ALLOW_NETWORK_FS=1 to open it anyway`,
    upgrade:
        'migrating the database from schema v6 to v7 failed and was rolled back: sqlite error: disk I/O error',
    no_profile: 'the platform reports no home directory — cannot resolve profile/cache paths',
    other: 'could not enable WAL journal mode (got "delete")',
};

/** The startup failure `?startup=<class>` asks the dev mock to report. */
function devStartupFailure(): StartupFailureDto | null {
    const params = new URLSearchParams(window.location.search);
    const requested = params.get('startup') as StartupFailureClass | null;
    if (requested === null || !(requested in DEV_STARTUP_MESSAGES)) {
        return null;
    }
    const hasProfile = requested !== 'no_profile';
    const hasBackup =
        hasProfile &&
        requested !== 'locked' &&
        requested !== 'profile' &&
        requested !== 'network_fs' &&
        params.get('backup') !== '0';
    return {
        class: requested,
        message: DEV_STARTUP_MESSAGES[requested],
        profile_dir: hasProfile ? DEV_PROFILE : null,
        backup: hasBackup ? `${DEV_PROFILE}/curio.db.pre-v7.bak` : null,
        app_version: '0.5.0',
    };
}

/**
 * `?legacy=<status>` (optionally `&inPlace=1`) previews a startup that met a
 * library from an earlier release — the notice and the doctor panel.
 */
function devLegacyProfiles(): unknown[] {
    const query = new URLSearchParams(window.location.search);
    const status = query.get('legacy');
    if (status === null) {
        return [];
    }
    return [
        {
            path: '~/Library/Application Support/curio',
            default_dir: '~/Library/Application Support/io.github.alexnodeland.curio',
            status,
            detail:
                status === 'move_failed' || status === 'unreadable'
                    ? 'Cross-device link (os error 18)'
                    : null,
            opened_in_place: query.get('inPlace') === '1',
        },
    ];
}

export async function installDevMock(): Promise<void> {
    if (installed) return;
    if (!import.meta.env.DEV) return;
    if (typeof window === 'undefined') return;
    // Real Tauri sets this before our code runs; never shadow the real IPC.
    if ('__TAURI_INTERNALS__' in window && !('__CURIO_DEV_MOCK__' in window)) return;
    installed = true;
    (window as unknown as Record<string, boolean>).__CURIO_DEV_MOCK__ = true;

    const { mockIPC } = await import('@tauri-apps/api/mocks');
    const { previewFeeds, previewArticles, previewDestinations } = await import('./preview-data');

    const feeds = structuredClone(previewFeeds);
    const articles = structuredClone(previewArticles);
    const destinations = structuredClone(previewDestinations);
    const settings = new Map<string, string>([
        ['ui.theme', 'dark'],
        ['ui.theme.mode', 'dark'],
    ]);

    // A canned custom theme the dev mock "reads" on import, so the theme
    // import flow can be exercised in the plain-browser visual loop.
    const DEV_THEME_YAML = `schema: curio.theme/v1
id: ocean
name: Ocean
appearance: dark
tokens:
  bg: "#0b1622"
  bg-secondary: "#0f1d2e"
  bg-tertiary: "#13243a"
  bg-hover: "#1b3350"
  fg: "#cfe3f2"
  fg-muted: "#7fa9c9"
  fg-subtle: "#5a7f9c"
  accent: "#38bdf8"
  accent-hover: "#0ea5e9"
  accent-fg: "#001018"
  accent-muted: "rgba(56, 189, 248, 0.15)"
  border: "#1b3350"
  border-subtle: "#13243a"
  error: "#fb7185"
  error-bg: "rgba(251, 113, 133, 0.15)"
  warning: "#fbbf24"
  success: "#34d399"
  unread: "#38bdf8"
  read: "#7fa9c9"
  link: "#7dd3fc"
  link-visited: "#c4b5fd"
`;

    const byId = (id: number) => articles.find((a) => a.summary.id === id);
    const unreadCounts = () => {
        const by = new Map<number, number>();
        for (const a of articles) {
            if (a.summary.feed_id != null && !a.state.read) {
                by.set(a.summary.feed_id, (by.get(a.summary.feed_id) ?? 0) + 1);
            }
        }
        const total = [...by.values()].reduce((n, v) => n + v, 0);
        return { total, by_feed: [...by.entries()] };
    };

    mockIPC(
        // biome-ignore lint/complexity/noExcessiveCognitiveComplexity: a flat 1:1 IPC dispatch table for the dev mock; splitting it would obscure the mapping
        (cmd, raw) => {
            const args = (raw ?? {}) as Record<string, unknown>;
            switch (cmd) {
                case 'list_feeds':
                    return structuredClone(feeds);
                case 'get_unread_counts':
                    return unreadCounts();
                case 'count_articles':
                    return articles.length;
                case 'list_tags':
                    return [...new Set(articles.flatMap((a) => a.tags))].sort();
                case 'list_destinations':
                    return structuredClone(destinations);
                case 'list_articles': {
                    const p = (args.params ?? {}) as Record<string, unknown>;
                    let rows = articles
                        .filter((a) => (p.feed_id == null ? true : a.summary.feed_id === p.feed_id))
                        .filter((a) => (p.read == null ? true : a.state.read === p.read))
                        .filter((a) => (p.starred == null ? true : a.state.starred === p.starred))
                        .filter((a) =>
                            p.read_later == null ? true : a.state.read_later === p.read_later,
                        )
                        .filter((a) =>
                            p.archived == null ? true : a.state.archived === p.archived,
                        )
                        .filter((a) => (p.tag == null ? true : a.tags.includes(p.tag as string)))
                        .sort((x, y) => y.summary.id - x.summary.id);
                    if (p.before != null)
                        rows = rows.filter((a) => a.summary.id < (p.before as number));
                    return rows
                        .slice(0, (p.limit as number) ?? 50)
                        .map((a) => structuredClone(a.summary));
                }
                case 'search_articles': {
                    const q = String(args.query ?? '').toLowerCase();
                    return articles
                        .filter((a) => a.summary.title.toLowerCase().includes(q))
                        .slice(0, (args.limit as number) ?? 20)
                        .map((a) => structuredClone(a.summary));
                }
                case 'get_article':
                    return structuredClone(byId(args.articleId as number)?.full ?? null);
                case 'get_article_state':
                    return structuredClone(byId(args.articleId as number)?.state ?? null);
                case 'get_article_tags':
                    return structuredClone(byId(args.articleId as number)?.tags ?? []);
                case 'mark_read':
                    return flip(byId(args.articleId as number), 'read', args.read as boolean);
                case 'set_starred':
                    return flip(byId(args.articleId as number), 'starred', args.starred as boolean);
                case 'set_read_later':
                    return flip(
                        byId(args.articleId as number),
                        'read_later',
                        args.readLater as boolean,
                    );
                case 'set_archived':
                    return flip(
                        byId(args.articleId as number),
                        'archived',
                        args.archived as boolean,
                    );
                case 'record_opened':
                    return null;
                case 'tag_article': {
                    const a = byId(args.articleId as number);
                    const tag = args.tag as string;
                    if (a && !a.tags.includes(tag)) {
                        a.tags.push(tag);
                        return true;
                    }
                    return false;
                }
                case 'untag_article': {
                    const a = byId(args.articleId as number);
                    const before = a?.tags.length ?? 0;
                    if (a) a.tags = a.tags.filter((t) => t !== args.tag);
                    return (a?.tags.length ?? 0) !== before;
                }
                case 'promote_article':
                    return {
                        destination: String(args.destination),
                        path: 'curio/promoted-note.md',
                        checksum: `sha256:${'0'.repeat(64)}`,
                        disposition: 'created',
                    };
                case 'get_setting':
                    return settings.get(args.key as string) ?? null;
                case 'set_setting':
                    settings.set(args.key as string, args.value as string);
                    return null;
                case 'request_notification_permission':
                    return true;
                case 'get_feed_by_url':
                    return structuredClone(feeds.find((f) => f.url === args.url) ?? null);
                case 'set_feed_status': {
                    const f = feeds.find((x) => x.id === args.feedId);
                    if (f) f.status = args.status as typeof f.status;
                    return null;
                }
                case 'set_feed_title': {
                    const f = feeds.find((x) => x.id === args.feedId);
                    if (f) f.title = (args.title as string | null) ?? null;
                    return null;
                }
                case 'set_feed_tags': {
                    const f = feeds.find((x) => x.id === args.feedId);
                    if (f) f.tags = (args.tags as string[] | undefined) ?? [];
                    return null;
                }
                case 'set_feed_metadata': {
                    const f = feeds.find((x) => x.id === args.feedId);
                    if (f) {
                        f.site_url = (args.siteUrl as string | null) ?? null;
                        f.description = (args.description as string | null) ?? null;
                    }
                    return null;
                }
                case 'recent_fetches':
                    return [
                        {
                            feed_id: args.feedId as number,
                            fetched_at: '2026-07-06T09:00:00.000Z',
                            status: 'ok',
                            http_status: 200,
                            error: null,
                            error_kind: null,
                            articles_new: 2,
                            duration_ms: 143,
                        },
                        {
                            feed_id: args.feedId as number,
                            fetched_at: '2026-07-06T06:00:00.000Z',
                            status: 'ok',
                            http_status: 200,
                            error: null,
                            error_kind: null,
                            articles_new: 0,
                            duration_ms: 98,
                        },
                        {
                            feed_id: args.feedId as number,
                            fetched_at: '2026-07-05T22:00:00.000Z',
                            status: 'not_modified',
                            http_status: 304,
                            error: null,
                            error_kind: null,
                            articles_new: 0,
                            duration_ms: 61,
                        },
                    ];
                case 'refresh_feed':
                    return {
                        feed_id: args.feedId as number,
                        status: 'ok',
                        http_status: 200,
                        new_articles: 0,
                        updated_articles: 0,
                        error: null,
                        error_kind: null,
                    };
                case 'refresh_all':
                    return feeds.map((f) => ({
                        feed_id: f.id,
                        status: 'ok',
                        http_status: 200,
                        new_articles: 0,
                        updated_articles: 0,
                        error: null,
                        error_kind: null,
                    }));
                // `?offline=1` previews the sidebar's "Offline" badge.
                case 'connectivity_online':
                    return new URLSearchParams(window.location.search).get('offline') !== '1';
                case 'get_app_info':
                    return {
                        version: '0.5.0',
                        profile_dir: '~/Library/Application Support/io.github.alexnodeland.curio',
                        db_schema_version: 5,
                        schema_supported: true,
                    };
                case 'legacy_profiles':
                    return devLegacyProfiles();
                case 'db_stats':
                    return {
                        article_count: articles.length,
                        schema_version: 5,
                        db_path:
                            '~/Library/Application Support/io.github.alexnodeland.curio/curio.db',
                    };
                case 'integrity_check':
                case 'fts_integrity_check':
                case 'vacuum_db':
                    return null;
                case 'sweep_event_retention':
                    return [];
                case 'add_destination':
                    destinations.push({ name: String(args.name), root: '/Users/alex/picked' });
                    return null;
                case 'remove_destination': {
                    const i = destinations.findIndex((d) => d.name === args.name);
                    if (i >= 0) destinations.splice(i, 1);
                    return null;
                }
                case 'pick_import_file':
                case 'pick_export_path':
                case 'pick_theme_export_path':
                case 'pick_theme_import_file':
                case 'pick_destination_root':
                    return { token: 'dev-token', path: '/Users/alex/picked/path' };
                case 'read_text_file':
                    return DEV_THEME_YAML;
                case 'export_text_file':
                    return null;
                case 'import_opml':
                    return { added: 4, skipped: 1, invalid: 0 };
                case 'import_file':
                    // A CSV source plays the messy-export path (rows with
                    // unusable URLs are counted, not fatal); OPML stays clean.
                    if (args.source !== 'opml') {
                        return {
                            feeds_added: 0,
                            feeds_skipped: 0,
                            feeds_invalid: 0,
                            articles_added: 12,
                            articles_skipped: 3,
                            articles_invalid: 2,
                            rejected: [
                                {
                                    kind: 'article',
                                    reason: 'unsupported_scheme',
                                    detail: 'javascript:void(0)',
                                },
                                { kind: 'article', reason: 'invalid_url', detail: 'n/a' },
                            ],
                        };
                    }
                    return {
                        feeds_added: 4,
                        feeds_skipped: 1,
                        feeds_invalid: 0,
                        articles_added: 12,
                        articles_skipped: 0,
                        articles_invalid: 0,
                        rejected: [],
                    };
                case 'export_opml':
                    return null;
                case 'discover_feeds': {
                    const query = String(args.url);
                    // A canned stand-in for `curio_core::feeds::discover` (no
                    // normalization, rewrites, probing or validation): a
                    // YouTube @handle resolves to its channel videos feed,
                    // anything else to itself.
                    if (/nofeed/i.test(query)) {
                        return { feeds: [], favicon: null };
                    }
                    if (/youtube\.com\/@/.test(query)) {
                        return {
                            feeds: [
                                {
                                    url: 'https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw',
                                    title: 'YouTube Channel',
                                },
                            ],
                            favicon: null,
                        };
                    }
                    return {
                        feeds: [{ url: query, title: 'Discovered Feed' }],
                        favicon: null,
                    };
                }
                case 'get_cached_image': {
                    // `.invalid` hosts (RFC 2606) stand in for an unreachable
                    // image, so the reader's failed-image path is previewable.
                    const url = String(args.url ?? '');
                    if (/^https?:\/\/[^/]*\.invalid\//i.test(url)) {
                        throw {
                            kind: 'user',
                            code: 'NETWORK',
                            message: `could not fetch ${url}`,
                            recoverable: true,
                        };
                    }
                    return url;
                }
                case 'copy_diagnostics_bundle':
                    return { token: 'dev-token', path: '/Users/alex/curio-diagnostics.zip' };
                case 'add_feed': {
                    // A canned stand-in for subscribe-time validation: a URL
                    // mentioning `nofeed` has no feed, one mentioning
                    // `offline` is unreachable — unless `validate: false`
                    // (the "use this URL directly" escape hatch).
                    const newFeed = args.newFeed as { url: string; validate: boolean };
                    if (newFeed.validate && /nofeed/i.test(newFeed.url)) {
                        throw {
                            kind: 'user',
                            code: 'NO_FEED',
                            message: `no feed found at ${newFeed.url}`,
                            recoverable: false,
                        };
                    }
                    if (newFeed.validate && /offline/i.test(newFeed.url)) {
                        throw {
                            kind: 'user',
                            code: 'NETWORK',
                            message: `could not reach ${newFeed.url}: connection refused`,
                            recoverable: true,
                        };
                    }
                    return structuredClone(feeds[0]);
                }
                case 'remove_feed':
                case 'update_feed_metadata':
                    return null;
                // Startup recovery: `?startup=<class>` (and `&backup=0` for
                // no backup) previews the recovery screen. A retry still
                // fails; restore / start fresh succeed.
                case 'get_startup_failure':
                    return devStartupFailure();
                case 'retry_startup':
                    return { failure: devStartupFailure(), set_aside: null };
                case 'restore_startup_backup':
                case 'start_fresh_profile':
                    return {
                        failure: null,
                        set_aside: `${DEV_PROFILE}/set-aside/20261031T120000Z`,
                    };
                default:
                    return null;
            }
        },
        { shouldMockEvents: true },
    );

    function flip(
        a: { state: ArticleStateDto } | undefined,
        key: keyof ArticleStateDto,
        value: boolean,
    ): boolean {
        if (!a) return false;
        const changed = a.state[key] !== value;
        a.state[key] = value;
        return changed;
    }
}
