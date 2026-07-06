/**
 * The hand-rolled virtualizer: windowed rendering, ARIA listbox semantics,
 * scroll-into-view on selection movement (j/k), and near-end detection for
 * keyset infinite scroll. jsdom has no layout, so the fixture host pins
 * `fallbackViewportHeight` at 240px over 40px rows.
 */
import { fireEvent, render } from '@testing-library/svelte';
import { tick } from 'svelte';
import { describe, expect, it, vi } from 'vitest';
import VirtualListHost from './fixtures/VirtualListHost.svelte';

function manyItems(count: number): number[] {
    return Array.from({ length: count }, (_, index) => index);
}

describe('VirtualList', () => {
    it('renders only the visible window plus overscan, never the full list', () => {
        const { container } = render(VirtualListHost, { props: { items: manyItems(200) } });
        const rows = [...container.querySelectorAll('.test-row')];
        // 240px viewport / 40px rows = 6 visible + 4 overscan (top pinned at 0)
        expect(rows).toHaveLength(10);
        expect(rows[0]?.textContent).toContain('item-0');
        expect(rows[9]?.textContent).toContain('item-9');
    });

    it('is an ARIA listbox whose rows are positioned options', () => {
        const { getByRole, container } = render(VirtualListHost, {
            props: { items: manyItems(50) },
        });
        const listbox = getByRole('listbox');
        expect(listbox.getAttribute('aria-label')).toBe('test list');
        const first = container.querySelector('.test-row');
        expect(first?.getAttribute('role')).toBe('option');
        expect(first?.getAttribute('aria-posinset')).toBe('1');
        expect(first?.getAttribute('aria-setsize')).toBe('50');
    });

    it('re-windows on scroll', async () => {
        const { getByRole, container } = render(VirtualListHost, {
            props: { items: manyItems(200) },
        });
        const listbox = getByRole('listbox');
        listbox.scrollTop = 4000; // row 100 at the top of the viewport
        await fireEvent.scroll(listbox);
        const rows = [...container.querySelectorAll('.test-row')];
        expect(rows[0]?.textContent).toContain('item-96'); // 100 - overscan
        expect(rows.some((row) => row.textContent?.includes('item-0'))).toBe(false);
    });

    it('scrolls the selection into view in both directions (j/k)', async () => {
        const { getByRole, rerender } = render(VirtualListHost, {
            props: { items: manyItems(200), selectedIndex: 0 },
        });
        const listbox = getByRole('listbox');
        expect(listbox.scrollTop).toBe(0);

        // j past the viewport: bottom of row 50 (2040px) minus the 240px view
        await rerender({ items: manyItems(200), selectedIndex: 50 });
        expect(listbox.scrollTop).toBe(1800);

        // k back above the window: top of row 10
        await rerender({ items: manyItems(200), selectedIndex: 10 });
        expect(listbox.scrollTop).toBe(400);
    });

    it('keeps the selected row rendered after scroll-into-view', async () => {
        const { container, rerender } = render(VirtualListHost, {
            props: { items: manyItems(200), selectedIndex: 0 },
        });
        await rerender({ items: manyItems(200), selectedIndex: 120 });
        const rows = [...container.querySelectorAll('.test-row')];
        expect(rows.some((row) => row.textContent?.includes('item-120'))).toBe(true);
        const selected = container.querySelector('[aria-selected="true"]');
        expect(selected?.textContent).toContain('item-120');
    });

    it('fires onnearend only when the window approaches the loaded end', async () => {
        const onnearend = vi.fn();
        const { getByRole } = render(VirtualListHost, {
            props: { items: manyItems(30), onnearend },
        });
        await tick();
        expect(onnearend).not.toHaveBeenCalled();

        const listbox = getByRole('listbox');
        listbox.scrollTop = 960; // the bottom of 30 * 40px
        await fireEvent.scroll(listbox);
        expect(onnearend).toHaveBeenCalled();
    });
});
