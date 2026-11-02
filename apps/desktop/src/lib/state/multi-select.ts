/**
 * The pure list math behind multi-selection (the selection store owns the
 * state): a Shift range between two rows in list order, and which selected
 * ids are still listed. Rows are identified by article id, never by index,
 * so a refetch that shifts rows cannot move a selection onto other rows.
 */

/** A row with an article id — every list row is one. */
interface Row {
    readonly id: number;
}

/** A Shift range: its anchor row, the target's index, and the ids it spans. */
export interface Range {
    /** The row the range pivots on (the target itself when the anchor is gone). */
    readonly anchor: number;
    /** The target row's index in the list. */
    readonly index: number;
    /** Every id from the anchor to the target, inclusive, in list order. */
    readonly ids: readonly number[];
}

/**
 * The contiguous rows from `anchorId` to `targetId`, in list order. An anchor
 * that is no longer listed collapses the range to the target alone; `null`
 * when the target itself is not listed.
 */
export function rangeBetween(
    items: readonly Row[],
    anchorId: number,
    targetId: number,
): Range | null {
    const index = items.findIndex((item) => item.id === targetId);
    if (index === -1) {
        return null;
    }
    const found = items.findIndex((item) => item.id === anchorId);
    const from = found === -1 ? index : found;
    const span = items.slice(Math.min(from, index), Math.max(from, index) + 1);
    return { anchor: items[from].id, index, ids: span.map((item) => item.id) };
}

/** The `selected` ids still present in `items` (selection order kept). */
export function stillListed(selected: Iterable<number>, items: readonly Row[]): number[] {
    const listed = new Set(items.map((item) => item.id));
    return [...selected].filter((id) => listed.has(id));
}
