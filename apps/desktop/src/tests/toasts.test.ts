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

    it('renders queued toasts with their tone and a polite live region', async () => {
        const { container, getByText } = render(Toasts);
        uiStore.showToast('note saved', 'success', 0);
        uiStore.showToast('fetch failed', 'error', 0);
        await tick();

        expect(container.querySelector('[aria-live="polite"]')).not.toBeNull();
        expect(getByText('note saved').closest('.toast')?.className).toContain('toast-success');
        expect(getByText('fetch failed').closest('.toast')?.className).toContain('toast-error');
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
