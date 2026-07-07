/**
 * View-mode heuristics: which reader layout an article resolves to, and the
 * RSS-native id/name extraction the layouts lean on. Pure — no IPC.
 */
import {
    detectViewMode,
    feedHomeType,
    parseRedditPost,
    posterHue,
    stripRedditFooter,
    subredditOf,
    youTubeThumbnailUrl,
    youTubeVideoId,
} from '$lib/reader/view-mode';
import { describe, expect, it } from 'vitest';

describe('detectViewMode', () => {
    it('routes YouTube by source URL', () => {
        expect(detectViewMode(null, 'https://www.youtube.com/watch?v=dQw4w9WgXcQ')).toBe('youtube');
        expect(detectViewMode(null, 'https://youtu.be/dQw4w9WgXcQ')).toBe('youtube');
    });

    it('routes YouTube by the channel feed URL even when the item URL is bare', () => {
        expect(
            detectViewMode(
                'https://www.youtube.com/feeds/videos.xml?channel_id=UC123',
                'https://example.com/x',
            ),
        ).toBe('youtube');
    });

    it('routes Reddit by host', () => {
        expect(
            detectViewMode(
                'https://www.reddit.com/r/rust/.rss',
                'https://www.reddit.com/r/rust/comments/abc/title/',
            ),
        ).toBe('reddit');
    });

    it('prefers YouTube over Reddit for a crossposted video', () => {
        expect(
            detectViewMode('https://www.reddit.com/r/videos/.rss', 'https://youtu.be/dQw4w9WgXcQ'),
        ).toBe('youtube');
    });

    it('falls back to the article layout for everything else', () => {
        expect(
            detectViewMode('https://blog.example.org/feed', 'https://blog.example.org/p/1'),
        ).toBe('article');
        expect(detectViewMode(null, 'not a url')).toBe('article');
    });
});

describe('youTubeVideoId', () => {
    it('extracts from watch, youtu.be, embed, and shorts forms', () => {
        expect(youTubeVideoId('https://www.youtube.com/watch?v=dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
        expect(youTubeVideoId('https://youtu.be/dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
        expect(youTubeVideoId('https://www.youtube.com/embed/dQw4w9WgXcQ?rel=0')).toBe(
            'dQw4w9WgXcQ',
        );
        expect(youTubeVideoId('https://www.youtube.com/shorts/dQw4w9WgXcQ')).toBe('dQw4w9WgXcQ');
        expect(youTubeVideoId('https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ')).toBe(
            'dQw4w9WgXcQ',
        );
    });

    it('rejects non-YouTube hosts and malformed ids', () => {
        expect(youTubeVideoId('https://vimeo.com/watch?v=dQw4w9WgXcQ')).toBeNull();
        expect(youTubeVideoId('https://www.youtube.com/watch?v=short')).toBeNull();
        expect(youTubeVideoId('https://www.youtube.com/feed/subscriptions')).toBeNull();
        expect(youTubeVideoId('garbage')).toBeNull();
    });
});

describe('subredditOf', () => {
    it('reads the subreddit from the path', () => {
        expect(subredditOf('https://www.reddit.com/r/rust/comments/abc/title/')).toBe('rust');
        expect(subredditOf('https://old.reddit.com/r/ProgrammingLanguages/')).toBe(
            'ProgrammingLanguages',
        );
    });

    it('returns null without an /r/ segment', () => {
        expect(subredditOf('https://www.reddit.com/user/someone')).toBeNull();
        expect(subredditOf('nonsense')).toBeNull();
    });
});

const LINK_POST = `<p>body text</p><table><tr><td><a href="https://www.reddit.com/r/typography/comments/abc/x/">t</a></td><td> submitted by <a href="https://www.reddit.com/user/serif_lover">/u/serif_lover</a> to <a href="https://www.reddit.com/r/typography/">r/typography</a> <br/> <span><a href="https://foundry.example/canela">[link]</a></span> <span><a href="https://www.reddit.com/r/typography/comments/abc/x/">[comments]</a></span> </td></tr></table>`;

const SELF_POST = `<p>a self post</p><table><tr><td> submitted by <a href="https://www.reddit.com/user/writer">/u/writer</a> to <a href="https://www.reddit.com/r/rust/">r/rust</a> <br/> <span><a href="https://www.reddit.com/r/rust/comments/def/y/">[link]</a></span> <span><a href="https://www.reddit.com/r/rust/comments/def/y/">[comments]</a></span> </td></tr></table>`;

describe('parseRedditPost', () => {
    it('reads author, comments, and the external link of a link post', () => {
        const p = parseRedditPost(LINK_POST, 'https://www.reddit.com/r/typography/comments/abc/x/');
        expect(p.author).toBe('serif_lover');
        expect(p.commentsUrl).toBe('https://www.reddit.com/r/typography/comments/abc/x/');
        expect(p.linkUrl).toBe('https://foundry.example/canela');
        expect(p.isLinkPost).toBe(true);
    });

    it('treats a self post (link === comments) as having no external link', () => {
        const p = parseRedditPost(SELF_POST, 'https://www.reddit.com/r/rust/comments/def/y/');
        expect(p.author).toBe('writer');
        expect(p.linkUrl).toBeNull();
        expect(p.isLinkPost).toBe(false);
    });

    it('degrades to the source URL when the footer is absent', () => {
        const p = parseRedditPost(
            '<p>no footer here</p>',
            'https://www.reddit.com/r/x/comments/z/',
        );
        expect(p.commentsUrl).toBe('https://www.reddit.com/r/x/comments/z/');
        expect(p.linkUrl).toBeNull();
        expect(p.author).toBeNull();
    });
});

describe('stripRedditFooter', () => {
    it('prunes the submitted-by/[comments] boilerplate block', () => {
        const out = stripRedditFooter(LINK_POST);
        expect(out).toContain('body text');
        expect(out).not.toContain('[comments]');
        expect(out).not.toContain('submitted by');
    });

    it('leaves a body with no footer untouched', () => {
        const html = '<p>just a body</p>';
        expect(stripRedditFooter(html)).toContain('just a body');
    });
});

describe('feedHomeType', () => {
    it('maps YouTube/Reddit feeds to a home type and others to null', () => {
        expect(feedHomeType('https://youtube.com/@Fireship/videos.rss')).toBe('youtube');
        expect(feedHomeType('https://reddit.com/r/typography/.rss')).toBe('reddit');
        expect(feedHomeType('https://overreacted.io/rss.xml')).toBeNull();
        expect(feedHomeType(null)).toBeNull();
    });
});

describe('posterHue', () => {
    it('is deterministic and in range', () => {
        const a = posterHue('dQw4w9WgXcQ');
        expect(a).toBe(posterHue('dQw4w9WgXcQ'));
        expect(a).toBeGreaterThanOrEqual(0);
        expect(a).toBeLessThan(360);
        expect(posterHue('other')).not.toBe(a);
    });
});

describe('youTubeThumbnailUrl', () => {
    it('builds the hqdefault still URL', () => {
        expect(youTubeThumbnailUrl('dQw4w9WgXcQ')).toBe(
            'https://i.ytimg.com/vi/dQw4w9WgXcQ/hqdefault.jpg',
        );
    });
});
