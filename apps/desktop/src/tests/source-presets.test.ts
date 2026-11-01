/**
 * The source recognizers: turning a subreddit / Mastodon handle / YouTube
 * channel / GitHub repository / Kill the Newsletter inbox / Hacker News URL
 * into the feed URL to subscribe to. Pure URL
 * construction — the one fetch-dependent case (a YouTube @handle) is left to
 * discovery and returns null here.
 */
import { HN_PRESETS, detectSource, hostOf } from '$lib/utils/source-presets';
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

describe('detectSource — GitHub', () => {
    it('builds the releases feed from a repository URL', () => {
        expect(detectSource('https://github.com/rust-lang/rust')).toEqual({
            kind: 'github',
            feedUrl: 'https://github.com/rust-lang/rust/releases.atom',
            suggestedTag: 'Dev/GitHub',
        });
    });

    it('accepts scheme-less, www, .git, /releases and /tags forms', () => {
        expect(detectSource('github.com/tauri-apps/tauri')?.feedUrl).toBe(
            'https://github.com/tauri-apps/tauri/releases.atom',
        );
        expect(detectSource('https://www.github.com/a/b.git')?.feedUrl).toBe(
            'https://github.com/a/b/releases.atom',
        );
        expect(detectSource('https://github.com/a/b/releases/')?.feedUrl).toBe(
            'https://github.com/a/b/releases.atom',
        );
        expect(detectSource('https://github.com/a/b/tags')?.feedUrl).toBe(
            'https://github.com/a/b/tags.atom',
        );
        // An already-built feed URL passes through.
        expect(detectSource('https://github.com/a/b/releases.atom')?.feedUrl).toBe(
            'https://github.com/a/b/releases.atom',
        );
    });

    it('leaves site pages, bare owners and deep repo pages alone', () => {
        expect(detectSource('https://github.com/')).toBeNull();
        expect(detectSource('https://github.com/rust-lang')).toBeNull();
        expect(detectSource('https://github.com/topics/rust')).toBeNull();
        expect(detectSource('https://github.com/orgs/rust-lang')).toBeNull();
        expect(detectSource('https://github.com/a/b/issues')).toBeNull();
        expect(detectSource('https://github.com/a/b/blob/main/README.md')).toBeNull();
    });
});

describe('detectSource — newsletters (Kill the Newsletter)', () => {
    it('builds the inbox feed from its email address', () => {
        expect(detectSource('k7xq2p@kill-the-newsletter.com')).toEqual({
            kind: 'newsletter',
            feedUrl: 'https://kill-the-newsletter.com/feeds/k7xq2p.xml',
            suggestedTag: 'Newsletters',
        });
        expect(detectSource('mailto:k7xq2p@kill-the-newsletter.com')?.feedUrl).toBe(
            'https://kill-the-newsletter.com/feeds/k7xq2p.xml',
        );
    });

    it('recognizes the inbox feed URL itself', () => {
        expect(detectSource('https://kill-the-newsletter.com/feeds/k7xq2p.xml')?.feedUrl).toBe(
            'https://kill-the-newsletter.com/feeds/k7xq2p.xml',
        );
        expect(detectSource('kill-the-newsletter.com/feeds/k7xq2p')?.feedUrl).toBe(
            'https://kill-the-newsletter.com/feeds/k7xq2p.xml',
        );
    });

    it('ignores the site itself and other addresses', () => {
        expect(detectSource('https://kill-the-newsletter.com/')).toBeNull();
        expect(detectSource('someone@example.com')).toBeNull();
    });
});

describe('hostOf', () => {
    it('finds the host of scheme-less input that new URL() rejects', () => {
        expect(hostOf('example.com')).toBe('example.com');
        expect(hostOf('  blog.example.org/posts ')).toBe('blog.example.org');
        expect(hostOf('//example.net/')).toBe('example.net');
    });

    it('keeps a full URL working and rejects non-URLs', () => {
        expect(hostOf('https://news.example/feed.xml')).toBe('news.example');
        expect(hostOf('')).toBeNull();
        expect(hostOf('not a url')).toBeNull();
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
