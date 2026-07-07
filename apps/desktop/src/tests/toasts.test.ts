/**
 * The toast outlet: rendering, tones, manual dismiss, auto-dismiss, and the
 * polite live region.
 */
import Toasts from '$components/common/Toasts.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';

describe('Toasts', () => {
    afterEach(() => {
        uiStore.reset();
        vi.useRealTimers();
    });

    it('announces errors assertively and everything else politely', async () => {
        const { getByText } = render(Toasts);
        uiStore.showToast('note saved', 'success', 0);
        uiStore.showToast('fetch failed', 'error', 0);
        await tick();

        const saved = getByText('note saved').closest('.toast');
        const failed = getByText('fetch failed').closest('.toast');
        // role="status" is an implicit polite live region; role="alert" is
        // assertive — an error interrupts, a success waits its turn.
        expect(saved?.getAttribute('role')).toBe('status');
        expect(saved?.className).toContain('toast-success');
        expect(failed?.getAttribute('role')).toBe('alert');
        expect(failed?.className).toContain('toast-error');
    });

    it('dismisses on click', async () => {
        const { getByLabelText, queryByText } = render(Toasts);
        uiStore.showToast('transient', 'info', 0);
        await tick();

        await fireEvent.click(getByLabelText('Dismiss'));
        expect(queryByText('transient')).toBeNull();
        expect(uiStore.toasts).toHaveLength(0);
    });

    it('auto-dismisses after its duration', async () => {
        vi.useFakeTimers();
        const { queryByText } = render(Toasts);
        uiStore.showToast('brief', 'info', 100);
        await tick();
        expect(queryByText('brief')).not.toBeNull();

        vi.advanceTimersByTime(150);
        await tick();
        expect(uiStore.toasts).toHaveLength(0);
        expect(queryByText('brief')).toBeNull();
    });
});
