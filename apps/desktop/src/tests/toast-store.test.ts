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
        const first = ui.showToast('offline', 'warning', 1000);
        vi.advanceTimersByTime(800);

        const again = ui.showToast('offline', 'warning', 1000);
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

    it('never merges an unkeyed toast carrying an action (each Undo is its own)', () => {
        const ui = new ToastStore();
        const first = ui.showToast('Marked 3 read', 'success', {
            action: { label: 'Undo', run: vi.fn() },
        });
        const second = ui.showToast('Marked 3 read', 'success', {
            action: { label: 'Undo', run: vi.fn() },
        });
        expect(second).not.toBe(first);
        expect(ui.toasts).toHaveLength(2);
    });

    it('a repeat failure under the same key merges, taking the newest retry', () => {
        const ui = new ToastStore();
        const newest = vi.fn();
        const retryA = ui.showToast('offline', 'error', {
            action: { label: 'Retry', run: vi.fn() },
            key: 'refresh:1',
        });
        const retryB = ui.showToast('offline', 'error', {
            action: { label: 'Retry', run: newest },
            key: 'refresh:1',
        });
        expect(retryB).toBe(retryA);
        expect(ui.assertiveAnnouncements).toHaveLength(1); // a merge is not re-announced
        ui.runAction(retryA);
        expect(newest).toHaveBeenCalledOnce();
    });

    it('the same text from a different action never swallows its Retry', () => {
        const ui = new ToastStore();
        const star = vi.fn();
        const refresh = vi.fn();
        const starToast = ui.showToast('Something went wrong', 'error', {
            action: { label: 'Retry', run: star },
            key: 'starred:7',
        });
        const refreshToast = ui.showToast('Something went wrong', 'error', {
            action: { label: 'Retry', run: refresh },
            key: 'refresh:3',
        });
        expect(refreshToast).not.toBe(starToast);
        expect(ui.toasts).toHaveLength(2);
        ui.runAction(starToast);
        ui.runAction(refreshToast);
        expect(star).toHaveBeenCalledOnce();
        expect(refresh).toHaveBeenCalledOnce();
    });

    it('a new failure under a key replaces the old one when the text differs', () => {
        const ui = new ToastStore();
        ui.showToast('offline', 'error', { key: 'refresh:1' });
        ui.showToast('feed gone', 'error', { key: 'refresh:1' });
        expect(ui.toasts.map((toast) => toast.message)).toEqual(['feed gone']);
        expect(ui.assertiveAnnouncements.map((line) => line.text)).toEqual(['feed gone']);
    });

    it('withdraw closes only the toasts under that key', () => {
        const ui = new ToastStore();
        ui.showToast('rename failed', 'error', { key: 'feed:1:title' });
        ui.showToast('move failed', 'error', { key: 'feed:1:folder' });
        ui.showToast('plain', 'info', 0);
        ui.withdraw('feed:1:title');
        expect(ui.toasts.map((toast) => toast.message)).toEqual(['move failed', 'plain']);
        ui.withdraw('nothing:here'); // no-op
        expect(ui.toasts).toHaveLength(2);
    });

    it('an error is never timed shorter than the action window', () => {
        const ui = new ToastStore();
        ui.showToast('theme import failed', 'error', 6000);
        vi.advanceTimersByTime(ACTION_TOAST_MS - 1);
        expect(ui.toasts).toHaveLength(1);
        vi.advanceTimersByTime(1);
        expect(ui.toasts).toHaveLength(0);
        // A longer explicit duration is honoured as given.
        ui.showToast('slow', 'error', { durationMs: ACTION_TOAST_MS + 5000 });
        vi.advanceTimersByTime(ACTION_TOAST_MS + 4999);
        expect(ui.toasts).toHaveLength(1);
    });

    it('a hold freezes every timer and its release restarts each with its time left', () => {
        const ui = new ToastStore();
        ui.showToast('one', 'info', 1000);
        vi.advanceTimersByTime(400);
        ui.hold('pointer');
        expect(ui.paused).toBe(true);
        // A toast queued while held waits too.
        ui.showToast('two', 'info', 500);
        vi.advanceTimersByTime(10_000);
        expect(ui.toasts).toHaveLength(2);

        ui.release('pointer');
        vi.advanceTimersByTime(500);
        expect(ui.toasts.map((toast) => toast.message)).toEqual(['one']);
        vi.advanceTimersByTime(100);
        expect(ui.toasts).toHaveLength(0);
    });

    it('pointer and focus hold independently: timers wait for both to leave', () => {
        const ui = new ToastStore();
        ui.showToast('one', 'info', 1000);
        ui.hold('pointer');
        ui.hold('focus');
        ui.release('pointer');
        vi.advanceTimersByTime(5000);
        expect(ui.toasts).toHaveLength(1); // focus still holds it
        ui.release('focus');
        vi.advanceTimersByTime(1000);
        expect(ui.toasts).toHaveLength(0);
    });

    it('closing a toast from its own button ends the focus hold (no focusout needed)', () => {
        const ui = new ToastStore();
        const undo = ui.showToast('Marked 2 read', 'success', {
            action: { label: 'Undo', run: vi.fn() },
        });
        ui.showToast('other', 'info', 1000);
        ui.hold('focus'); // keyboard focus on the Undo button
        ui.runAction(undo); // its toast (and the focused button) goes away
        expect(ui.paused).toBe(false);
        vi.advanceTimersByTime(1000);
        expect(ui.toasts).toHaveLength(0); // the other toast still times out

        const closer = ui.showToast('closable', 'info', 0);
        ui.showToast('timed', 'info', 1000);
        ui.hold('focus');
        ui.dismissToast(closer);
        vi.advanceTimersByTime(1000);
        expect(ui.toasts).toHaveLength(0);
    });

    it('an emptied stack drops every hold (its last node never fired mouseleave)', () => {
        const ui = new ToastStore();
        ui.showToast('only', 'info', 1000);
        ui.hold('pointer');
        ui.hold('focus');
        ui.settleAll();
        expect(ui.paused).toBe(false);
        ui.showToast('next', 'info', 1000);
        vi.advanceTimersByTime(1000);
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
