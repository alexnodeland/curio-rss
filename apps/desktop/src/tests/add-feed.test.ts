/**
 * The add-feed modal: autodiscovery (`discover_feeds`) surfaces candidates,
 * subscribe dedupes against `get_feed_by_url` then calls `add_feed`, and the
 * Google-favicon fallback stays an explicit opt-in (`set_setting`). Errors
 * unwrap through the generated `CommandError` shape.
 */
import AddFeedModal from '$components/modals/AddFeedModal.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { resetImageCache } from '$lib/utils/images';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    commandErrorFixture,
    discoveryFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';

function base(overrides: Record<string, Responder> = {}): Record<string, Responder> {
    return {
        discover_feeds: discoveryFixture(),
        get_cached_image: '/cache/fav.png',
        get_feed_by_url: null,
        add_feed: feedFixture({ title: 'Example Feed' }),
        set_setting: null,
        ...overrides,
    };
}

describe('AddFeedModal', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetImageCache();
        uiStore.reset();
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [];
        harness?.teardown();
        harness = null;
    });

    it('discovers feeds and subscribes to the chosen candidate', async () => {
        const onclose = vi.fn();
        harness = installIpcHarness(base());
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://example.org' },
        });
        await fireEvent.click(getByText('Find feeds'));
        await flushIpc();

        // The discovered candidate is listed.
        expect(getByText('Example Feed')).toBeTruthy();
        expect(harness.callsFor('discover_feeds')).toEqual([{ url: 'https://example.org' }]);

        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        // Deduped, then subscribed to the candidate feed URL (camelCase wire arg).
        expect(harness.callsFor('get_feed_by_url')).toEqual([
            { url: 'https://example.org/feed.xml' },
        ]);
        expect(harness.callsFor('add_feed')).toEqual([
            { newFeed: { url: 'https://example.org/feed.xml', title: null, tags: [] } },
        ]);
        expect(onclose).toHaveBeenCalledOnce();
    });

    it('discards a stale discovery candidate when the URL is edited', async () => {
        harness = installIpcHarness(base());
        const { getByText, queryByText, getByPlaceholderText } = render(AddFeedModal, {
            onclose: vi.fn(),
        });
        const input = getByPlaceholderText('https://example.com');

        await fireEvent.input(input, { target: { value: 'https://example.org' } });
        await fireEvent.click(getByText('Find feeds'));
        await flushIpc();
        expect(getByText('Example Feed')).toBeTruthy(); // candidate listed

        // Editing the URL must clear the now-stale candidate...
        await fireEvent.input(input, { target: { value: 'https://other.example/feed.xml' } });
        expect(queryByText('Example Feed')).toBeNull();

        // ...so subscribe uses the edited URL, not the old candidate's feed URL.
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();
        expect(harness.callsFor('add_feed')).toEqual([
            { newFeed: { url: 'https://other.example/feed.xml', title: null, tags: [] } },
        ]);
    });

    it('parses comma-separated tags into the subscription', async () => {
        harness = installIpcHarness(base());
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://example.org/feed.xml' },
        });
        await fireEvent.input(getByPlaceholderText('comma, separated'), {
            target: { value: ' rust,  reading ,, ' },
        });
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(harness.callsFor('add_feed')[0]).toEqual({
            newFeed: {
                url: 'https://example.org/feed.xml',
                title: null,
                tags: ['rust', 'reading'],
            },
        });
    });

    it('refuses a duplicate feed without calling add_feed', async () => {
        harness = installIpcHarness(base({ get_feed_by_url: feedFixture() }));
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://example.org/feed.xml' },
        });
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(harness.callsFor('add_feed')).toHaveLength(0);
        expect(uiStore.toasts.at(-1)?.message).toBe('You are already subscribed to this feed.');
    });

    it('surfaces a discovery failure as an error toast', async () => {
        harness = installIpcHarness(
            base({
                discover_feeds: rejectWith(
                    commandErrorFixture({
                        kind: 'user',
                        code: 'NETWORK',
                        message: 'host unreachable',
                    }),
                ),
            }),
        );
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://nope.test' },
        });
        await fireEvent.click(getByText('Find feeds'));
        await flushIpc();

        expect(uiStore.toasts.at(-1)?.message).toBe('host unreachable');
        expect(uiStore.toasts.at(-1)?.tone).toBe('error');
    });

    it('persists the Google-favicon opt-in through set_setting', async () => {
        harness = installIpcHarness(base());
        const { getByRole } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.click(getByRole('checkbox'));
        await flushIpc();

        expect(uiStore.allowRemoteFavicon).toBe(true);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.favicon.allow-remote', value: 'true' },
        ]);
    });

    it('recognizes an r/ shorthand and subscribes to the built feed with a suggested folder', async () => {
        harness = installIpcHarness(base());
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'r/rust' },
        });
        // The detection chip shows the constructed feed URL — no discovery needed.
        expect(getByText('https://www.reddit.com/r/rust/.rss')).toBeTruthy();

        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(harness.callsFor('discover_feeds')).toHaveLength(0);
        expect(harness.callsFor('add_feed')).toEqual([
            {
                newFeed: {
                    url: 'https://www.reddit.com/r/rust/.rss',
                    title: null,
                    tags: ['Community/Reddit'],
                },
            },
        ]);
    });

    it('offers Hacker News presets that fill a working feed URL', async () => {
        harness = installIpcHarness(base());
        const { getByText, queryByText, getByPlaceholderText } = render(AddFeedModal, {
            onclose: vi.fn(),
        });

        // The sub-presets are hidden until the HN button is pressed.
        expect(queryByText('Front page')).toBeNull();
        await fireEvent.click(getByText('Hacker News'));
        await fireEvent.click(getByText('Front page'));

        // The preset fills the URL input with the hnrss feed URL directly.
        expect((getByPlaceholderText('https://example.com') as HTMLInputElement).value).toBe(
            'https://hnrss.org/frontpage',
        );
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(harness.callsFor('add_feed')).toEqual([
            {
                newFeed: {
                    url: 'https://hnrss.org/frontpage',
                    title: null,
                    tags: ['Community/Hacker News'],
                },
            },
        ]);
    });
});
