/**
 * Toast store: auto-dismiss timers, sticky toasts, de-duplication, the
 * on-screen cap, and reset. The rendered outlet lives in toasts.test.ts.
 */
import { MAX_TOASTS, ToastStore } from '$lib/state/toasts.svelte';
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

    it('reset clears every toast and its pending timer', () => {
        const ui = new ToastStore();
        ui.showToast('one', 'info', 1000);
        ui.showToast('two', 'info', 0);

        ui.reset();
        expect(ui.toasts).toEqual([]);
        expect(vi.getTimerCount()).toBe(0);
    });
});
