/**
 * The add-feed modal: autodiscovery (`discover_feeds`) surfaces candidates,
 * subscribe dedupes against `get_feed_by_url` then calls `add_feed`, and the
 * Google-favicon fallback stays an explicit opt-in (`set_setting`). Errors
 * unwrap through the generated `CommandError` shape.
 */
import AddFeedModal from '$components/modals/AddFeedModal.svelte';
import { localeStore } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { resetImageCache } from '$lib/utils/images';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    type Responder,
    articleFixture,
    commandErrorFixture,
    discoveryFixture,
    feedFixture,
    flushIpc,
    installIpcHarness,
    rejectWith,
} from './ipc-harness';
import { resetUiStores } from './ui-stores';

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
        resetUiStores();
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
            {
                newFeed: {
                    url: 'https://example.org/feed.xml',
                    title: null,
                    tags: [],
                    validate: true,
                },
            },
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
            {
                newFeed: {
                    url: 'https://other.example/feed.xml',
                    title: null,
                    tags: [],
                    validate: true,
                },
            },
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
                validate: true,
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
        expect(toastStore.toasts.at(-1)?.message).toBe('You are already subscribed to this feed.');
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

        expect(toastStore.toasts.at(-1)?.message).toBe('host unreachable');
        expect(toastStore.toasts.at(-1)?.tone).toBe('error');
    });

    it('shows a network failure in the active locale, naming the host', async () => {
        harness = installIpcHarness(
            base({
                discover_feeds: rejectWith(
                    commandErrorFixture({
                        kind: 'user',
                        code: 'CONNECTION_FAILED',
                        params: { host: 'nope.test' },
                        message:
                            'could not reach https://nope.test/: connection refused fetching "https://nope.test/": Connection refused (os error 111)',
                        recoverable: true,
                    }),
                ),
            }),
        );
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });
        const find = getByText('Find feeds');
        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://nope.test' },
        });

        localeStore.active = 'de';
        try {
            await fireEvent.click(find);
            await flushIpc();
        } finally {
            localeStore.reset();
        }

        expect(toastStore.toasts.at(-1)?.message).toBe(
            'Curio konnte keine Verbindung zu nope.test herstellen. Bitte Verbindung prüfen und erneut versuchen.',
        );
        expect(toastStore.toasts.at(-1)?.tone).toBe('error');
    });

    it('persists the Google-favicon opt-in through set_setting', async () => {
        harness = installIpcHarness(base());
        const { getByRole } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.click(getByRole('checkbox'));
        await flushIpc();

        expect(prefsStore.allowRemoteFavicon).toBe(true);
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
                    validate: true,
                },
            },
        ]);
    });

    it('saves the typed URL to read-later without subscribing', async () => {
        const onclose = vi.fn();
        harness = installIpcHarness(
            base({
                save_url: {
                    article: articleFixture({ title: 'Clipped Essay' }),
                    created: true,
                    hydrated: true,
                },
            }),
        );
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://example.org/essay' },
        });
        await fireEvent.click(getByText('Save to Read Later'));
        await flushIpc();

        expect(harness.callsFor('save_url')).toEqual([
            { url: 'https://example.org/essay', tags: [] },
        ]);
        expect(harness.callsFor('add_feed')).toHaveLength(0);
        expect(toastStore.toasts.at(-1)?.message).toContain('Clipped Essay');
        expect(toastStore.toasts.at(-1)?.tone).toBe('success');
        expect(onclose).toHaveBeenCalledOnce();
    });

    it('warns when a saved link could not be fetched, and disables save for non-URLs', async () => {
        harness = installIpcHarness(
            base({
                save_url: {
                    article: articleFixture({ title: 'https://dead.example/x' }),
                    created: true,
                    hydrated: false,
                },
            }),
        );
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });
        const saveButton = getByText('Save to Read Later').closest('button') as HTMLButtonElement;

        // Not a page URL (an r/ shorthand) — the save-link path stays off.
        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'r/rust' },
        });
        expect(saveButton.disabled).toBe(true);

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://dead.example/x' },
        });
        expect(saveButton.disabled).toBe(false);
        await fireEvent.click(saveButton);
        await flushIpc();

        expect(toastStore.toasts.at(-1)?.tone).toBe('warning');
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
                    validate: true,
                },
            },
        ]);
    });

    it('validates a typed homepage, and subscribes unvalidated only when asked', async () => {
        const onclose = vi.fn();
        let attempts = 0;
        harness = installIpcHarness(
            base({
                add_feed: () => {
                    attempts += 1;
                    return attempts === 1
                        ? Promise.reject(
                              commandErrorFixture({
                                  kind: 'user',
                                  code: 'NO_FEED',
                                  message: 'no feed found at example.com',
                              }),
                          )
                        : feedFixture({ url: 'https://example.com/', title: null });
                },
            }),
        );
        const { getByText, getByTestId, getByPlaceholderText } = render(AddFeedModal, { onclose });

        // A bare domain, Find never pressed: Subscribe still validates.
        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'example.com' },
        });
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(harness.callsFor('add_feed')).toEqual([
            { newFeed: { url: 'example.com', title: null, tags: [], validate: true } },
        ]);
        // Refused inline — no subscription, no error toast, the modal stays.
        expect(getByText('No feeds found on that page — try the direct feed URL.')).toBeTruthy();
        expect(toastStore.toasts).toHaveLength(0);
        expect(onclose).not.toHaveBeenCalled();

        // The explicit escape hatch subscribes without validation.
        await fireEvent.click(getByTestId('add-feed-unvalidated'));
        await flushIpc();
        expect(harness.callsFor('add_feed')[1]).toEqual({
            newFeed: { url: 'example.com', title: null, tags: [], validate: false },
        });
        expect(onclose).toHaveBeenCalledOnce();
    });

    it('offers the escape hatch when the address is unreachable, and toasts why', async () => {
        harness = installIpcHarness(
            base({
                add_feed: rejectWith(
                    commandErrorFixture({
                        kind: 'user',
                        code: 'NETWORK',
                        message: 'could not reach https://lan.example/',
                    }),
                ),
            }),
        );
        const { getByText, getByTestId, getByPlaceholderText } = render(AddFeedModal, {
            onclose: vi.fn(),
        });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'https://lan.example/' },
        });
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(toastStore.toasts.at(-1)?.message).toBe('could not reach https://lan.example/');
        expect(getByText('Couldn’t reach that address to check it for a feed.')).toBeTruthy();
        expect(getByTestId('add-feed-unvalidated')).toBeTruthy();
    });

    it.each([
        ['add_feed', 'Subscribe', 'HTTP_STATUS', { host: 'lan.example', status: '503' }],
        ['discover_feeds', 'Find feeds', 'NETWORK_TIMEOUT', { host: 'lan.example' }],
    ] as const)(
        'treats a finer fetch code from %s as unreachable too (%s)',
        async (command, button, code, params) => {
            harness = installIpcHarness(
                base({
                    [command]: rejectWith(
                        commandErrorFixture({
                            kind: 'user',
                            code,
                            params,
                            message: 'could not reach https://lan.example/',
                            recoverable: true,
                        }),
                    ),
                }),
            );
            const { getByText, getByTestId, getByPlaceholderText } = render(AddFeedModal, {
                onclose: vi.fn(),
            });

            await fireEvent.input(getByPlaceholderText('https://example.com'), {
                target: { value: 'https://lan.example/' },
            });
            await fireEvent.click(getByText(button));
            await flushIpc();

            expect(toastStore.toasts.at(-1)?.tone).toBe('error');
            expect(toastStore.toasts.at(-1)?.message).toContain('lan.example');
            expect(getByText('Couldn’t reach that address to check it for a feed.')).toBeTruthy();
            expect(getByTestId('add-feed-unvalidated')).toBeTruthy();
        },
    );

    it('offers the escape hatch when Find comes back empty, and clears it on edit', async () => {
        harness = installIpcHarness(base({ discover_feeds: discoveryFixture({ feeds: [] }) }));
        const { getByText, queryByTestId, getByPlaceholderText } = render(AddFeedModal, {
            onclose: vi.fn(),
        });
        const input = getByPlaceholderText('https://example.com');

        await fireEvent.input(input, { target: { value: 'https://plain.example' } });
        await fireEvent.click(getByText('Find feeds'));
        await flushIpc();
        expect(queryByTestId('add-feed-unvalidated')).toBeTruthy();

        await fireEvent.input(input, { target: { value: 'https://other.example' } });
        expect(queryByTestId('add-feed-unvalidated')).toBeNull();
    });

    it('drops a late subscribe refusal once the URL was edited meanwhile', async () => {
        let refuse: (error: unknown) => void = () => {};
        harness = installIpcHarness(
            base({
                add_feed: () =>
                    new Promise((_resolve, reject) => {
                        refuse = reject;
                    }),
            }),
        );
        const { getByText, queryByTestId, queryByText, getByPlaceholderText } = render(
            AddFeedModal,
            { onclose: vi.fn() },
        );
        const input = getByPlaceholderText('https://example.com');

        await fireEvent.input(input, { target: { value: 'https://first.example' } });
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();
        // The validating subscribe is still in flight: the user edits the URL.
        await fireEvent.input(input, { target: { value: 'https://second.example' } });
        refuse(
            commandErrorFixture({
                kind: 'user',
                code: 'NO_FEED',
                message: 'no feed found at https://first.example',
            }),
        );
        await flushIpc();

        // The refusal was about the first URL: no escape hatch for the second.
        expect(queryByText('No feeds found on that page — try the direct feed URL.')).toBeNull();
        expect(queryByTestId('add-feed-unvalidated')).toBeNull();
    });

    it('drops a late unreachable discovery once the URL was edited meanwhile', async () => {
        let refuse: (error: unknown) => void = () => {};
        harness = installIpcHarness(
            base({
                discover_feeds: () =>
                    new Promise((_resolve, reject) => {
                        refuse = reject;
                    }),
            }),
        );
        const { getByText, queryByTestId, getByPlaceholderText } = render(AddFeedModal, {
            onclose: vi.fn(),
        });
        const input = getByPlaceholderText('https://example.com');

        await fireEvent.input(input, { target: { value: 'https://first.example' } });
        await fireEvent.click(getByText('Find feeds'));
        await flushIpc();
        await fireEvent.input(input, { target: { value: 'https://second.example' } });
        refuse(
            commandErrorFixture({
                kind: 'user',
                code: 'NETWORK',
                message: 'could not reach https://first.example',
            }),
        );
        await flushIpc();

        expect(queryByTestId('add-feed-unvalidated')).toBeNull();
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('warns when the typed address resolves to a feed already subscribed', async () => {
        harness = installIpcHarness(
            base({
                add_feed: rejectWith(
                    commandErrorFixture({
                        kind: 'user',
                        code: 'ALREADY_SUBSCRIBED',
                        message: 'already subscribed to https://example.org/feed.xml',
                    }),
                ),
            }),
        );
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'example.org' },
        });
        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();

        expect(toastStore.toasts.at(-1)?.message).toBe('You are already subscribed to this feed.');
        expect(toastStore.toasts.at(-1)?.tone).toBe('warning');
    });

    it('falls back to the opted-in Google favicon for a scheme-less address', async () => {
        prefsStore.allowRemoteFavicon = true;
        harness = installIpcHarness(
            base({
                discover_feeds: discoveryFixture({ favicon: 'https://example.com/favicon.ico' }),
                get_cached_image: (args) =>
                    String(args.url).startsWith('https://www.google.com/')
                        ? '/cache/google.png'
                        : Promise.reject(
                              commandErrorFixture({
                                  kind: 'user',
                                  code: 'NETWORK',
                                  message: '404',
                              }),
                          ),
            }),
        );
        const { getByText, getByPlaceholderText } = render(AddFeedModal, { onclose: vi.fn() });

        await fireEvent.input(getByPlaceholderText('https://example.com'), {
            target: { value: 'example.com' },
        });
        await fireEvent.click(getByText('Find feeds'));
        await flushIpc();
        await flushIpc();

        expect(harness.callsFor('get_cached_image').map((call) => call.url)).toContain(
            'https://www.google.com/s2/favicons?domain=example.com&sz=64',
        );
    });

    it('offers GitHub-releases and newsletter presets', async () => {
        harness = installIpcHarness(base());
        const { getByText, queryByText, getByPlaceholderText } = render(AddFeedModal, {
            onclose: vi.fn(),
        });
        const input = getByPlaceholderText('https://example.com') as HTMLInputElement;

        await fireEvent.click(getByText('GitHub releases'));
        expect(input.value).toBe('https://github.com/');
        await fireEvent.input(input, { target: { value: 'https://github.com/rust-lang/rust' } });
        expect(getByText('https://github.com/rust-lang/rust/releases.atom')).toBeTruthy();

        // The newsletter preset explains the email-to-feed bridge.
        expect(queryByText('Open Kill the Newsletter')).toBeNull();
        await fireEvent.click(getByText('Newsletter'));
        expect(getByText('Open Kill the Newsletter')).toBeTruthy();
        await fireEvent.input(input, { target: { value: 'abc123@kill-the-newsletter.com' } });
        expect(getByText('https://kill-the-newsletter.com/feeds/abc123.xml')).toBeTruthy();

        await fireEvent.click(getByText('Subscribe'));
        await flushIpc();
        expect(harness.callsFor('add_feed')).toEqual([
            {
                newFeed: {
                    url: 'https://kill-the-newsletter.com/feeds/abc123.xml',
                    title: null,
                    tags: ['Newsletters'],
                    validate: true,
                },
            },
        ]);
    });
});
