/**
 * Pure keyboard-navigation semantics for the sidebar `role="tree"`. Given the
 * flattened visible rows and the active index, `treeKeyAction` maps a key to
 * one of four intents — move the active row, toggle a folder's expansion,
 * activate (select) the current row, or nothing — following the WAI-ARIA tree
 * pattern (no wrap at the ends; Left/Right collapse/expand or step to
 * parent/first-child). Kept side-effect-free so the component only interprets
 * the result, and it stays unit-tested without a DOM.
 */
import type { VisibleRow } from '$lib/state/feed-tree';

export type TreeKeyResult =
    | { readonly type: 'none' }
    | { readonly type: 'move'; readonly index: number }
    | { readonly type: 'toggle'; readonly path: string }
    | { readonly type: 'activate'; readonly index: number };

const NONE: TreeKeyResult = { type: 'none' };

/** The nearest earlier row shallower than `rows[index]` — its tree parent. */
function parentIndex(rows: readonly VisibleRow[], index: number): number {
    const depth = rows[index].depth;
    for (let i = index - 1; i >= 0; i -= 1) {
        if (rows[i].depth < depth) {
            return i;
        }
    }
    return -1;
}

/** Right: expand a collapsed folder, else step into its first child (if any). */
function expandOrDescend(
    rows: readonly VisibleRow[],
    activeIndex: number,
    current: VisibleRow,
): TreeKeyResult {
    if (current.kind !== 'folder') {
        return NONE;
    }
    if (!current.expanded) {
        return { type: 'toggle', path: current.path };
    }
    const child = rows[activeIndex + 1];
    return child !== undefined && child.depth > current.depth
        ? { type: 'move', index: activeIndex + 1 }
        : NONE;
}

/** Left: collapse an expanded folder, else step out to the parent row (if any). */
function collapseOrAscend(
    rows: readonly VisibleRow[],
    activeIndex: number,
    current: VisibleRow,
): TreeKeyResult {
    if (current.kind === 'folder' && current.expanded) {
        return { type: 'toggle', path: current.path };
    }
    const parent = parentIndex(rows, activeIndex);
    return parent >= 0 ? { type: 'move', index: parent } : NONE;
}

/**
 * Resolves a keydown against the visible tree. `activeIndex < 0` means nothing
 * is active yet, so any navigation key lands on the first row.
 */
export function treeKeyAction(
    rows: readonly VisibleRow[],
    activeIndex: number,
    key: string,
): TreeKeyResult {
    const count = rows.length;
    if (count === 0) {
        return NONE;
    }
    if (activeIndex < 0) {
        // First interaction: any move/activate key selects the top row.
        return key === 'Enter' || key === ' '
            ? { type: 'activate', index: 0 }
            : { type: 'move', index: 0 };
    }
    const current = rows[activeIndex];
    switch (key) {
        case 'ArrowDown':
            return { type: 'move', index: Math.min(activeIndex + 1, count - 1) };
        case 'ArrowUp':
            return { type: 'move', index: Math.max(activeIndex - 1, 0) };
        case 'Home':
            return { type: 'move', index: 0 };
        case 'End':
            return { type: 'move', index: count - 1 };
        case 'Enter':
        case ' ':
            return { type: 'activate', index: activeIndex };
        case 'ArrowRight':
            return expandOrDescend(rows, activeIndex, current);
        case 'ArrowLeft':
            return collapseOrAscend(rows, activeIndex, current);
        default:
            return NONE;
    }
}
