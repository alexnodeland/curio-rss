/**
 * DEV-ONLY in-browser IPC mock. When `npm run dev` runs the app in a plain
 * browser (no Tauri webview), this installs `@tauri-apps/api/mocks` with a
 * stateful responder over the preview dataset so the whole UI renders and
 * reacts — the substrate for visual design review. Guarded by
 * `import.meta.env.DEV` + a no-real-Tauri check and dynamic-imported, so it is
 * absent from the production bundle and never runs inside the packaged app.
 */
import type { ArticleStateDto } from '$lib/bindings';

let installed = false;

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
                case 'get_feed_by_url':
                    return structuredClone(feeds.find((f) => f.url === args.url) ?? null);
                case 'set_feed_status': {
                    const f = feeds.find((x) => x.id === args.feedId);
                    if (f) f.status = args.status as typeof f.status;
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
                            articles_new: 2,
                            duration_ms: 143,
                        },
                        {
                            feed_id: args.feedId as number,
                            fetched_at: '2026-07-06T06:00:00.000Z',
                            status: 'ok',
                            http_status: 200,
                            error: null,
                            articles_new: 0,
                            duration_ms: 98,
                        },
                        {
                            feed_id: args.feedId as number,
                            fetched_at: '2026-07-05T22:00:00.000Z',
                            status: 'not_modified',
                            http_status: 304,
                            error: null,
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
                    };
                case 'refresh_all':
                    return feeds.map((f) => ({
                        feed_id: f.id,
                        status: 'ok',
                        http_status: 200,
                        new_articles: 0,
                        updated_articles: 0,
                        error: null,
                    }));
                case 'get_app_info':
                    return {
                        version: '0.1.0',
                        profile_dir: '~/Library/Application Support/io.github.alexnodeland.curio',
                        db_schema_version: 3,
                        schema_supported: true,
                    };
                case 'db_stats':
                    return {
                        article_count: articles.length,
                        schema_version: 3,
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
                case 'pick_destination_root':
                    return { token: 'dev-token', path: '/Users/alex/picked/path' };
                case 'import_opml':
                    return { added: 4, skipped: 1 };
                case 'export_opml':
                    return null;
                case 'discover_feeds':
                    return {
                        feeds: [{ url: String(args.url), title: 'Discovered Feed', kind: 'rss' }],
                        favicon: null,
                    };
                case 'get_cached_image':
                    return String(args.url ?? '');
                case 'copy_diagnostics_bundle':
                    return { token: 'dev-token', path: '/Users/alex/curio-diagnostics.zip' };
                case 'add_feed':
                    return structuredClone(feeds[0]);
                case 'remove_feed':
                case 'update_feed_metadata':
                    return null;
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
