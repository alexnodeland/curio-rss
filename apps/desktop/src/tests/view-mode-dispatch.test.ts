/**
 * The reader body dispatcher and its layouts: mode selection from URL
 * heuristics, and the YouTube facade's click-to-load behaviour (no iframe —
 * hence no network — until the user clicks). Layouts are RSS-native only;
 * these tests assert no enrichment call is ever made (there is no IPC here).
 */
import ViewModeDispatch from '$components/reader/ViewModeDispatch.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { articleFixture, feedFixture } from './ipc-harness';

afterEach(() => {
    cleanup();
});

describe('ViewModeDispatch', () => {
    it('renders the default article layout for an ordinary source', () => {
        const { container } = render(ViewModeDispatch, {
            article: articleFixture({
                source_url: 'https://blog.example.org/post/1',
                content_html: '<p>Ordinary <em>body</em></p>',
            }),
            feed: feedFixture({ url: 'https://blog.example.org/feed' }),
        });
        expect(container.querySelector('[data-view-mode]')?.getAttribute('data-view-mode')).toBe(
            'article',
        );
        expect(container.querySelector('em')?.textContent).toBe('body');
    });

    it('renders the Reddit layout with the subreddit chip', () => {
        const { container, getByText } = render(ViewModeDispatch, {
            article: articleFixture({
                source_url: 'https://www.reddit.com/r/rust/comments/abc/title/',
            }),
            feed: feedFixture({ url: 'https://www.reddit.com/r/rust/.rss' }),
        });
        expect(container.querySelector('[data-view-mode]')?.getAttribute('data-view-mode')).toBe(
            'reddit',
        );
        expect(getByText('r/rust')).toBeTruthy();
    });

    it('renders the YouTube facade and only loads the iframe on click', async () => {
        const { container, getByRole } = render(ViewModeDispatch, {
            article: articleFixture({
                title: 'A talk',
                source_url: 'https://www.youtube.com/watch?v=dQw4w9WgXcQ',
            }),
            feed: null,
        });
        expect(container.querySelector('[data-view-mode]')?.getAttribute('data-view-mode')).toBe(
            'youtube',
        );

        // Before the click: a poster button, no iframe (nothing has loaded).
        expect(container.querySelector('iframe')).toBeNull();
        const poster = getByRole('button', { name: /Play A talk/ });

        await fireEvent.click(poster);

        // After the click: the nocookie iframe, scoped to the extracted id.
        const frame = container.querySelector('iframe');
        expect(frame).not.toBeNull();
        expect(frame?.getAttribute('src')).toContain(
            'https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ',
        );
    });

    it('degrades a YouTube item without a parseable id to the body only', () => {
        const { container } = render(ViewModeDispatch, {
            article: articleFixture({ source_url: 'https://www.youtube.com/feed/subscriptions' }),
            feed: feedFixture({ url: 'https://www.youtube.com/feeds/videos.xml?channel_id=UC1' }),
        });
        expect(container.querySelector('[data-view-mode]')?.getAttribute('data-view-mode')).toBe(
            'youtube',
        );
        expect(container.querySelector('iframe')).toBeNull();
        expect(container.querySelector('button')).toBeNull(); // no facade poster
    });
});
