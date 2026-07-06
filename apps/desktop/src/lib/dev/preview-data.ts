/**
 * DEV-ONLY preview dataset. Powers the in-browser mock (install-mock.ts) so
 * `npm run dev` renders a fully-populated, realistic Curio without the Tauri
 * backend — the substrate for visual design review. Never imported by the
 * production bundle (the installer dynamic-imports it behind `import.meta.env.DEV`).
 */
import type {
    ArticleDto,
    ArticleStateDto,
    ArticleSummaryDto,
    DestinationDto,
    FeedDto,
} from '$lib/bindings';

export interface PreviewArticle {
    summary: ArticleSummaryDto;
    full: ArticleDto;
    state: ArticleStateDto;
    tags: string[];
}

const gradientPoster = (a: string, b: string): string =>
    `data:image/svg+xml;utf8,${encodeURIComponent(
        `<svg xmlns='http://www.w3.org/2000/svg' width='1200' height='630'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${a}'/><stop offset='1' stop-color='${b}'/></linearGradient></defs><rect width='1200' height='630' fill='url(#g)'/></svg>`,
    )}`;

export const previewFeeds: FeedDto[] = [
    {
        id: 1,
        url: 'https://overreacted.io/rss.xml',
        title: 'Overreacted',
        site_url: 'https://overreacted.io',
        description: "Dan Abramov's blog on React and the web",
        status: 'active',
        added_at: '2026-05-02T10:00:00.000Z',
        last_fetched_at: '2026-07-06T08:12:00.000Z',
        tags: ['Engineering'],
    },
    {
        id: 2,
        url: 'https://www.robinrendle.com/feed.xml',
        title: 'Robin Rendle',
        site_url: 'https://robinrendle.com',
        description: 'Essays on design, type, and the web',
        status: 'active',
        added_at: '2026-05-10T10:00:00.000Z',
        last_fetched_at: '2026-07-06T07:40:00.000Z',
        tags: ['Design'],
    },
    {
        id: 3,
        url: 'https://reddit.com/r/typography/.rss',
        title: 'r/typography',
        site_url: 'https://reddit.com/r/typography',
        description: 'Everything about type',
        status: 'active',
        added_at: '2026-06-01T10:00:00.000Z',
        last_fetched_at: '2026-07-06T09:01:00.000Z',
        tags: ['Design', 'Community'],
    },
    {
        id: 4,
        url: 'https://youtube.com/@Fireship/videos.rss',
        title: 'Fireship',
        site_url: 'https://youtube.com/@Fireship',
        description: 'High-intensity code tutorials',
        status: 'active',
        added_at: '2026-06-03T10:00:00.000Z',
        last_fetched_at: '2026-07-06T06:22:00.000Z',
        tags: ['Video'],
    },
    {
        id: 5,
        url: 'https://stratechery.com/feed/',
        title: 'Stratechery',
        site_url: 'https://stratechery.com',
        description: 'Ben Thompson on tech and strategy',
        status: 'active',
        added_at: '2026-04-18T10:00:00.000Z',
        last_fetched_at: '2026-07-06T05:55:00.000Z',
        tags: ['Business'],
    },
    {
        id: 6,
        url: 'https://www.craigmod.com/index.xml',
        title: 'Craig Mod',
        site_url: 'https://craigmod.com',
        description: 'Walking, books, and software',
        status: 'active',
        added_at: '2026-04-22T10:00:00.000Z',
        last_fetched_at: '2026-07-06T04:30:00.000Z',
        tags: ['Essays'],
    },
    {
        id: 7,
        url: 'https://newsletter.pragmaticengineer.com/feed',
        title: 'The Pragmatic Engineer',
        site_url: 'https://pragmaticengineer.com',
        description: 'Big Tech and startups, from the inside',
        status: 'paused',
        added_at: '2026-03-30T10:00:00.000Z',
        last_fetched_at: '2026-07-05T22:10:00.000Z',
        tags: ['Engineering'],
    },
];

function mk(
    id: number,
    feedId: number,
    title: string,
    author: string | null,
    published: string,
    words: number,
    html: string,
    state: Partial<ArticleStateDto> = {},
    tags: string[] = [],
    sourceUrl = 'https://example.com/post',
): PreviewArticle {
    const summary: ArticleSummaryDto = {
        id,
        feed_id: feedId,
        title,
        source_url: sourceUrl,
        author,
        published_at: published,
        saved_at: published,
        word_count: words,
        lang: 'en',
    };
    return {
        summary,
        full: {
            ...summary,
            curio_id: `019780f0-0000-7000-8000-${String(id).padStart(12, '0')}`,
            content_html: html,
            content_text: title,
            source_updated_at: null,
        },
        state: { read: false, starred: false, read_later: false, archived: false, ...state },
        tags,
    };
}

export const previewArticles: PreviewArticle[] = [
    mk(
        101,
        1,
        'The Two Reacts',
        'Dan Abramov',
        '2026-07-06T08:00:00.000Z',
        2400,
        `<figure><img src="${gradientPoster('#4f46e5', '#0ea5e9')}" alt="Abstract gradient"/><figcaption>Two mental models, one library.</figcaption></figure>
