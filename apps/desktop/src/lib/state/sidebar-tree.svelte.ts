/**
 * The sidebar tree's keyboard cursor: the DOM id (= a `VisibleRow.key`) of the
 * row the arrow keys are currently on. Kept apart from `selectionStore` because
 * navigating the tree is distinct from selecting a feed — you move the cursor
 * with the arrows, then Enter commits the selection (which reloads the list).
 * The tree container reads it for `aria-activedescendant`; each row reads it to
 * paint the focus ring.
 */
class SidebarTreeStore {
    /** The active row's key/id, or `null` when the tree has no cursor yet. */
    activeKey: string | null = $state(null);

    /**
     * Whether the tree currently holds keyboard focus. The cursor ring only
     * paints while this is true, so it never lingers on a row after focus has
     * moved into the list/reader (which would read as a second selection).
     */
    focused: boolean = $state(false);

    /** Test isolation + "the tree yielded focus" reset. */
    reset(): void {
        this.activeKey = null;
        this.focused = false;
    }
}

export const sidebarTreeStore = new SidebarTreeStore();
