/**
 * View-mode heuristics: which reader layout an article resolves to, and the
 * RSS-native id/name extraction the layouts lean on. Pure — no IPC.
 */
import { detectViewMode, subredditOf, youTubeVideoId } from '$lib/reader/view-mode';
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