<p>React has always had a <strong>dual nature</strong>. There is the React that runs on the server, assembling markup before a single byte of JavaScript reaches the browser — and the React that hydrates in the client, wiring up event handlers and reconciling state.</p>
<h2>A tale of two runtimes</h2>
<p>For years we pretended these were the same thing. They are not. The server runtime is a <em>template engine with superpowers</em>; the client runtime is an <em>interactive state machine</em>. Conflating them is the source of most confusion about Server Components.</p>
<blockquote>The mistake is thinking of the network as an implementation detail. It is the <strong>defining constraint</strong>.</blockquote>
<p>Consider what actually crosses the wire. In the server model, you ship a description of the UI. In the client model, you ship the machinery to <em>produce</em> that description. The art is deciding, per component, which React you want.</p>
<h3>What this means in practice</h3>
<ul><li>Data-fetching lives where the data lives.</li><li>Interactivity is opt-in, not ambient.</li><li>The bundle is a budget, not an afterthought.</li></ul>
<pre><code>export default async function Page() {
  const posts = await db.posts.findMany();
  return &lt;List items={posts} /&gt;;
}</code></pre>
<p>None of this is new advice. What is new is that the framework finally makes the good path the <em>default</em> path.</p>`,
        { starred: true },
        ['react', 'architecture'],
    ),
    mk(
        102,
        5,
        'Aggregation and the Open Web',
        'Ben Thompson',
        '2026-07-06T06:00:00.000Z',
        3100,
        `<p>The open web was supposed to be a level playing field. Instead it became the <strong>substrate on which aggregators built moats</strong>. This is not a contradiction — it is the predictable outcome of zero distribution costs meeting infinite supply.</p>
<h2>Supply, demand, and the missing middle</h2>
<p>When distribution is free, the scarce resource is <em>attention</em>. Whoever owns the demand side owns the market. Publishers, who own supply, are structurally disadvantaged — a fact RSS readers quietly resist.</p>
<p>There is a version of the web where you own your inputs. It looks a lot like a good feed reader.</p>`,
        {},
        ['strategy', 'web'],
    ),
    mk(
        103,
        2,
        'On Reading Type at Length',
        'Robin Rendle',
        '2026-07-05T18:00:00.000Z',
        1800,
        `<p>Good typography disappears. You notice it only in its <em>absence</em> — the line that is a touch too long, the leading that pinches, the measure that makes your eyes hunt for the next row.</p>
