/**
 * Reader view-mode heuristics — pure, data-only. Given a feed URL and an
 * article's source URL (both RSS-native fields, never enrichment), decide
 * which reader layout renders it. The hard rule from the design corpus
 * (D8): layouts sit over RSS-native data ONLY — no Reddit JSON API, no
 * yt-dlp, ever. These helpers extract nothing the feed did not already give
 * us (a video id from the watch URL, a subreddit from the path).
 */

/** The layouts the reader dispatches between. */
export type ViewMode = 'article' | 'reddit' | 'youtube';

/** Lowercased registrable host, `www.` stripped; `null` if unparseable. */
function hostOf(url: string | null): string | null {
    if (url === null) {
        return null;
    }
    try {
        return new URL(url).hostname.toLowerCase().replace(/^www\./, '');
    } catch {
        return null;
    }
}

function isYouTubeHost(host: string | null): boolean {
    if (host === null) {
        return false;
    }
    return (
        host === 'youtube.com' ||
        host === 'youtu.be' ||
        host === 'youtube-nocookie.com' ||
        host.endsWith('.youtube.com')
    );
}

function isRedditHost(host: string | null): boolean {
    if (host === null) {
        return false;
    }
    return host === 'reddit.com' || host.endsWith('.reddit.com');
}

/**
 * The reader layout for one article. YouTube wins over Reddit (a crosspost
 * of a video is still a video); anything unmatched is the default article
 * layout. Either the feed URL or the source URL matching is enough — a
 * YouTube channel feed and a bare `youtu.be` share link both resolve.
 */
export function detectViewMode(feedUrl: string | null, sourceUrl: string): ViewMode {
    const sourceHost = hostOf(sourceUrl);
    const feedHost = hostOf(feedUrl);
    if (isYouTubeHost(sourceHost) || isYouTubeHost(feedHost)) {
        return 'youtube';
    }
    if (isRedditHost(sourceHost) || isRedditHost(feedHost)) {
        return 'reddit';
    }
    return 'article';
}

/**
 * The home-page-style list layout a feed can offer, from its own URL, or
 * `null` when the feed isn't a YouTube/Reddit type. The list pane uses this
 * to decide whether to show the grid/feed toggle.
 */
export function feedHomeType(feedUrl: string | null): 'youtube' | 'reddit' | null {
    const mode = detectViewMode(feedUrl, feedUrl ?? '');
    return mode === 'article' ? null : mode;
}

/**
 * A stable hue (0–359) from a seed string — the deterministic poster tint
 * for a video card, so each reads as its own without ever fetching the real
 * thumbnail (shared by the YouTube facade and the home grid).
 */
export function posterHue(seed: string): number {
    return [...seed].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);
}

/**
 * The canonical still URL for a YouTube video id. `hqdefault` (480×360)
 * always exists for a valid id (unlike `maxresdefault`). Only fetched when
 * the remote-media prefetch setting is on, through the policed image cache.
 */
export function youTubeThumbnailUrl(videoId: string): string {
    return `https://i.ytimg.com/vi/${videoId}/hqdefault.jpg`;
}

/** A canonical YouTube video id is exactly 11 url-safe base64 characters. */
const VIDEO_ID = /^[A-Za-z0-9_-]{11}$/;

function validId(id: string | undefined): id is string {
    return id !== undefined && VIDEO_ID.test(id);
}

/**
 * The video id embedded in a YouTube URL, or `null`. Handles `watch?v=`,
 * `youtu.be/<id>`, and `/embed|/shorts|/v/<id>` — the forms a feed's
 * source URL actually carries. Never fetches; parses the given string only.
 */
