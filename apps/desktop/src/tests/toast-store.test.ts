/**
 * Toast store: auto-dismiss timers, sticky toasts, de-duplication, the
 * on-screen cap, and reset. The rendered outlet lives in toasts.test.ts.
 */
import {
    ACTION_TOAST_MS,
    MAX_TOASTS,
    MAX_TOAST_DURATION_MS,
    MIN_TOAST_DURATION_MS,
    ToastStore,
    defaultToastDuration,
} from '$lib/state/toasts.svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

describe('toast store', () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('auto-dismisses after the duration', () => {
        const ui = new ToastStore();
        ui.showToast('saved', 'success', 1000);
        expect(ui.toasts).toHaveLength(1);
        expect(ui.toasts[0]).toMatchObject({ message: 'saved', tone: 'success' });

        vi.advanceTimersByTime(999);
        expect(ui.toasts).toHaveLength(1);
        vi.advanceTimersByTime(1);
        expect(ui.toasts).toHaveLength(0);
    });

    it('duration 0 sticks until dismissed by hand', () => {
        const ui = new ToastStore();
        const id = ui.showToast('stuck', 'error', 0);
        vi.advanceTimersByTime(60_000);
        expect(ui.toasts).toHaveLength(1);

        ui.dismissToast(id);
        expect(ui.toasts).toHaveLength(0);
    });

    it('dismissing one toast leaves the others (and clears its timer)', () => {
        const ui = new ToastStore();
        const first = ui.showToast('one');
        ui.showToast('two');

        ui.dismissToast(first);
        expect(ui.toasts.map((toast) => toast.message)).toEqual(['two']);

        vi.runAllTimers();
        expect(ui.toasts).toHaveLength(0);
    });

    it('an identical message + tone refreshes the existing toast instead of stacking', () => {
        const ui = new ToastStore();
        const first = ui.showToast('offline', 'error', 1000);
        vi.advanceTimersByTime(800);

        const again = ui.showToast('offline', 'error', 1000);
        expect(again).toBe(first);
        expect(ui.toasts).toHaveLength(1);

        // The timer was re-armed: the original deadline passes, the toast stays.
        vi.advanceTimersByTime(800);
        expect(ui.toasts).toHaveLength(1);
        vi.advanceTimersByTime(200);
        expect(ui.toasts).toHaveLength(0);

        // Same message, different tone → a distinct toast.
        ui.showToast('offline', 'error', 0);
        ui.showToast('offline', 'warning', 0);
        expect(ui.toasts).toHaveLength(2);
    });

    it('caps the stack at MAX_TOASTS, dropping the oldest', () => {
        const ui = new ToastStore();
        for (let index = 0; index < MAX_TOASTS + 2; index += 1) {
            ui.showToast(`toast ${index}`, 'info', 1000);
        }
        expect(ui.toasts).toHaveLength(MAX_TOASTS);
        expect(ui.toasts[0].message).toBe('toast 2');
        expect(ui.toasts.at(-1)?.message).toBe(`toast ${MAX_TOASTS + 1}`);

        vi.runAllTimers();
        expect(ui.toasts).toHaveLength(0);
    });

    it('errors stay until dismissed by default (well past 8 s)', () => {
        const ui = new ToastStore();
        ui.showToast('feed failed', 'error');
        vi.advanceTimersByTime(60_000);
        expect(ui.toasts).toHaveLength(1);
    });

    it('a plain toast lives longer the longer its text, within the bounds', () => {
        expect(defaultToastDuration('ok', 'info', false)).toBe(MIN_TOAST_DURATION_MS + 100);
        expect(defaultToastDuration('x'.repeat(1000), 'info', false)).toBe(MAX_TOAST_DURATION_MS);
        // An action guarantees the full undo/retry window.
        expect(defaultToastDuration('ok', 'success', true)).toBe(ACTION_TOAST_MS);
        expect(defaultToastDuration('anything', 'error', true)).toBe(0);
    });

    it('onClose reports why the toast went away, exactly once', () => {
        const ui = new ToastStore();
        const reasons: string[] = [];
        const onClose = (reason: string): void => {
            reasons.push(reason);
        };
        const run = vi.fn();
        const timed = ui.showToast('a', 'info', { durationMs: 100, onClose });
        const clicked = ui.showToast('b', 'info', { action: { label: 'Undo', run }, onClose });
        const closed = ui.showToast('c', 'info', { durationMs: 0, onClose });

        vi.advanceTimersByTime(100);
        ui.runAction(clicked);
        ui.dismissToast(closed);
        ui.dismissToast(timed); // already gone — no second call
        expect(reasons).toEqual(['timeout', 'action', 'dismiss']);
        expect(run).toHaveBeenCalledOnce();
    });

    it('a toast pushed off the capped stack closes as evicted', () => {
        const ui = new ToastStore();
        const onClose = vi.fn();
        ui.showToast('first', 'info', { durationMs: 0, onClose });
        for (let index = 0; index < MAX_TOASTS; index += 1) {
            ui.showToast(`later ${index}`, 'info', 0);
        }
        expect(onClose).toHaveBeenCalledWith('evicted');
    });

    it('settleAll closes everything as dismissed (deferred commits go ahead)', () => {
        const ui = new ToastStore();
        const onClose = vi.fn();
        ui.showToast('pending', 'info', { action: { label: 'Undo', run: vi.fn() }, onClose });
        ui.settleAll();
        expect(onClose).toHaveBeenCalledWith('dismiss');
        expect(ui.toasts).toHaveLength(0);
    });

    it('never merges toasts carrying an Undo; merges an opted-in Retry', () => {
        const ui = new ToastStore();
        const first = ui.showToast('Marked 3 read', 'success', {
            action: { label: 'Undo', run: vi.fn() },
        });
        const second = ui.showToast('Marked 3 read', 'success', {
            action: { label: 'Undo', run: vi.fn() },
        });
        expect(second).not.toBe(first);
        expect(ui.toasts).toHaveLength(2);

        const newest = vi.fn();
        const retryA = ui.showToast('offline', 'error', {
            action: { label: 'Retry', run: vi.fn() },
            dedupe: true,
        });
        const retryB = ui.showToast('offline', 'error', {
            action: { label: 'Retry', run: newest },
            dedupe: true,
        });
        expect(retryB).toBe(retryA);
        ui.runAction(retryA);
        expect(newest).toHaveBeenCalledOnce(); // the merged toast took the newest retry
    });

    it('pause holds every timer and resume restarts each with its time left', () => {
        const ui = new ToastStore();
        ui.showToast('one', 'info', 1000);
        vi.advanceTimersByTime(400);
        ui.pause();
        // A toast queued while paused waits too.
        ui.showToast('two', 'info', 500);
        vi.advanceTimersByTime(10_000);
        expect(ui.toasts).toHaveLength(2);

        ui.resume();
        vi.advanceTimersByTime(500);
        expect(ui.toasts.map((toast) => toast.message)).toEqual(['one']);
        vi.advanceTimersByTime(100);
        expect(ui.toasts).toHaveLength(0);
    });

    it('announces each toast into the matching live region', () => {
        const ui = new ToastStore();
        ui.showToast('saved', 'success', 0);
        ui.showToast('failed', 'error', 0);
        ui.announce('standalone');
        expect(ui.politeAnnouncements.map((line) => line.text)).toEqual(['saved', 'standalone']);
        expect(ui.assertiveAnnouncements.map((line) => line.text)).toEqual(['failed']);
        // A repeat is a new node (so it is read again), capped per region.
        for (let index = 0; index < 5; index += 1) ui.announce('again');
        expect(ui.politeAnnouncements).toHaveLength(3);
        expect(new Set(ui.politeAnnouncements.map((line) => line.id)).size).toBe(3);
    });

    it('reset clears every toast and its pending timer', () => {
        const ui = new ToastStore();
        ui.showToast('one', 'info', 1000);
        ui.showToast('two', 'info', 0);

        ui.reset();
        expect(ui.toasts).toEqual([]);
        expect(vi.getTimerCount()).toBe(0);
    });
});