<h2>The measure is the message</h2>
<p>Sixty-six characters. That is the number the typographers keep returning to, and they are not wrong. A comfortable measure is the difference between reading and <strong>skimming</strong>.</p>
<blockquote>Set the text first. Everything else is decoration around a column of words.</blockquote>
<p>A reader is, at bottom, a machine for setting text well. If it does that, the rest is polish. If it does not, no amount of polish will save it.</p>`,
        { read_later: true },
        ['typography', 'design'],
    ),
    mk(
        104,
        3,
        'What typeface is this? Spotted on a shopfront in Lisbon',
        'u/serif_lover',
        '2026-07-06T09:00:00.000Z',
        60,
        `<p>Saw this gorgeous signage in Alfama yesterday — high contrast, tall ascenders, a very distinctive ampersand. WhatTheFont came up empty. Any ideas? Reminds me of a Didone but the serifs are chunkier.</p><p>—</p><p><strong>Top comment (u/type_nerd, 340 points):</strong> That's almost certainly a custom cut, but the closest commercial match is Canela. The ampersand gives it away.</p>`,
        {},
        ['identification'],
        'https://reddit.com/r/typography/comments/abc123',
    ),
    mk(
        105,
        4,
        'Rust in 100 Seconds',
        'Fireship',
        '2026-07-06T06:00:00.000Z',
        120,
        `<p>Rust is a memory-safe systems programming language with no garbage collector. In this video we cover ownership, borrowing, and why everyone is rewriting their tools in it.</p>`,
        {},
        ['rust', 'video'],
        'https://youtube.com/watch?v=5C_HPTJg5ek',
    ),
    mk(
        106,
        6,
        'Ridgeline: On Finishing',
        'Craig Mod',
        '2026-07-05T12:00:00.000Z',
        2200,
        `<figure><img src="${gradientPoster('#f59e0b', '#ef4444')}" alt="Warm gradient"/><figcaption>A morning on the Nakasendo.</figcaption></figure>
<p>I have been thinking about <em>finishing</em> — the specific, physical satisfaction of a thing brought to completion. Not shipped. Not launched. <strong>Finished.</strong></p>
<h2>The last ten percent</h2>
<p>Everyone knows the last ten percent is the hardest. Fewer people talk about why: it is the part with no dopamine. The idea gave you a rush. The middle had momentum. The end is just <em>care</em>, applied slowly, to details no one will consciously notice.</p>
<blockquote>The reader will never see the work you did to make the seams invisible. That is the work.</blockquote>
<p>And yet the finished thing feels different in the hand. You cannot fake it. The care is legible even when the details are not.</p>`,
        {},
        ['essays', 'craft'],
    ),
    mk(
        107,
        1,
        'Before You memo() Anything',
        'Dan Abramov',
        '2026-07-04T10:00:00.000Z',
        1600,
        `<p>The instinct to reach for <code>useMemo</code> is almost always premature. Before you memoize, ask a better question: <strong>why is this re-rendering at all?</strong></p>
<p>Most performance problems are structure problems wearing a performance costume.</p>`,
        { read: true },
        ['react', 'performance'],
    ),
    mk(
        108,
        5,
        'The End of the Beginning',
        'Ben Thompson',
        '2026-07-03T06:00:00.000Z',
        2800,
        `<p>We are past the era of platform shifts and into the era of <em>platform maturity</em>. The interesting question is no longer "what is the next device" but "what do we build now that the substrate has stopped moving?"</p>`,
        { read: true },
        ['strategy'],
    ),
    mk(
        109,
        2,
        'A Grid You Can Feel',
        'Robin Rendle',
        '2026-07-02T14:00:00.000Z',
        1400,
        `<p>Grids are not cages. A good grid is a <strong>rhythm section</strong> — it keeps time so the melody can wander. The best layouts break the grid on purpose, and you feel the break precisely because the grid was there.</p>`,
        {},
        ['layout', 'css'],
    ),
    mk(
        110,
        6,
        'Kissa by Kissa',
        'Craig Mod',
        '2026-06-30T09:00:00.000Z',
        900,
        `<p>Two hundred kilometers on foot, one pizza toast at a time. A note on why the small, repeated ritual outlasts the grand gesture.</p>`,
        { starred: true },
        ['walking', 'japan'],
    ),
];

export const previewDestinations: DestinationDto[] = [
    { name: 'Obsidian Vault', root: '/Users/alex/Documents/vault' },
    { name: 'Reading Notes', root: '/Users/alex/notes/reading' },
];
