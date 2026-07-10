/**
 * Popover dismissal behaviour for a floating panel (`role="dialog"`): focuses
 * the node on mount so keyboard users land inside, and closes it on Escape or a
 * pointer press outside it. The `onclose` callback is the single source of
 * truth for the open flag — the action never mutates state itself.
 *
 * Listeners are capture-phase and window-level so a press anywhere (including
 * inside other capturing widgets) is caught. The toggle that opened the panel
 * can be excluded via `ignore` so its own click cleanly toggles closed instead
 * of the panel closing then immediately reopening.
 */
export interface DismissableOptions {
    /** Close the panel (flip the caller's open flag to false). */
    onclose: () => void;
    /** An element (e.g. the toggle button) that must not count as "outside". */
    ignore?: () => Element | null | undefined;
    /** Where focus returns on an Escape dismiss (not on outside-click). */
    returnFocus?: () => HTMLElement | null | undefined;
}

export function dismissable(node: HTMLElement, options: DismissableOptions) {
    let opts = options;

    // Focus after the current microtask so the node is mounted and reachable.
    queueMicrotask(() => {
        if (node.isConnected && !node.contains(document.activeElement)) {
            node.focus();
        }
    });

    function onKeydown(event: KeyboardEvent): void {
        if (event.key === 'Escape') {
            event.stopPropagation();
            opts.returnFocus?.()?.focus();
            opts.onclose();
        }
    }

    function onPointerDown(event: PointerEvent): void {
        const target = event.target as Node | null;
        if (target === null || node.contains(target)) {
            return;
        }
        const ignored = opts.ignore?.();
        if (ignored?.contains(target)) {
            return;
        }
        opts.onclose();
    }

    window.addEventListener('keydown', onKeydown, true);
    window.addEventListener('pointerdown', onPointerDown, true);

    return {
        update(next: DismissableOptions): void {
            opts = next;
        },
        destroy(): void {
            window.removeEventListener('keydown', onKeydown, true);
            window.removeEventListener('pointerdown', onPointerDown, true);
        },
    };
}
