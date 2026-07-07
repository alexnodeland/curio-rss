/**
 * Feed drag-reorder ordering math: moving within a sibling group, and
 * folding that group permutation back into the one global feed order without
 * disturbing feeds outside the group.
 */
import { moveWithinGroup, rebuildGlobalOrder } from '$lib/state/feed-dnd.svelte';
import { describe, expect, it } from 'vitest';

describe('moveWithinGroup', () => {
    it('drops after the target when dragging downward', () => {
        expect(moveWithinGroup([1, 2, 3, 4], 1, 3)).toEqual([2, 3, 1, 4]);
    });

    it('drops before the target when dragging upward', () => {
        expect(moveWithinGroup([1, 2, 3, 4], 4, 1)).toEqual([4, 1, 2, 3]);
    });

    it('can reach the very end of the list', () => {
        expect(moveWithinGroup([1, 2, 3], 1, 3)).toEqual([2, 3, 1]);
    });

    it('is a no-op for a drop on itself or an unknown id', () => {
        expect(moveWithinGroup([1, 2, 3], 2, 2)).toEqual([1, 2, 3]);
        expect(moveWithinGroup([1, 2, 3], 9, 2)).toEqual([1, 2, 3]);
    });
});

describe('rebuildGlobalOrder', () => {
    it('replaces only the group members, in their new order, in place', () => {
        // Group {2, 4} reordered to [4, 2]; feeds 1, 3, 5 never move.
        expect(rebuildGlobalOrder([1, 2, 3, 4, 5], [4, 2])).toEqual([1, 4, 3, 2, 5]);
    });

    it('leaves a single-item or untouched group identical', () => {
        expect(rebuildGlobalOrder([1, 2, 3], [2])).toEqual([1, 2, 3]);
        expect(rebuildGlobalOrder([1, 2, 3], [1, 2, 3])).toEqual([1, 2, 3]);
    });

    it('handles a full-list group as a straight reorder', () => {
        expect(rebuildGlobalOrder([1, 2, 3], [3, 1, 2])).toEqual([3, 1, 2]);
    });
});
