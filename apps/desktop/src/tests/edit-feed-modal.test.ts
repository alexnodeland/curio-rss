/**
 * The edit-feed modal: the feed URL is visible and copyable; name, site URL,
 * description and folders/tags edit through their commands; the Health
 * section keeps the old feed-health surface (status controls, recent fetches,
 * 403/429 hints, mark-all-read); and unsubscribe stays a two-step confirm.
 */
import EditFeedModal from '$components/modals/EditFeedModal.svelte';
import { feedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
    type IpcHarness,
    feedFixture,
    fetchRecordFixture,
    flushIpc,
    installIpcHarness,
    unreadCountsFixture,
} from './ipc-harness';

// jsdom has no layout engine; the health-section scroll is a no-op there.
Element.prototype.scrollIntoView = vi.fn();

function harnessFor(status: 'active' | 'paused' | 'dead', overrides = {}): IpcHarness {
    return installIpcHarness({
        list_feeds: [
            feedFixture({
                id: 1,
                title: 'Alpha',
                url: 'https://alpha.example/feed.xml',
                tags: ['Tech'],
                status,
            }),
        ],
        get_unread_counts: unreadCountsFixture({ total: 0, by_feed: [] }),
        recent_fetches: [
            fetchRecordFixture({ status: 'ok', http_status: 200, articles_new: 3 }),
            fetchRecordFixture({
                status: 'error',
                http_status: 500,
                error: 'server error',
                articles_new: 0,
            }),
        ],
        set_feed_status: null,
        set_feed_title: null,
        set_feed_tags: null,
        set_feed_metadata: null,
        remove_feed: null,
        mark_all_read: 3,
        ...overrides,
    });
}

describe('EditFeedModal', () => {
    let harness: IpcHarness | null = null;

    beforeEach(() => {
        Object.defineProperty(navigator, 'clipboard', {
            value: { writeText: vi.fn().mockResolvedValue(undefined) },
            configurable: true,
        });
    });

    afterEach(() => {
        cleanup();
        resetQueryCache();
        feedsStore.refreshing = false;
        feedsStore.refreshOutcomes = [];
        uiStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('shows the feed URL and copies it to the clipboard', async () => {
        harness = harnessFor('active');
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        expect(getByText('https://alpha.example/feed.xml')).toBeTruthy();
        await fireEvent.click(getByText('Copy'));
        await flushIpc();

        expect(navigator.clipboard.writeText).toHaveBeenCalledWith(
            'https://alpha.example/feed.xml',
        );
        expect(uiStore.toasts.some((toast) => toast.message === 'Feed URL copied')).toBe(true);
    });

    it('renames a feed through set_feed_title', async () => {
        harness = harnessFor('active');
        const { getByLabelText, getByText } = render(EditFeedModal, {
            feedId: 1,
            onclose: vi.fn(),
        });
        await flushIpc();

        const input = getByLabelText('Name') as HTMLInputElement;
        expect(input.value).toBe('Alpha');
        await fireEvent.input(input, { target: { value: 'Company Blog' } });
        await fireEvent.click(getByText('Rename'));
        await flushIpc();

        expect(harness.callsFor('set_feed_title')).toEqual([{ feedId: 1, title: 'Company Blog' }]);
    });

    it('overwrites site URL and description through set_feed_metadata', async () => {
        harness = harnessFor('active');
        const { getByLabelText, getByText } = render(EditFeedModal, {
            feedId: 1,
            onclose: vi.fn(),
        });
        await flushIpc();

        await fireEvent.input(getByLabelText('Site URL'), {
            target: { value: 'https://alpha.example' },
        });
        await fireEvent.input(getByLabelText('Description'), {
            target: { value: 'A hand-written note' },
        });
        await fireEvent.click(getByText('Save'));
        await flushIpc();

        expect(harness.callsFor('set_feed_metadata')).toEqual([
            { feedId: 1, siteUrl: 'https://alpha.example', description: 'A hand-written note' },
        ]);
    });

    it('adds and removes tags through the chip editor (set_feed_tags)', async () => {
        harness = harnessFor('active');
        const { getByText, getByLabelText, getByPlaceholderText } = render(EditFeedModal, {
            feedId: 1,
            onclose: vi.fn(),
        });
        await flushIpc();

        // The existing tag renders as a chip.
        expect(getByText('Tech')).toBeTruthy();

        // Adding appends to the live tag list.
        const input = getByPlaceholderText('Add a tag…');
        await fireEvent.input(input, { target: { value: 'News' } });
        await fireEvent.keyDown(input, { key: 'Enter' });
        await flushIpc();
        expect(harness.callsFor('set_feed_tags')).toEqual([{ feedId: 1, tags: ['Tech', 'News'] }]);

        // Removing a chip persists the remainder.
        await fireEvent.click(getByLabelText('Remove Tech'));
        await flushIpc();
        expect(harness.callsFor('set_feed_tags')).toContainEqual({ feedId: 1, tags: [] });
    });

    it('renders the recent fetches in the Health section', async () => {
        harness = harnessFor('active');
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        expect(harness.callsFor('recent_fetches')).toEqual([{ feedId: 1, limit: 20 }]);
        expect(getByText('3 new')).toBeTruthy();
        expect(getByText('server error')).toBeTruthy();
    });

    it('pauses an active feed through set_feed_status', async () => {
        harness = harnessFor('active');
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();
        await fireEvent.click(getByText('Pause'));
        await flushIpc();
        expect(harness.callsFor('set_feed_status')).toEqual([{ feedId: 1, status: 'paused' }]);
    });

    it('offers reactivation for a dead feed', async () => {
        harness = harnessFor('dead');
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();
        await fireEvent.click(getByText('Reactivate'));
        await flushIpc();
        expect(harness.callsFor('set_feed_status')).toEqual([{ feedId: 1, status: 'active' }]);
    });

    it('marks all read and opens scrolled to Health when asked', async () => {
        harness = harnessFor('active');
        const { getByText } = render(EditFeedModal, {
            feedId: 1,
            section: 'health',
            onclose: vi.fn(),
        });
        await flushIpc();

        expect(Element.prototype.scrollIntoView).toHaveBeenCalled();
        await fireEvent.click(getByText('Mark all read'));
        await flushIpc();
        expect(harness.callsFor('mark_all_read')).toEqual([{ feedId: 1 }]);
        expect(uiStore.toasts.some((toast) => toast.message === 'Marked 3 read')).toBe(true);
    });

    it('unsubscribes only after a two-step confirm, then closes', async () => {
        harness = harnessFor('active');
        const onclose = vi.fn();
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose });
        await flushIpc();

        await fireEvent.click(getByText('Unsubscribe'));
        expect(harness.callsFor('remove_feed')).toHaveLength(0);
        expect(onclose).not.toHaveBeenCalled();

        await fireEvent.click(getByText('Remove'));
        await flushIpc();
        expect(harness.callsFor('remove_feed')).toEqual([{ feedId: 1 }]);
        expect(onclose).toHaveBeenCalledTimes(1);
    });

    it('surfaces a hint when the latest fetch was refused (403)', async () => {
        harness = harnessFor('active', {
            recent_fetches: [
                fetchRecordFixture({ status: 'error', http_status: 403, error: 'HTTP 403' }),
            ],
        });
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();
        expect(getByText(/refused the request \(403\)/)).toBeTruthy();
    });
});
