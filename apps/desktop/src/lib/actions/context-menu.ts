/**
 * The `contextMenu` action: right-click (and the keyboard ContextMenu key /
 * Shift+F10) opens a positioned menu at the pointer or the element, driven by
 * the `menuStore` singleton. Items are supplied by a getter so they reflect
 * current state at open time. Pair with `role`-bearing rows; the menu itself
 * carries the ARIA menu semantics.
 */
import { type MenuItem, menuStore } from '$lib/state/menu.svelte';

export interface ContextMenuOptions {
    /** Computes the menu descriptors at open time. Empty ⇒ no menu opens. */
    items: () => readonly MenuItem[];
    /** Accessible label for the menu container. */
    ariaLabel?: string;
    /** When true, the action is inert (no listeners fire). */
    disabled?: boolean;
}

export type ContextMenuParam = ContextMenuOptions | (() => readonly MenuItem[]);

function normalize(param: ContextMenuParam): ContextMenuOptions {
    return typeof param === 'function' ? { items: param } : param;
}

export function contextMenu(
    node: HTMLElement,
    param: ContextMenuParam,
): { update(next: ContextMenuParam): void; destroy(): void } {
    let options = normalize(param);

    function open(x: number, y: number): void {
        if (options.disabled === true) return;
        const items = options.items();
        if (items.length === 0) return;
        menuStore.openAt(items, x, y, node, options.ariaLabel);
    }

    function onContextMenu(event: MouseEvent): void {
        if (options.disabled === true) return;
        event.preventDefault();
        event.stopPropagation();
        open(event.clientX, event.clientY);
    }

    function onKeydown(event: KeyboardEvent): void {
        const isMenuKey = event.key === 'ContextMenu';
        const isShiftF10 = event.shiftKey && event.key === 'F10';
        if (!isMenuKey && !isShiftF10) return;
        event.preventDefault();
        event.stopPropagation();
        const rect = node.getBoundingClientRect();
        open(rect.left + 8, rect.bottom - 4);
    }

    node.addEventListener('contextmenu', onContextMenu);
    node.addEventListener('keydown', onKeydown);

    return {
        update(next: ContextMenuParam): void {
            options = normalize(next);
        },
        destroy(): void {
            node.removeEventListener('contextmenu', onContextMenu);
            node.removeEventListener('keydown', onKeydown);
        },
    };
}
