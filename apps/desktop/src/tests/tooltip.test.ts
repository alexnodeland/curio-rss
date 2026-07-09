/**
 * The tooltip primitive: the action reveals on hover (mouse) and focus, wires
 * aria-describedby to the single rendered tooltip element, hides on
 * leave/blur, and stays hidden for touch pointers.
 */
import TooltipHost from '$components/common/TooltipHost.svelte';
import { tooltip } from '$lib/actions/tooltip';
import { tooltipStore } from '$lib/state/tooltip.svelte';
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

beforeEach(() => {
    vi.useFakeTimers();
    tooltipStore.reset();
});

afterEach(() => {
    vi.runOnlyPendingTimers();
    vi.useRealTimers();
    tooltipStore.reset();
});

function trigger(): { node: HTMLElement; destroy: () => void } {
    const node = document.createElement('button');
    document.body.appendChild(node);
    const action = tooltip(node, { text: 'Add feed', kbd: 'A' });
    return {
        node,
        destroy: () => {
            action.destroy();
            node.remove();
        },
    };
}

describe('tooltip action', () => {
    it('reveals after the hover delay and wires aria-describedby', async () => {
        const { node, destroy } = trigger();
        await fireEvent.pointerEnter(node, { pointerType: 'mouse' });
        // Not yet — the delay has not elapsed.
        expect(tooltipStore.current).toBeNull();
        vi.advanceTimersByTime(600);
        expect(tooltipStore.current?.text).toBe('Add feed');
        expect(tooltipStore.current?.kbd).toBe('A');
        expect(node.getAttribute('aria-describedby')).toBe(tooltipStore.id);
        destroy();
    });

    it('reveals immediately on keyboard focus (no delay)', async () => {
        const { node, destroy } = trigger();
        await fireEvent.focus(node);
        expect(tooltipStore.current?.text).toBe('Add feed');
        destroy();
    });

    it('hides on pointer leave and clears aria-describedby', async () => {
        const { node, destroy } = trigger();
        await fireEvent.focus(node);
        expect(tooltipStore.current).not.toBeNull();
        await fireEvent.blur(node);
        expect(tooltipStore.current).toBeNull();
        expect(node.hasAttribute('aria-describedby')).toBe(false);
        destroy();
    });

    it('stays hidden for touch pointers', async () => {
        const { node, destroy } = trigger();
        // jsdom drops pointerType off synthetic events, so dispatch one that
        // carries it explicitly (mirrors a real touch pointerenter).
        const event = new Event('pointerenter', { bubbles: true });
        Object.defineProperty(event, 'pointerType', { value: 'touch' });
        node.dispatchEvent(event);
        vi.advanceTimersByTime(600);
        expect(tooltipStore.current).toBeNull();
        destroy();
    });

    it('renders through TooltipHost with role=tooltip and a kbd suffix', async () => {
        render(TooltipHost);
        const { node, destroy } = trigger();
        await fireEvent.focus(node);
        await tick();
        const tip = document.getElementById(tooltipStore.id);
        expect(tip?.getAttribute('role')).toBe('tooltip');
        expect(tip?.querySelector('.tooltip-kbd')?.textContent).toBe('A');
        destroy();
    });
});
