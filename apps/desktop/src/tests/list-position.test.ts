/**
 * Per-view scroll memory: positions are anchored to a row id (plus the offset
 * into it), so they survive rows arriving above and density changes, and a
 * view with no memory opens at the top.
 */
import { ListPositions, capturePosition, restoreScrollTop } from '$lib/state/list-position';
import { describe, expect, it } from 'vitest';

function rows(...ids: number[]): { id: number }[] {
    return ids.map((id) => ({ id }));
}

const LIST = rows(10, 9, 8, 7, 6, 5);

describe('list position', () => {
    it('captures the top row and the offset into it', () => {
        expect(capturePosition(LIST, 2 * 50 + 13, 50)).toEqual({
            anchorId: 8,
            index: 2,
            offset: 13,
        });
        expect(capturePosition(LIST, 0, 50)).toEqual({ anchorId: 10, index: 0, offset: 0 });
    });

    it('clamps past the end and captures nothing for an empty list', () => {
        expect(capturePosition(LIST, 50 * 99, 50)?.anchorId).toBe(5);
        expect(capturePosition([], 120, 50)).toBeNull();
        expect(capturePosition(LIST, 120, 0)).toBeNull();
    });

    it('restores the anchor where it sits now, not where it sat', () => {
        const position = capturePosition(LIST, 2 * 50 + 13, 50);
        expect(position).not.toBeNull();
        // Two rows arrived above: the anchor (8) moved from index 2 to 4.
        const grown = rows(12, 11, ...LIST.map((row) => row.id));
        expect(restoreScrollTop(position ?? undefined, grown, 50)).toBe(4 * 50 + 13);
    });

    it('falls back to the captured index when the anchor is gone', () => {
        const position = { anchorId: 99, index: 3, offset: 10 };
        expect(restoreScrollTop(position, LIST, 50)).toBe(3 * 50 + 10);
        expect(restoreScrollTop(position, rows(10, 9), 50)).toBe(1 * 50 + 10);
    });

    it('re-scales to a new row height (density change) without overshooting a row', () => {
        const position = { anchorId: 8, index: 2, offset: 70 };
        expect(restoreScrollTop(position, LIST, 56)).toBe(2 * 56 + 55);
    });

    it('opens at the top with no memory or no rows', () => {
        expect(restoreScrollTop(undefined, LIST, 50)).toBe(0);
        expect(restoreScrollTop({ anchorId: 8, index: 2, offset: 0 }, [], 50)).toBe(0);
    });

    it('remembers per key, forgets on null, and resets', () => {
        const positions = new ListPositions();
        const a = { anchorId: 8, index: 2, offset: 0 };
        positions.save('a', a);
        expect(positions.get('a')).toEqual(a);
        expect(positions.get('b')).toBeUndefined();
        positions.save('a', null);
        expect(positions.get('a')).toBeUndefined();
        positions.save('a', a);
        positions.reset();
        expect(positions.get('a')).toBeUndefined();
    });
});
