/**
 * The menu primitive: opening via the store, roving keyboard focus,
 * activation, dismissal, submenu open/close, and the contextMenu action's
 * pointer + keyboard triggers. a11y follows the existing focus/listbox
 * precedents — role="menu"/menuitem, one open at a time.
 */
import Menu from '$components/common/Menu.svelte';
import MenuHost from '$components/common/MenuHost.svelte';
import { contextMenu } from '$lib/actions/context-menu';
import { type MenuItem, menuStore } from '$lib/state/menu.svelte';
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

function items(overrides: Partial<MenuItem>[] = []): MenuItem[] {
    const base: MenuItem[] = [
        { id: 'open', label: 'Open', onSelect: vi.fn() },
        { id: 'rename', label: 'Rename', onSelect: vi.fn() },
        { id: 'delete', label: 'Delete', danger: true, onSelect: vi.fn() },
    ];
    return base.map((item, i) => ({ ...item, ...overrides[i] }));
}

beforeEach(() => {
    menuStore.reset();
});

afterEach(() => {
    menuStore.reset();
});

describe('Menu (via MenuHost + menuStore)', () => {
    it('opens from the store and renders every item as a menuitem', async () => {
        render(MenuHost);
        menuStore.openAt(items(), 20, 20, null, 'Feed actions');
        await tick();

        const menu = document.querySelector('[role="menu"]');
        expect(menu).not.toBeNull();
        expect(menu?.getAttribute('aria-label')).toBe('Feed actions');
        const menuitems = document.querySelectorAll('[role="menuitem"]');
        expect(menuitems.length).toBe(3);
        expect(menuitems[0].textContent).toContain('Open');
    });

    it('focuses the first item on open', async () => {
        render(MenuHost);
        menuStore.openAt(items(), 20, 20, null);
        await tick();
        await tick();
        const first = document.querySelectorAll('[role="menuitem"]')[0];
        expect(document.activeElement).toBe(first);
    });

    it('ArrowDown/ArrowUp move roving focus and wrap', async () => {
        render(MenuHost);
        menuStore.openAt(items(), 20, 20, null);
        await tick();
        await tick();
        const menu = document.querySelector('[role="menu"]') as HTMLElement;
        const rows = document.querySelectorAll('[role="menuitem"]');

        await fireEvent.keyDown(menu, { key: 'ArrowDown' });
        expect(document.activeElement).toBe(rows[1]);
        await fireEvent.keyDown(menu, { key: 'ArrowUp' });
        expect(document.activeElement).toBe(rows[0]);
        // Up off the first wraps to the last.
        await fireEvent.keyDown(menu, { key: 'ArrowUp' });
        expect(document.activeElement).toBe(rows[2]);
    });

    it('Enter activates the focused item and closes the menu', async () => {
        const rows = items();
        render(MenuHost);
        menuStore.openAt(rows, 20, 20, null);
        await tick();
        await tick();
        const menu = document.querySelector('[role="menu"]') as HTMLElement;
        await fireEvent.keyDown(menu, { key: 'Enter' });
        expect(rows[0].onSelect).toHaveBeenCalledOnce();
        expect(menuStore.isOpen).toBe(false);
    });

    it('Escape dismisses without activating', async () => {
        const rows = items();
        render(MenuHost);
        menuStore.openAt(rows, 20, 20, null);
        await tick();
        const menu = document.querySelector('[role="menu"]') as HTMLElement;
        await fireEvent.keyDown(menu, { key: 'Escape' });
        expect(menuStore.isOpen).toBe(false);
        expect(rows[0].onSelect).not.toHaveBeenCalled();
    });

    it('skips disabled items during navigation', async () => {
        const rows = items([{}, { disabled: true }]);
        render(MenuHost);
        menuStore.openAt(rows, 20, 20, null);
        await tick();
        await tick();
        const menu = document.querySelector('[role="menu"]') as HTMLElement;
        const menuitems = document.querySelectorAll('[role="menuitem"]');
        await fireEvent.keyDown(menu, { key: 'ArrowDown' });
        // index 1 is disabled → focus lands on index 2.
        expect(document.activeElement).toBe(menuitems[2]);
    });

    it('opens a submenu with ArrowRight and marks aria-haspopup/expanded', async () => {
        const rows: MenuItem[] = [
            {
                id: 'move',
                label: 'Move to folder',
                children: [
                    { id: 'a', label: 'Folder A', onSelect: vi.fn() },
                    { id: 'b', label: 'Folder B', onSelect: vi.fn() },
                ],
            },
            { id: 'x', label: 'Other', onSelect: vi.fn() },
        ];
        render(MenuHost);
        menuStore.openAt(rows, 20, 20, null);
        await tick();
        await tick();
        const parentItem = document.querySelectorAll('[role="menuitem"]')[0];
        expect(parentItem.getAttribute('aria-haspopup')).toBe('menu');

        const menu = document.querySelector('[role="menu"]') as HTMLElement;
        await fireEvent.keyDown(menu, { key: 'ArrowRight' });
        await tick();
        expect(document.querySelectorAll('[role="menu"]').length).toBe(2);
        expect(parentItem.getAttribute('aria-expanded')).toBe('true');
    });
});

