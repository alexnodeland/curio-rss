/**
 * The `tooltip` action: shows a styled tooltip on hover AND keyboard focus
 * (a11y), after a short delay, and wires `aria-describedby` from the trigger
 * to the single rendered tooltip element. Hidden for touch pointers, never
 * blocks pointer events (the tooltip element is `pointer-events: none`).
 * Replaces native `title=` on icon-only controls.
 */
import { tooltipStore } from '$lib/state/tooltip.svelte';

export interface TooltipOptions {
    text: string;
    /** Optional shortcut hint rendered as a <kbd> suffix, e.g. "⌘R". */
    kbd?: string;
}

export type TooltipParam = TooltipOptions | string;

const SHOW_DELAY_MS = 500;

function normalize(param: TooltipParam): TooltipOptions {
    return typeof param === 'string' ? { text: param } : param;
}

export function tooltip(
    node: HTMLElement,
    param: TooltipParam,
): { update(next: TooltipParam): void; destroy(): void } {
    let options = normalize(param);
    let timer: ReturnType<typeof setTimeout> | undefined;

    function reveal(): void {
        if (!options.text) return;
        tooltipStore.show(node, options.text, options.kbd, node.getBoundingClientRect());
        node.setAttribute('aria-describedby', tooltipStore.id);
    }

    function scheduleShow(): void {
        if (!options.text) return;
        clearTimeout(timer);
        timer = setTimeout(reveal, SHOW_DELAY_MS);
    }

    function hide(): void {
        clearTimeout(timer);
        if (node.getAttribute('aria-describedby') === tooltipStore.id) {
            node.removeAttribute('aria-describedby');
        }
        tooltipStore.hide(node);
    }

    function onPointerEnter(event: PointerEvent): void {
        if (event.pointerType === 'touch') return;
        scheduleShow();
    }

    // Keyboard focus reveals immediately (no hover intent to gauge).
    function onFocus(): void {
        reveal();
    }

    node.addEventListener('pointerenter', onPointerEnter);
    node.addEventListener('pointerleave', hide);
    node.addEventListener('pointerdown', hide);
    node.addEventListener('focus', onFocus);
    node.addEventListener('blur', hide);

    return {
        update(next: TooltipParam): void {
            options = normalize(next);
            // If this node owns the visible tooltip, refresh its text live.
            if (tooltipStore.owner === node && options.text) {
                tooltipStore.show(node, options.text, options.kbd, node.getBoundingClientRect());
            }
        },
        destroy(): void {
            clearTimeout(timer);
            hide();
            node.removeEventListener('pointerenter', onPointerEnter);
            node.removeEventListener('pointerleave', hide);
            node.removeEventListener('pointerdown', hide);
            node.removeEventListener('focus', onFocus);
            node.removeEventListener('blur', hide);
        },
    };
}
