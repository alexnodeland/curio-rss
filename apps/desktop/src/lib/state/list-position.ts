/**
 * Per-view scroll memory for the article list. Each filter combination
 * (`filterKey`) remembers where the reader left it as an *anchor* — the id of
 * the row at the viewport top plus the pixel offset into it — rather than a
 * raw `scrollTop`, so the position survives rows arriving above it (a
 * background refresh), a density change, or the window being reloaded.
 * A view never visited opens at the top; a view never inherits another
 * view's offset.
 */

/** Where a list was left: the top row's id, its index then, and the offset into it. */
export interface ListPosition {
    readonly anchorId: number;
    /** The anchor's index when captured — the fallback when the anchor is gone. */
    readonly index: number;
    /** Pixels scrolled into the anchor row (0 ≤ offset < row height). */
    readonly offset: number;
}

/** Captures the position for `scrollTop` over `items`, or `null` for an empty list. */
export function capturePosition(
    items: readonly { id: number }[],
    scrollTop: number,
    rowHeight: number,
): ListPosition | null {
    if (items.length === 0 || rowHeight <= 0) {
        return null;
    }
    const index = Math.min(Math.max(0, Math.floor(scrollTop / rowHeight)), items.length - 1);
    const offset = Math.max(0, Math.min(scrollTop - index * rowHeight, rowHeight - 1));
    return { anchorId: items[index].id, index, offset };
}

/**
 * The `scrollTop` that puts `position` back at the viewport top over the
 * current `items`: the anchor row's present index (it may have moved), else
 * its captured index clamped to the list. No position → the top.
 */
export function restoreScrollTop(
    position: ListPosition | undefined,
    items: readonly { id: number }[],
    rowHeight: number,
): number {
    if (position === undefined || items.length === 0) {
        return 0;
    }
    const found = items.findIndex((item) => item.id === position.anchorId);
    const index = found === -1 ? Math.min(position.index, items.length - 1) : found;
    return index * rowHeight + Math.min(position.offset, rowHeight - 1);
}

/** The per-view position map (keyed by `filterKey`). */
export class ListPositions {
    #positions = new Map<string, ListPosition>();

    /** Remembers `position` for the view `key`; `null` forgets it. */
    save(key: string, position: ListPosition | null): void {
        if (position === null) {
            this.#positions.delete(key);
        } else {
            this.#positions.set(key, position);
        }
    }

    /** The view's remembered position, if any. */
    get(key: string): ListPosition | undefined {
        return this.#positions.get(key);
    }

    /** Test isolation — forgets every view. */
    reset(): void {
        this.#positions.clear();
    }
}