describe('contextMenu action', () => {
    it('opens the menu on a right-click at the pointer', async () => {
        menuStore.reset();
        const node = document.createElement('button');
        document.body.appendChild(node);
        const rows = items();
        const action = contextMenu(node, () => rows);

        await fireEvent.contextMenu(node, { clientX: 40, clientY: 60 });
        expect(menuStore.isOpen).toBe(true);
        expect(menuStore.current?.x).toBe(40);
        expect(menuStore.current?.y).toBe(60);
        expect(menuStore.current?.invoker).toBe(node);

        action.destroy();
        node.remove();
    });

    it('opens on the ContextMenu key and Shift+F10', async () => {
        menuStore.reset();
        const node = document.createElement('button');
        document.body.appendChild(node);
        const action = contextMenu(node, () => items());

        await fireEvent.keyDown(node, { key: 'ContextMenu' });
        expect(menuStore.isOpen).toBe(true);
        menuStore.reset();

        await fireEvent.keyDown(node, { key: 'F10', shiftKey: true });
        expect(menuStore.isOpen).toBe(true);

        action.destroy();
        node.remove();
    });

    it('openBelow anchors flush under the invoking element', () => {
        menuStore.reset();
        const node = document.createElement('button');
        document.body.appendChild(node);
        node.getBoundingClientRect = () =>
            ({ left: 30, bottom: 50, top: 40, right: 90, width: 60, height: 10 }) as DOMRect;
        menuStore.openBelow(items(), node, 'Dropdown');
        expect(menuStore.current?.anchor).toBe('below');
        expect(menuStore.current?.x).toBe(30);
        expect(menuStore.current?.y).toBe(54);
        expect(menuStore.current?.invoker).toBe(node);
        node.remove();
        menuStore.reset();
    });

    it('does not open when the items getter is empty', async () => {
        menuStore.reset();
        const node = document.createElement('button');
        document.body.appendChild(node);
        const action = contextMenu(node, () => []);
        await fireEvent.contextMenu(node, { clientX: 5, clientY: 5 });
        expect(menuStore.isOpen).toBe(false);
        action.destroy();
        node.remove();
    });
});

describe('Menu component (direct)', () => {
    it('renders kbd hints and separators', async () => {
        const onclose = vi.fn();
        render(Menu, {
            props: {
                items: [
                    { id: 'refresh', label: 'Refresh', kbd: 'R' },
                    { id: 'del', label: 'Delete', danger: true, separatorBefore: true },
                ] as MenuItem[],
                x: 0,
                y: 0,
                onclose,
            },
        });
        await tick();
        expect(document.querySelector('.menu-kbd')?.textContent).toBe('R');
        expect(document.querySelector('[role="separator"]')).not.toBeNull();
    });
});
