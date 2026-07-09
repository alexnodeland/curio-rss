/**
 * Source presets — turning what a user knows (a subreddit, a Mastodon handle,
 * a YouTube channel, "Hacker News") into the feed URL they don't. Pure URL
 * construction over well-known conventions: no new fetch paths, no APIs. The
 * one case that genuinely needs a fetch — resolving a YouTube @handle to its
 * channel id — is left to desktop discovery (which already fetches the page
 * through the policed client); this module only handles what can be built from
 * the string alone.
 *
 * Kept dependency-light (only the i18n key type) so the fiddly pattern logic
 * is unit-tested without a component or IPC.
 */
import type { MessageKey } from '$lib/i18n';

/** The platforms this module can recognize from a typed/pasted string. */
export type SourceKind = 'reddit' | 'mastodon' | 'youtube' | 'hackernews';

/** A recognized source: the feed URL to subscribe to, plus a folder to suggest. */
export interface DetectedSource {
    kind: SourceKind;
    /** The constructed, ready-to-subscribe feed URL. */
    feedUrl: string;
    /** A folder tag to pre-fill (e.g. `Community/Reddit`). */
    suggestedTag: string;
}

/** One Hacker News feed the presets row offers (hnrss.org). */
export interface HnPreset {
    id: string;
    labelKey: MessageKey;
    feedUrl: string;
}

/** The Hacker News presets (hnrss.org — a plain RSS bridge, no API/auth). */
export const HN_PRESETS: readonly HnPreset[] = [
    { id: 'frontpage', labelKey: 'presets.hn.frontpage', feedUrl: 'https://hnrss.org/frontpage' },
    { id: 'newest', labelKey: 'presets.hn.newest', feedUrl: 'https://hnrss.org/newest' },
    { id: 'ask', labelKey: 'presets.hn.ask', feedUrl: 'https://hnrss.org/ask' },
    { id: 'show', labelKey: 'presets.hn.show', feedUrl: 'https://hnrss.org/show' },
    { id: 'best', labelKey: 'presets.hn.best', feedUrl: 'https://hnrss.org/bestcomments' },
] as const;

/** Hosts whose `/@handle` path is NOT a Mastodon profile (they reuse the shape). */
const NON_MASTODON_HOSTS = /(^|\.)(youtube\.com|youtu\.be|twitter\.com|x\.com|threads\.net)$/;

/**
 * Recognizes a source from a typed/pasted string and returns the feed URL to
 * subscribe to, or `null` if nothing matched (fall back to raw-URL + discovery).
 * Order matters: YouTube is tried before Mastodon so a `youtube.com/@handle`
 * URL is not mistaken for a Mastodon profile.
 */
export function detectSource(input: string): DetectedSource | null {
    const raw = input.trim();
    if (raw === '') {
        return null;
    }
    return detectReddit(raw) ?? detectYouTube(raw) ?? detectMastodon(raw) ?? detectHackerNews(raw);
}

/** Parses `raw` as a URL, defaulting a missing scheme to https. */
function parseUrl(raw: string): URL | null {
    try {
        const withScheme = /^[a-z][a-z0-9+.-]*:\/\//i.test(raw) ? raw : `https://${raw}`;
        return new URL(withScheme);
    } catch {
        return null;
    }
}

function reddit(kind: 'r' | 'user', name: string): DetectedSource {
    return {
        kind: 'reddit',
        feedUrl: `https://www.reddit.com/${kind}/${name}/.rss`,
        suggestedTag: 'Community/Reddit',
    };
}

/** `r/foo`, `/r/foo`, `u/foo`, or any reddit.com subreddit/user URL. */
function detectReddit(raw: string): DetectedSource | null {
    // Bare shorthand — matched before URL parsing (`r/foo` is not a URL).
    const bareSub = /^\/?r\/([A-Za-z0-9_]{2,21})\b/.exec(raw);
    if (bareSub) {
        return reddit('r', bareSub[1]);
    }
    const bareUser = /^\/?u(?:ser)?\/([A-Za-z0-9_-]{2,20})\b/.exec(raw);
    if (bareUser) {
        return reddit('user', bareUser[1]);
    }
    const parsed = parseUrl(raw);
    if (parsed === null || !/(^|\.)reddit\.com$/.test(parsed.hostname.toLowerCase())) {
        return null;
    }
    const sub = /^\/r\/([A-Za-z0-9_]{2,21})/.exec(parsed.pathname);
    if (sub) {
        return reddit('r', sub[1]);
    }
    const user = /^\/user\/([A-Za-z0-9_-]{2,20})/.exec(parsed.pathname);
    return user ? reddit('user', user[1]) : null;
}

/**
 * A YouTube `/channel/UC…` URL (or an already-built feeds URL). A `/@handle`,
 * `/c/name` or `/user/name` URL needs the page fetched to find its channel id,
 * so it returns `null` here and is left to desktop discovery.
 */
function detectYouTube(raw: string): DetectedSource | null {
    const parsed = parseUrl(raw);
    if (parsed === null || !/(^|\.)youtube\.com$/.test(parsed.hostname.toLowerCase())) {
        return null;
    }
    if (parsed.pathname === '/feeds/videos.xml' && parsed.searchParams.has('channel_id')) {
        return youtube(parsed.toString());
    }
    const channel = /^\/channel\/(UC[A-Za-z0-9_-]{22})\/?$/.exec(parsed.pathname);
    return channel
        ? youtube(`https://www.youtube.com/feeds/videos.xml?channel_id=${channel[1]}`)
        : null;
}

function youtube(feedUrl: string): DetectedSource {
    return { kind: 'youtube', feedUrl, suggestedTag: 'Video/YouTube' };
}

/** `@user@instance` or an `https://instance/@user` Mastodon profile URL. */
function detectMastodon(raw: string): DetectedSource | null {
    const acct = /^@([A-Za-z0-9_]+)@([A-Za-z0-9.-]+\.[A-Za-z]{2,})$/.exec(raw);
    if (acct) {
        return mastodon(acct[2], acct[1]);
    }
    const parsed = parseUrl(raw);
    if (parsed === null) {
        return null;
    }
    const host = parsed.hostname.toLowerCase();
    if (NON_MASTODON_HOSTS.test(host)) {
        return null;
    }
    const profile = /^\/@([A-Za-z0-9_]+)\/?$/.exec(parsed.pathname);
    return profile ? mastodon(host, profile[1]) : null;
}

function mastodon(instance: string, user: string): DetectedSource {
    return {
        kind: 'mastodon',
        feedUrl: `https://${instance}/@${user}.rss`,
        suggestedTag: 'Community/Mastodon',
    };
}

/** A `news.ycombinator.com` URL → the Hacker News front-page bridge feed. */
function detectHackerNews(raw: string): DetectedSource | null {
    const parsed = parseUrl(raw);
    if (parsed === null || !/(^|\.)ycombinator\.com$/.test(parsed.hostname.toLowerCase())) {
        return null;
    }
    return {
        kind: 'hackernews',
        feedUrl: HN_PRESETS[0].feedUrl,
        suggestedTag: 'Community/Hacker News',
    };
}
