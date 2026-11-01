/**
 * The edit-feed modal: the feed URL is visible and copyable; name, site URL,
 * description and folders/tags edit through their commands; the Health
 * section keeps the old feed-health surface (status controls, recent fetches,
 * 403/429/rate-limit hints, the schedule note, mark-all-read); and unsubscribe stays a two-step confirm.
 */
import EditFeedModal from '$components/modals/EditFeedModal.svelte';
import { formatIntlDateTime } from '$lib/i18n';
import { feedsStore } from '$lib/state/feeds.svelte';
import { resetQueryCache } from '$lib/state/query-cache.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
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
import { resetUiStores } from './ui-stores';

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
        set_feed_full_text: null,
        remove_feed: null,
        mark_all_read: [11, 12, 13],
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
        resetUiStores();
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
        expect(toastStore.toasts.some((toast) => toast.message === 'Feed URL copied')).toBe(true);
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

    it('auto-saves the name on blur (no explicit Save click needed)', async () => {
        harness = harnessFor('active');
        const { getByLabelText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        const input = getByLabelText('Name') as HTMLInputElement;
        await fireEvent.input(input, { target: { value: 'Renamed On Blur' } });
        await fireEvent.blur(input);
        await flushIpc();

        expect(harness.callsFor('set_feed_title')).toEqual([
            { feedId: 1, title: 'Renamed On Blur' },
        ]);
    });

    it('flushes a dirty edit when the modal is closed', async () => {
        harness = harnessFor('active');
        const onclose = vi.fn();
        const { getByLabelText } = render(EditFeedModal, { feedId: 1, onclose });
        await flushIpc();

        // Edit without blurring or clicking Save, then close via the header X.
        await fireEvent.input(getByLabelText('Description'), {
            target: { value: 'unsaved note' },
        });
        await fireEvent.click(getByLabelText('Close'));
        await flushIpc();

        expect(harness.callsFor('set_feed_metadata')).toContainEqual(
            expect.objectContaining({ feedId: 1, description: 'unsaved note' }),
        );
        expect(onclose).toHaveBeenCalled();
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

    it('flips full-text mode through set_feed_full_text', async () => {
        harness = harnessFor('active');
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();

        const toggle = getByText('Fetch full text')
            .closest('label')
            ?.querySelector('input') as HTMLInputElement;
        expect(toggle.checked).toBe(false);
        await fireEvent.click(toggle);
        await flushIpc();

        expect(harness.callsFor('set_feed_full_text')).toEqual([{ feedId: 1, enabled: true }]);
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
        expect(toastStore.toasts.some((toast) => toast.message === 'Marked 3 read')).toBe(true);
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

    it('explains a 503 with Retry-After as a rate limit too', async () => {
        harness = harnessFor('active', {
            recent_fetches: [
                fetchRecordFixture({
                    status: 'error',
                    http_status: 503,
                    error: 'HTTP 503 — the host is rate-limiting Curio (Retry-After: 120 s)',
                    error_kind: 'rate_limited',
                }),
            ],
        });
        const { getByText } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();
        expect(getByText(/The site is rate-limiting requests\./)).toBeTruthy();
    });

    it('drops the next-attempt note once that time has passed', async () => {
        // Fake clock (still ticking, so the IPC flush resolves): the modal
        // stays open past the deadline without the test waiting it out.
        vi.useFakeTimers({ shouldAdvanceTime: true });
        try {
            harness = harnessFor('active', {
                list_feeds: [
                    feedFixture({
                        id: 1,
                        title: 'Alpha',
                        consecutive_failures: 1,
                        next_attempt_at: new Date(Date.now() + 10 * 60_000).toISOString(),
                    }),
                ],
            });
            const { container } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
            await flushIpc();
            expect(container.querySelector('.schedule')).not.toBeNull();
            vi.advanceTimersByTime(9 * 60_000);
            await flushIpc();
            expect(container.querySelector('.schedule')).not.toBeNull();
            vi.advanceTimersByTime(2 * 60_000);
            await flushIpc();
            expect(container.querySelector('.schedule')).toBeNull();
        } finally {
            vi.useRealTimers();
        }
    });

    it('shows when a failing feed is next attempted', async () => {
        const nextAttempt = new Date(Date.now() + 45 * 60_000);
        harness = harnessFor('active', {
            list_feeds: [
                feedFixture({
                    id: 1,
                    title: 'Alpha',
                    consecutive_failures: 3,
                    next_attempt_at: nextAttempt.toISOString(),
                }),
            ],
        });
        const { container } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();
        const note = container.querySelector('.schedule');
        expect(note?.textContent).toContain('3 failed fetches in a row — next attempt');
        expect(note?.textContent).toContain(formatIntlDateTime(nextAttempt));
    });

    it('explains an automatic pause and resumes with one click', async () => {
        harness = harnessFor('paused', {
            list_feeds: [
                feedFixture({
                    id: 1,
                    title: 'Alpha',
                    status: 'paused',
                    consecutive_failures: 10,
                    auto_paused_at: '2026-07-05T22:10:00.000Z',
                }),
            ],
        });
        const { container, getByText } = render(EditFeedModal, {
            feedId: 1,
            onclose: vi.fn(),
        });
        await flushIpc();
        expect(container.querySelector('.schedule')?.textContent).toBe(
            'Curio paused this feed after 10 failed fetches in a row. Resume to try again.',
        );
        await fireEvent.click(getByText('Resume'));
        await flushIpc();
        expect(harness.callsFor('set_feed_status')).toEqual([{ feedId: 1, status: 'active' }]);
    });

    it('says nothing about the schedule of a healthy feed', async () => {
        harness = harnessFor('active');
        const { container } = render(EditFeedModal, { feedId: 1, onclose: vi.fn() });
        await flushIpc();
        expect(container.querySelector('.schedule')).toBeNull();
    });
});
