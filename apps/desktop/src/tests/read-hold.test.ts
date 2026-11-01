/**
 * The session read-hold's pure merge: held rows the backend window dropped
 * come back in the slot they were displayed in, chained runs keep their order,
 * and the backend order is never re-sorted.
 */
import { type HeldRow, captureHeld, mergeHeld } from '$lib/state/read-hold';
import { describe, expect, it } from 'vitest';

interface Row {
    id: number;
}

function rows(...ids: number[]): Row[] {
    return ids.map((id) => ({ id }));
}

function ids(items: readonly Row[]): number[] {
    return items.map((item) => item.id);
}

/** Holds each of `heldIds` against `displayed`, in order (as the UI would). */
function holdAll(displayed: readonly Row[], heldIds: number[]): Map<number, HeldRow<Row>> {
    const held = new Map<number, HeldRow<Row>>();
    for (const id of heldIds) {
        const slot = captureHeld(mergeHeld(displayed, held), id);
        if (slot !== null) {
            held.set(id, slot);
        }
    }
    return held;
}

describe('read-hold', () => {
    it('captures a displayed row with its predecessor and index', () => {
        expect(captureHeld(rows(5, 4, 3), 4)).toEqual({
            row: { id: 4 },
            prevId: 5,
            nextId: 3,
            index: 1,
        });
        expect(captureHeld(rows(5, 4, 3), 3)).toEqual({
            row: { id: 3 },
            prevId: 4,
            nextId: null,
            index: 2,
        });
    });

    it('captures nothing for a row that is not displayed', () => {
        expect(captureHeld(rows(5, 4, 3), 9)).toBeNull();
    });

    it('returns the window itself when nothing held is missing', () => {
        const window = rows(5, 4, 3);
        expect(mergeHeld(window, new Map())).toBe(window);
        expect(mergeHeld(window, holdAll(window, [4]))).toBe(window);
    });

    it('re-inserts a dropped row right below its old predecessor', () => {
        const held = holdAll(rows(5, 4, 3), [4]);
        expect(ids(mergeHeld(rows(5, 3), held))).toEqual([5, 4, 3]);
    });

    it('re-inserts a dropped first row at the top', () => {
        const held = holdAll(rows(5, 4, 3), [5]);
        expect(ids(mergeHeld(rows(4, 3), held))).toEqual([5, 4, 3]);
    });

    it('keeps a run of held rows in order, whichever was held first', () => {
        const display = rows(6, 5, 4, 3);
        expect(ids(mergeHeld(rows(6, 3), holdAll(display, [5, 4])))).toEqual([6, 5, 4, 3]);
        expect(ids(mergeHeld(rows(6, 3), holdAll(display, [4, 5])))).toEqual([6, 5, 4, 3]);
    });

    it('keeps backend order verbatim — held rows go by neighbour, not by id', () => {
        // Non-monotonic backend order: the merge must not sort anything.
        const held = holdAll(rows(9, 40, 2), [40]);
        expect(ids(mergeHeld(rows(9, 2), held))).toEqual([9, 40, 2]);
    });

    it('follows its predecessor when new rows arrive above', () => {
        const held = holdAll(rows(5, 4, 3), [4]);
        expect(ids(mergeHeld(rows(8, 7, 5, 3), held))).toEqual([8, 7, 5, 4, 3]);
    });

    it('sits above its old successor when the predecessor is gone', () => {
        const held = holdAll(rows(5, 4, 3, 2), [3]);
        // 4 (its predecessor) left the window and is not held.
        expect(ids(mergeHeld(rows(5, 2), held))).toEqual([5, 3, 2]);
    });

    it('a run keeps its order when the row above it is released', () => {
        // 4 and 3 held, then 4 let go (archived): 3 still sits above 2.
        const held = holdAll(rows(5, 4, 3, 2), [4, 3]);
        held.delete(4);
        expect(ids(mergeHeld(rows(5, 2), held))).toEqual([5, 3, 2]);
    });

    it('falls back to the old index when both neighbours are gone', () => {
        const held = holdAll(rows(5, 4, 3, 2), [3]);
        expect(ids(mergeHeld(rows(5), held))).toEqual([5, 3]);
        expect(ids(mergeHeld(rows(9, 8, 7, 6), held))).toEqual([9, 8, 3, 7, 6]);
    });

    it('a held last row follows its predecessor, else keeps its old slot', () => {
        const held = holdAll(rows(5, 4), [4]);
        expect(ids(mergeHeld(rows(5), held))).toEqual([5, 4]);
        // No successor to fall back on once 5 is gone too.
        expect(ids(mergeHeld(rows(9), held))).toEqual([9, 4]);
    });
});
