/**
 * The menu/context-menu singleton. One menu is open at a time — mirroring the
 * `ModalKind` discipline in ui.svelte.ts — so a module-level store owns the
 * open menu's descriptors, its anchor position, and the element that invoked
 * it (focus returns there on dismissal). Menu content is DATA (`MenuItem[]`),
 * not markup, so the same descriptors drive context menus (WP7), the native
 * menu bar (WP11), and dropdown buttons.
 */
import type { IconName } from '$components/common/Icon.svelte';
import type { MessageKey } from '$lib/i18n';

/**
 * One menu entry. Either `label` (already resolved) or `labelKey` (routed
 * through `t()` with optional `labelParams`) supplies the text. Items with
 * `children` open a submenu on Right/hover; leaf items run `onSelect` and
 * close the whole menu. `separatorBefore` draws a divider above the item.
 */
export interface MenuItem {
    readonly id: string;
    readonly label?: string;
    readonly labelKey?: MessageKey;
    readonly labelParams?: Record<string, string | number>;
    readonly icon?: IconName;
    /** Shortcut hint shown right-aligned, e.g. "⌘R". Display only. */
    readonly kbd?: string;
    readonly danger?: boolean;
    readonly disabled?: boolean;
    readonly separatorBefore?: boolean;
    readonly children?: readonly MenuItem[];
    readonly onSelect?: () => void;
}

/** How the top-level menu anchors: at the pointer, or below the invoker. */
export type MenuAnchor = 'pointer' | 'below';

export interface OpenMenu {
    readonly items: readonly MenuItem[];
    /** Viewport-relative coordinates for the menu's top-left (pre-flip). */
    readonly x: number;
    readonly y: number;
    readonly invoker: HTMLElement | null;
    readonly anchor: MenuAnchor;
    /** Optional accessible label for the menu container. */
    readonly ariaLabel?: string;
}

class MenuStore {
    current: OpenMenu | null = $state(null);

    get isOpen(): boolean {
        return this.current !== null;
    }

    /** Opens a menu at an explicit viewport point (context-menu style). */
    openAt(
        items: readonly MenuItem[],
        x: number,
        y: number,
        invoker: HTMLElement | null,
        ariaLabel?: string,
    ): void {
        this.current = { items, x, y, invoker, anchor: 'pointer', ariaLabel };
    }

    /** Opens a menu flush under an element (dropdown-button style). */
    openBelow(items: readonly MenuItem[], el: HTMLElement, ariaLabel?: string): void {
        const rect = el.getBoundingClientRect();
        this.current = {
            items,
            x: rect.left,
            y: rect.bottom + 4,
            invoker: el,
            anchor: 'below',
            ariaLabel,
        };
    }

    /**
     * Closes the menu. When `restoreFocus`, focus returns to the invoking
     * element — the default for Escape/outside dismissal. Item activation
     * passes `false` and lets the selected action own focus (e.g. a modal it
     * opens), which mounts after this and wins the focus.
     */
    close(restoreFocus = true): void {
        const invoker = this.current?.invoker ?? null;
        this.current = null;
        if (restoreFocus && invoker?.isConnected) {
            invoker.focus();
        }
    }

    /** Test isolation. */
    reset(): void {
        this.current = null;
    }
}

/** The app-wide singleton. */
export const menuStore = new MenuStore();
