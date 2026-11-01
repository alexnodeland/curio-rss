/**
 * The session read-hold: rows the reader marked read during one view session
 * stay in the list until the view changes or an explicit refresh, even when
 * the backend window no longer returns them (an unread-only view drops a row
 * the moment it is read). Without it the row is yanked out from under the
 * cursor and everything below shifts up — the NetNewsWire/Reeder behaviour is
 * to keep it, dimmed as read, until the reader moves on.
 *
 * Pure helpers over the backend window; `ArticleListState` owns the held map.
 */

/** A held row: its last displayed summary, and where it sat. */
export interface HeldRow<T> {
    /** The row as it was displayed when it was held. */
    readonly row: T;
    /** The id of the row displayed just above it then; `null` = it was first. */
    readonly prevId: number | null;
    /** The id of the row displayed just below it then; `null` = it was last. */
    readonly nextId: number | null;
    /** Its display index then — the fallback slot when both neighbours are gone. */
    readonly index: number;
}

/**
 * Captures `id`'s current display slot in `items`, or `null` when the row is
 * not displayed (nothing on screen to keep).
 */
export function captureHeld<T extends { id: number }>(
    items: readonly T[],
    id: number,
): HeldRow<T> | null {
    const index = items.findIndex((item) => item.id === id);
    if (index === -1) {
        return null;
    }
    return {
        row: items[index],
        prevId: index > 0 ? items[index - 1].id : null,
        nextId: index < items.length - 1 ? items[index + 1].id : null,
        index,
    };
}

/** Where `slot` goes in `merged` by one neighbour, or -1 when that neighbour is absent. */
function slotBy<T extends { id: number }>(
    merged: readonly T[],
    slot: HeldRow<T>,
    by: 'prev' | 'next',
): number {
    if (by === 'prev') {
        if (slot.prevId === null) {
            return 0;
        }
        const prev = merged.findIndex((item) => item.id === slot.prevId);
        return prev === -1 ? -1 : prev + 1;
    }
    if (slot.nextId === null) {
        return -1;
    }
    return merged.findIndex((item) => item.id === slot.nextId);
}

/**
 * Inserts into `merged` every slot a neighbour pins down, preferring the
 * predecessor; repeats while a pass makes progress (a neighbour may itself be
 * a held row placed in an earlier pass). The successor is only consulted once
 * predecessors are exhausted, so a run of held rows never splits. Returns the
 * slots neither neighbour could place.
 */
function placeByNeighbours<T extends { id: number }>(
    merged: T[],
    slots: readonly HeldRow<T>[],
): HeldRow<T>[] {
    let pending = [...slots];
    let by: 'prev' | 'next' = 'prev';
    while (pending.length > 0) {
        const unplaced = pending.filter((slot) => {
            const at = slotBy(merged, slot, by);
            if (at !== -1) {
                merged.splice(at, 0, slot.row);
            }
            return at === -1;
        });
        const progressed = unplaced.length < pending.length;
        pending = unplaced;
        if (progressed) {
            by = 'prev';
        } else if (by === 'prev') {
            by = 'next';
        } else {
            break;
        }
    }
    return pending;
}

/**
 * The displayed rows: the backend window verbatim, with every held row the
 * window no longer returns re-inserted where it sat — right below the row
 * that preceded it, else right above the row that followed it (chained, so a
 * run of held rows keeps its order), else at its old index. The backend order
 * is never re-sorted: held rows are placed relative to their neighbours, not
 * by any client-side key. Returns `window` itself (same identity) when nothing
 * is missing.
 */
export function mergeHeld<T extends { id: number }>(
    window: readonly T[],
    held: ReadonlyMap<number, HeldRow<T>>,
): readonly T[] {
    if (held.size === 0) {
        return window;
    }
    const present = new Set(window.map((item) => item.id));
    const missing = [...held.values()].filter((slot) => !present.has(slot.row.id));
    if (missing.length === 0) {
        return window;
    }
    const merged = [...window];
    const stranded = placeByNeighbours(merged, missing);
    // Both neighbours gone for good (deleted, archived): the old slot.
    for (const slot of stranded.sort((a, b) => a.index - b.index)) {
        merged.splice(Math.min(slot.index, merged.length), 0, slot.row);
    }
    return merged;
}
