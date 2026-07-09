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

    /** Test isolation + "the tree yielded focus" reset. */
    reset(): void {
        this.activeKey = null;
    }
}

export const sidebarTreeStore = new SidebarTreeStore();