export function youTubeVideoId(url: string): string | null {
    let parsed: URL;
    try {
        parsed = new URL(url);
    } catch {
        return null;
    }
    const host = parsed.hostname.toLowerCase().replace(/^www\./, '');
    if (host === 'youtu.be') {
        const id = parsed.pathname.slice(1).split('/')[0];
        return validId(id) ? id : null;
    }
    if (!isYouTubeHost(host)) {
        return null;
    }
    const queryId = parsed.searchParams.get('v');
    if (queryId !== null && validId(queryId)) {
        return queryId;
    }
    const pathMatch = parsed.pathname.match(/^\/(?:embed|shorts|v)\/([^/?#]+)/);
    if (pathMatch !== null && validId(pathMatch[1])) {
        return pathMatch[1];
    }
    return null;
}

/** The subreddit name in a Reddit URL (`/r/<name>`), or `null`. */
export function subredditOf(url: string): string | null {
    let parsed: URL;
    try {
        parsed = new URL(url);
    } catch {
        return null;
    }
    const match = parsed.pathname.match(/\/r\/([A-Za-z0-9_]+)/);
    return match === null ? null : match[1];
}

/**
 * The RSS-native structure of a Reddit item, extracted from the feed's own
 * `content_html` footer — Reddit RSS ends every item with `submitted by
 * /u/<author> … [link] [comments]`. Parsing the feed's markup is not
 * enrichment (no API, no fetch); it just reads what the feed already sent.
 * Every field degrades to `null` when the shape isn't recognised.
 */
export interface RedditPost {
    /** `/u/<name>` author, without the leading slash-u. */
    author: string | null;
    /** The comments/permalink URL (the `[comments]` anchor, else the source). */
    commentsUrl: string;
    /** The external link for a link post; `null` for a self/text post. */
    linkUrl: string | null;
    /** True when the post points off-site (link post) vs. a self post. */
    isLinkPost: boolean;
}

function anchorByText(doc: Document, text: string): HTMLAnchorElement | null {
    for (const a of doc.querySelectorAll('a[href]')) {
        if (a.textContent?.trim().toLowerCase() === text) {
            return a as HTMLAnchorElement;
        }
    }
    return null;
}

/** The `/u/<name>` author from a Reddit item's footer anchors, or `null`. */
function redditAuthorOf(doc: Document): string | null {
    for (const a of doc.querySelectorAll('a[href]')) {
        if (/\/(?:u|user)\//.test(a.getAttribute('href') ?? '')) {
            return (a.textContent ?? '').trim().replace(/^\/?u\//, '') || null;
        }
    }
    return null;
}

export function parseRedditPost(html: string, sourceUrl: string): RedditPost {
    // No DOM (e.g. SSR/tests without jsdom) → degrade to the source URL only.
    if (typeof DOMParser === 'undefined') {
        return { author: null, commentsUrl: sourceUrl, linkUrl: null, isLinkPost: false };
    }
    const doc = new DOMParser().parseFromString(html, 'text/html');

    const commentsUrl = anchorByText(doc, '[comments]')?.getAttribute('href') ?? sourceUrl;
    const rawLink = anchorByText(doc, '[link]')?.getAttribute('href') ?? null;
    // A self post's [link] equals its [comments] permalink — treat that as no
    // external link.
    const linkUrl = rawLink !== null && rawLink !== commentsUrl ? rawLink : null;

    return { author: redditAuthorOf(doc), commentsUrl, linkUrl, isLinkPost: linkUrl !== null };
}

/**
 * The article body with Reddit's boilerplate footer removed — the trailing
 * `submitted by … [link] [comments]` block, whose data we surface as buttons
 * instead. Returns the input unchanged when no such footer is found or when
 * no DOM is available. Never adds content; only prunes the known footer node.
 */
export function stripRedditFooter(html: string): string {
    if (typeof DOMParser === 'undefined') {
        return html;
    }
    const doc = new DOMParser().parseFromString(html, 'text/html');
    const commentsAnchor = anchorByText(doc, '[comments]');
    if (commentsAnchor === null) {
        return html;
    }
    // Remove the smallest self-contained block that holds the footer anchor
    // (Reddit wraps it in a <table> or a trailing <p>).
    const footer = commentsAnchor.closest('table, p, div');
    footer?.remove();
    return doc.body.innerHTML.trim();
}
