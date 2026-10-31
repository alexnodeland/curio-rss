// The smoke suite's network: one in-process HTTP server on 127.0.0.1 (never
// the real network). It serves two feeds built from the repo's hermetic
// fixtures and counts every request, so a scenario can prove what the app
// did — and did NOT — fetch.
//
//   /seed.xml   three items, one carrying fixtures/html/xss-corpus.html. The
//               harness subscribes to it through the CLI with the explicit
//               W1 opt-in (`--allow-private-network`), so it is fetchable.
//   /added.xml  fixtures/feeds/rss2.xml verbatim — the URL the add-feed
//               scenario types into the UI. Nothing in the UI can exempt a
//               feed from the SSRF guard, so the app must never fetch it.
//   /__hits     the request counts as JSON ({ "/seed.xml": 1, … }).
import { readFileSync } from 'node:fs';
import { type Server, createServer } from 'node:http';
import type { AddressInfo } from 'node:net';
import { resolve } from 'node:path';

/** Titles of the three seeded items. */
export const SEEDED_TITLES = ['Hostile fixture', 'Second seeded post', 'Third seeded post'];

/** Path of the feed the add-feed scenario subscribes to through the UI. */
export const ADDED_FEED_PATH = '/added.xml';

/** Path of the feed the harness seeds through the CLI. */
export const SEED_FEED_PATH = '/seed.xml';

/** Path of the request-count endpoint. */
export const HITS_PATH = '/__hits';

function item(base: string, title: string, slug: string, date: string, html: string): string {
    return `    <item>
      <title>${title}</title>
      <link>${base}/posts/${slug}</link>
      <guid isPermaLink="false">urn:curio-e2e:${slug}</guid>
      <pubDate>${date}</pubDate>
      <content:encoded><![CDATA[${html}]]></content:encoded>
    </item>`;
}

/** The seed feed: every link points back at the stub, so nothing derived
 *  from it (site favicon, article links) can reach a public host. */
function seedFeed(base: string, fixtures: string): string {
    const corpus = readFileSync(resolve(fixtures, 'html/xss-corpus.html'), 'utf8');
    if (corpus.includes(']]>')) {
        throw new Error('xss-corpus.html contains a CDATA terminator; it cannot be embedded');
    }
    const items = [
        item(base, SEEDED_TITLES[0], 'hostile', 'Wed, 01 Jul 2026 12:00:00 GMT', corpus),
        item(
            base,
            SEEDED_TITLES[1],
            'second',
            'Tue, 30 Jun 2026 12:00:00 GMT',
            '<p>The second seeded post.</p>',
        ),
        item(
            base,
            SEEDED_TITLES[2],
            'third',
            'Mon, 29 Jun 2026 12:00:00 GMT',
            '<p>The third seeded post.</p>',
        ),
    ];
    return `<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:content="http://purl.org/rss/1.0/modules/content/">
  <channel>
    <title>Curio e2e seed</title>
    <link>${base}/</link>
    <description>Seeded by the desktop smoke harness.</description>
${items.join('\n')}
  </channel>
</rss>
`;
}

/** A running stub: its base URL and a way to stop it. */
export interface FeedStub {
    readonly base: string;
    close(): Promise<void>;
}

/** Starts the stub on an ephemeral 127.0.0.1 port. */
export async function startFeedStub(fixtures: string): Promise<FeedStub> {
    const hits = new Map<string, number>();
    const added = readFileSync(resolve(fixtures, 'feeds/rss2.xml'), 'utf8');
    let base = '';
    const server: Server = createServer((request, response) => {
        const path = new URL(request.url ?? '/', base).pathname;
        if (path === HITS_PATH) {
            response.writeHead(200, { 'content-type': 'application/json' });
            response.end(JSON.stringify(Object.fromEntries(hits)));
            return;
        }
        hits.set(path, (hits.get(path) ?? 0) + 1);
        const rss = { 'content-type': 'application/rss+xml; charset=utf-8' };
        if (path === SEED_FEED_PATH) {
            response.writeHead(200, rss);
            response.end(seedFeed(base, fixtures));
        } else if (path === ADDED_FEED_PATH) {
            response.writeHead(200, rss);
            response.end(added);
        } else {
            response.writeHead(404);
            response.end();
        }
    });
    await new Promise<void>((done, fail) => {
        server.once('error', fail);
        server.listen(0, '127.0.0.1', () => done());
    });
    base = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
    return {
        base,
        close: () => new Promise<void>((done) => server.close(() => done())),
    };
}

/** Reads the stub's request counts (from a spec worker, over HTTP). */
export async function stubHits(base: string): Promise<Record<string, number>> {
    const response = await fetch(`${base}${HITS_PATH}`);
    return (await response.json()) as Record<string, number>;
}
