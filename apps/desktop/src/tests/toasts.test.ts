/**
 * The toast outlet: the two permanent live regions, tones, manual dismiss,
 * auto-dismiss, action buttons, and pause-on-hover/focus.
 */
import Toasts from '$components/common/Toasts.svelte';
import { toastStore } from '$lib/state/toasts.svelte';
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { resetUiStores } from './ui-stores';

function liveRegion(container: HTMLElement, politeness: 'polite' | 'assertive'): HTMLElement {
    const region = container.querySelector<HTMLElement>(`[data-live-region="${politeness}"]`);
    if (region === null) {
        throw new Error(`no ${politeness} live region`);
    }
    return region;
}

describe('Toasts', () => {
    afterEach(() => {
        resetUiStores();
        vi.useRealTimers();
    });

    it('mounts both live regions, empty, before the first toast', () => {
        const { container } = render(Toasts);
        const polite = liveRegion(container, 'polite');
        const assertive = liveRegion(container, 'assertive');
        // Present from mount, so the first toast is an *addition* to an
        // existing region — the change a screen reader reliably announces.
        expect(polite.getAttribute('aria-live')).toBe('polite');
        expect(polite.getAttribute('role')).toBe('status');
        expect(assertive.getAttribute('aria-live')).toBe('assertive');
        expect(assertive.getAttribute('role')).toBe('alert');
        expect(polite.textContent).toBe('');
        expect(assertive.textContent).toBe('');
        expect(container.querySelector('.toast')).toBeNull();
    });

    it('announces errors assertively and everything else politely', async () => {
        const { container } = render(Toasts);
        toastStore.showToast('note saved', 'success', 0);
        toastStore.showToast('fetch failed', 'error', 0);
        await tick();

        expect(liveRegion(container, 'polite').textContent).toContain('note saved');
        expect(liveRegion(container, 'polite').textContent).not.toContain('fetch failed');
        expect(liveRegion(container, 'assertive').textContent).toContain('fetch failed');

        // The visible cards carry no live role of their own (no double read).
        const cards = [...container.querySelectorAll('.toast')];
        expect(cards.map((card) => card.getAttribute('role'))).toEqual([null, null]);
        expect(cards[0]?.className).toContain('toast-success');
        expect(cards[1]?.className).toContain('toast-error');
    });

    it('dismisses on click, taking its announcement with it', async () => {
        const { container, getByLabelText } = render(Toasts);
        toastStore.showToast('transient', 'info', 0);
        await tick();

        await fireEvent.click(getByLabelText('Dismiss'));
        expect(container.querySelector('.toast')).toBeNull();
        expect(liveRegion(container, 'polite').textContent).toBe('');
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('auto-dismisses after its duration', async () => {
        vi.useFakeTimers();
        const { container } = render(Toasts);
        toastStore.showToast('brief', 'info', 100);
        await tick();
        expect(container.querySelector('.toast-message')?.textContent).toBe('brief');

        vi.advanceTimersByTime(150);
        await tick();
        expect(toastStore.toasts).toHaveLength(0);
        expect(container.querySelector('.toast')).toBeNull();
    });

    it('renders the action button; clicking it runs the action and closes the toast', async () => {
        const run = vi.fn();
        const { getByRole } = render(Toasts);
        toastStore.showToast('Marked 3 read', 'success', { action: { label: 'Undo', run } });
        await tick();

        await fireEvent.click(getByRole('button', { name: 'Undo' }));
        expect(run).toHaveBeenCalledOnce();
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('hovering the stack pauses dismissal; leaving resumes the time left', async () => {
        vi.useFakeTimers();
        const { container } = render(Toasts);
        toastStore.showToast('hold on', 'info', 1000);
        await tick();
        const stack = container.querySelector('.toasts') as HTMLElement;

        vi.advanceTimersByTime(600);
        await fireEvent.mouseEnter(stack);
        vi.advanceTimersByTime(60_000);
        expect(toastStore.toasts).toHaveLength(1);

        await fireEvent.mouseLeave(stack);
        vi.advanceTimersByTime(399);
        expect(toastStore.toasts).toHaveLength(1);
        vi.advanceTimersByTime(1);
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('keyboard focus inside the stack pauses dismissal too', async () => {
        vi.useFakeTimers();
        const { getByLabelText } = render(Toasts);
        toastStore.showToast('focus me', 'info', 1000);
        await tick();

        await fireEvent.focusIn(getByLabelText('Dismiss'));
        vi.advanceTimersByTime(5000);
        expect(toastStore.toasts).toHaveLength(1);

        await fireEvent.focusOut(getByLabelText('Dismiss'));
        vi.advanceTimersByTime(1000);
        expect(toastStore.toasts).toHaveLength(0);
    });

    it('pressing a focused Undo resumes the other timers even with no focusout', async () => {
        vi.useFakeTimers();
        const { getByRole } = render(Toasts);
        toastStore.showToast('Marked 2 read', 'success', {
            action: { label: 'Undo', run: vi.fn() },
        });
        toastStore.showToast('Saved', 'info', 1000);
        await tick();

        const undo = getByRole('button', { name: 'Undo' });
        await fireEvent.focusIn(undo);
        // WebKit may drop the focusout of a removed node: only the click lands.
        await fireEvent.click(undo);
        vi.advanceTimersByTime(1000);
        expect(toastStore.toasts).toHaveLength(0);
    });
});
