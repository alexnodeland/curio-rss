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

    it('opens at initialScrollTop: the DOM offset and the window both start there', async () => {
        const { getByRole, container } = render(VirtualListHost, {
            props: { items: manyItems(200), initialScrollTop: 40 * 50 },
        });
        await tick();
        expect(getByRole('listbox').scrollTop).toBe(2000);
        const rows = [...container.querySelectorAll('.test-row')];
        expect(rows[0]?.textContent).toContain('item-46'); // 50 - overscan
    });

    it('reports the viewport — rows passed, rows on screen, offset — at mount and on scroll', async () => {
        const reports: { first: number; end: number; scrollTop: number }[] = [];
        const { getByRole } = render(VirtualListHost, {
            props: {
                items: manyItems(200),
                initialScrollTop: 400,
                onviewport: (report) => reports.push(report),
            },
        });
        await tick();
        // 240px viewport over 40px rows from 400px: rows 10..16 on screen.
        expect(reports[0]).toEqual({ first: 10, end: 16, scrollTop: 400 });

        const listbox = getByRole('listbox');
        listbox.scrollTop = 420;
        await fireEvent.scroll(listbox);
        expect(reports[reports.length - 1]).toEqual({ first: 10, end: 17, scrollTop: 420 });
    });
    it('anchors the offset to the rows on screen when rows leave or arrive above them', async () => {
        const reports: { first: number; end: number; scrollTop: number }[] = [];
        const { getByRole, container, rerender } = render(VirtualListHost, {
            props: { items: manyItems(200), onviewport: (report) => reports.push(report) },
        });
        const listbox = getByRole('listbox');
        listbox.scrollTop = 40 * 50 + 15; // row 50 at the top, 15px in
        await fireEvent.scroll(listbox);

        // Twenty rows above the viewport go away: row 50 stays at the top.
        await rerender({ items: manyItems(200).slice(20) });
        await tick();
        expect(listbox.scrollTop).toBe(40 * 30 + 15);
        expect(reports[reports.length - 1]).toEqual({ first: 30, end: 37, scrollTop: 1215 });
        const rows = [...container.querySelectorAll('.test-row')];
        expect(rows[0]?.textContent).toContain('item-46'); // row 50 minus overscan

        // The top row itself goes too: the next row that was on screen anchors.
        await rerender({ items: manyItems(200).filter((item) => item >= 20 && item !== 50) });
        await tick();
        expect(listbox.scrollTop).toBe(40 * 29 + 15);

        // Rows arriving above push the offset down by as much.
        await rerender({
            items: [-3, -2, -1, ...manyItems(200).filter((item) => item >= 20 && item !== 50)],
        });
        await tick();
        expect(listbox.scrollTop).toBe(40 * 32 + 15);
    });

    it('stays at the top when rows arrive above an unscrolled list', async () => {
        const { getByRole, container, rerender } = render(VirtualListHost, {
            props: { items: manyItems(50) },
        });
        await rerender({ items: [-2, -1, ...manyItems(50)] });
        await tick();
        expect(getByRole('listbox').scrollTop).toBe(0);
        expect(container.querySelector('.test-row')?.textContent).toContain('item--2');
    });
});
