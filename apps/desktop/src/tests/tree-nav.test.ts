/**
 * The sidebar tree's keyboard semantics: no-wrap Up/Down, Home/End, Left/Right
 * collapse-expand or step to parent/first-child, Enter/Space activate, and the
 * "nothing active yet → first row" bootstrap.
 */
import type { FeedDto } from '$lib/bindings';
import type { VisibleRow } from '$lib/state/feed-tree';
import { treeKeyAction } from '$lib/utils/tree-nav';
import { describe, expect, it } from 'vitest';

const feed = (id: number): FeedDto =>
    ({ id, url: `https://f${id}.example`, title: `F${id}`, tags: [] }) as unknown as FeedDto;

// Tech (expanded) > [Tech/DB (collapsed), feed 1]; then ungrouped feed 2.
const ROWS: VisibleRow[] = [
    {
        kind: 'folder',
        key: 'folder:Tech',
        path: 'Tech',
        depth: 0,
        expanded: true,
        folder: {} as never,
    },
    {
        kind: 'folder',
        key: 'folder:Tech/DB',
        path: 'Tech/DB',
        depth: 1,
        expanded: false,
        folder: {} as never,
    },
    { kind: 'feed', key: 'feed:Tech:1', id: 1, depth: 1, feed: feed(1) },
    { kind: 'feed', key: 'feed::2', id: 2, depth: 0, feed: feed(2) },
];

describe('treeKeyAction', () => {
    it('an empty tree yields nothing', () => {
        expect(treeKeyAction([], -1, 'ArrowDown')).toEqual({ type: 'none' });
    });

    it('the first key with nothing active lands on the top row', () => {
        expect(treeKeyAction(ROWS, -1, 'ArrowDown')).toEqual({ type: 'move', index: 0 });
        expect(treeKeyAction(ROWS, -1, 'ArrowUp')).toEqual({ type: 'move', index: 0 });
        expect(treeKeyAction(ROWS, -1, 'Enter')).toEqual({ type: 'activate', index: 0 });
    });

    it('Down/Up move one row and clamp at the ends (no wrap)', () => {
        expect(treeKeyAction(ROWS, 0, 'ArrowDown')).toEqual({ type: 'move', index: 1 });
        expect(treeKeyAction(ROWS, 3, 'ArrowDown')).toEqual({ type: 'move', index: 3 });
        expect(treeKeyAction(ROWS, 2, 'ArrowUp')).toEqual({ type: 'move', index: 1 });
        expect(treeKeyAction(ROWS, 0, 'ArrowUp')).toEqual({ type: 'move', index: 0 });
    });

    it('Home/End jump to the ends', () => {
        expect(treeKeyAction(ROWS, 2, 'Home')).toEqual({ type: 'move', index: 0 });
        expect(treeKeyAction(ROWS, 0, 'End')).toEqual({ type: 'move', index: 3 });
    });

    it('Enter/Space activate the current row', () => {
        expect(treeKeyAction(ROWS, 2, 'Enter')).toEqual({ type: 'activate', index: 2 });
        expect(treeKeyAction(ROWS, 0, ' ')).toEqual({ type: 'activate', index: 0 });
    });

    it('Right expands a collapsed folder, else steps to its first child', () => {
        // row 1 = collapsed folder → expand it
        expect(treeKeyAction(ROWS, 1, 'ArrowRight')).toEqual({ type: 'toggle', path: 'Tech/DB' });
        // row 0 = expanded folder with a deeper child next → move into it
        expect(treeKeyAction(ROWS, 0, 'ArrowRight')).toEqual({ type: 'move', index: 1 });
        // row 2 = feed → Right is a no-op
        expect(treeKeyAction(ROWS, 2, 'ArrowRight')).toEqual({ type: 'none' });
    });

    it('Right on an expanded but childless folder is a no-op', () => {
        const rows: VisibleRow[] = [
            {
                kind: 'folder',
                key: 'folder:Empty',
                path: 'Empty',
                depth: 0,
                expanded: true,
                folder: {} as never,
            },
            { kind: 'feed', key: 'feed::9', id: 9, depth: 0, feed: feed(9) },
        ];
        expect(treeKeyAction(rows, 0, 'ArrowRight')).toEqual({ type: 'none' });
    });

    it('Left collapses an expanded folder, else steps to the parent', () => {
        // row 0 = expanded folder → collapse
        expect(treeKeyAction(ROWS, 0, 'ArrowLeft')).toEqual({ type: 'toggle', path: 'Tech' });
        // row 2 = feed at depth 1 → move to parent folder (row 0, depth 0)
        expect(treeKeyAction(ROWS, 2, 'ArrowLeft')).toEqual({ type: 'move', index: 0 });
        // row 3 = top-level feed with no parent → no-op
        expect(treeKeyAction(ROWS, 3, 'ArrowLeft')).toEqual({ type: 'none' });
    });

    it('an unowned key is a no-op', () => {
        expect(treeKeyAction(ROWS, 0, 'x')).toEqual({ type: 'none' });
    });
});
