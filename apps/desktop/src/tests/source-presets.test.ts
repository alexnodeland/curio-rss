/**
 * The source recognizers: turning a subreddit / Mastodon handle / YouTube
 * channel / Hacker News URL into the feed URL to subscribe to. Pure URL
 * construction — the one fetch-dependent case (a YouTube @handle) is left to
 * discovery and returns null here.
 */
import { HN_PRESETS, detectSource } from '$lib/utils/source-presets';
import { describe, expect, it } from 'vitest';

describe('detectSource — Reddit', () => {
    it('builds a subreddit feed from the r/ shorthand', () => {
        expect(detectSource('r/rust')).toEqual({
            kind: 'reddit',
            feedUrl: 'https://www.reddit.com/r/rust/.rss',
            suggestedTag: 'Community/Reddit',
        });
    });

    it('accepts a leading slash and a full URL, normalizing to www', () => {
        expect(detectSource('/r/rust')?.feedUrl).toBe('https://www.reddit.com/r/rust/.rss');
        expect(detectSource('https://old.reddit.com/r/rust/top/')?.feedUrl).toBe(
            'https://www.reddit.com/r/rust/.rss',
        );
    });

    it('handles a user feed', () => {
        expect(detectSource('u/spez')?.feedUrl).toBe('https://www.reddit.com/user/spez/.rss');
        expect(detectSource('https://www.reddit.com/user/spez')?.feedUrl).toBe(
            'https://www.reddit.com/user/spez/.rss',
        );
    });
});

describe('detectSource — Mastodon', () => {
    it('builds a profile feed from the @user@instance handle', () => {
        expect(detectSource('@simon@mastodon.social')).toEqual({
            kind: 'mastodon',
            feedUrl: 'https://mastodon.social/@simon.rss',
            suggestedTag: 'Community/Mastodon',
        });
    });

    it('builds a profile feed from a profile URL', () => {
        expect(detectSource('https://fosstodon.org/@curio')?.feedUrl).toBe(
            'https://fosstodon.org/@curio.rss',
        );
    });

    it('does not treat a YouTube @handle URL as a Mastodon profile', () => {
        // YouTube reuses the /@handle shape; it needs discovery, not .rss.
        expect(detectSource('https://www.youtube.com/@mkbhd')).toBeNull();
    });
});

describe('detectSource — YouTube', () => {
    it('builds the videos feed from a /channel/UC… URL', () => {
        expect(detectSource('https://www.youtube.com/channel/UCXuqSBlHAE6Xw-yeJA0Tunw')).toEqual({
            kind: 'youtube',
            feedUrl: 'https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw',
            suggestedTag: 'Video/YouTube',
        });
    });

    it('passes an already-built feeds URL through', () => {
        const url = 'https://www.youtube.com/feeds/videos.xml?channel_id=UCXuqSBlHAE6Xw-yeJA0Tunw';
        expect(detectSource(url)?.feedUrl).toBe(url);
    });

    it('returns null for a handle URL (discovery resolves it)', () => {
        expect(detectSource('https://www.youtube.com/@mkbhd')).toBeNull();
        expect(detectSource('https://www.youtube.com/c/mkbhd')).toBeNull();
    });
});

describe('detectSource — Hacker News', () => {
    it('maps a news.ycombinator.com URL to the front-page bridge', () => {
        expect(detectSource('https://news.ycombinator.com/')).toEqual({
            kind: 'hackernews',
            feedUrl: 'https://hnrss.org/frontpage',
            suggestedTag: 'Community/Hacker News',
        });
    });
});

describe('detectSource — misses', () => {
    it('returns null for an ordinary blog URL or empty input', () => {
        expect(detectSource('https://example.com/blog')).toBeNull();
        expect(detectSource('   ')).toBeNull();
        expect(detectSource('not a url')).toBeNull();
    });
});

describe('HN_PRESETS', () => {
    it('offers the five hnrss feeds', () => {
        expect(HN_PRESETS.map((preset) => preset.id)).toEqual([
            'frontpage',
            'newest',
            'ask',
            'show',
            'best',
        ]);
        for (const preset of HN_PRESETS) {
            expect(preset.feedUrl.startsWith('https://hnrss.org/')).toBe(true);
        }
    });
});
