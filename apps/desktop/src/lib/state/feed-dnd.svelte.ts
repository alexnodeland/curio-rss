/**
 * Drag-to-reorder for the sidebar feed lists: the shared drag state plus the
 * pure ordering math. A drag is scoped to one sibling group — the top-level
 * feeds, or the feeds inside one folder — and dropping a feed onto a sibling
 * moves it to that sibling's slot. Reordering never changes a feed's folder;
 * that stays the tags' job (drag only permutes within a group).
 */

class FeedDnd {
    /** The feed currently being dragged, or null. */
    draggingId: number | null = $state(null);

    start(feedId: number): void {
        this.draggingId = feedId;
    }

    clear(): void {
        this.draggingId = null;
    }
}

export const feedDnd = new FeedDnd();

/**
 * Moves `dragged` to `target`'s slot within `group`. Dragging downward drops
 * after the target, upward drops before it — the standard list-reorder feel,
 * and the only rule that can reach both ends of the list.
 */
export function moveWithinGroup(group: number[], dragged: number, target: number): number[] {
    const from = group.indexOf(dragged);
    const to = group.indexOf(target);
    if (from === -1 || to === -1 || from === to) {
        return group;
    }
    const result = [...group];
    result.splice(from, 1);
    result.splice(to, 0, dragged);
    return result;
}

/**
 * Rebuilds the complete feed order after a within-group reorder: each slot
 * that held a group member takes the next id from `newGroupOrder`; every
 * other feed keeps its exact position. Feeds live in one global sort order,
 * so a group permutation must fold back into it without disturbing the rest.
 */
export function rebuildGlobalOrder(allIds: number[], newGroupOrder: number[]): number[] {
    // Feed lists are small (dozens), so a linear membership check is fine and
    // keeps this a plain, dependency-free permutation.
    let next = 0;
    return allIds.map((id) => (newGroupOrder.includes(id) ? newGroupOrder[next++] : id));
}
