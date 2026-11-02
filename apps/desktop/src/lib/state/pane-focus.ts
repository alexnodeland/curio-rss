/**
 * Keeps the keyboard out of hidden panes. A pane the layout hides (sidebar
 * collapse, focus mode, the narrow-window auto-collapse) stays mounted but
 * `inert`, so whatever held focus inside it would be stranded: WebKit drops
 * it to `<body>` at some later frame, and a sidebar left as the focus zone
 * would keep swallowing every shortcut. ThreePane calls
 * {@link evacuateHiddenPanes} whenever pane visibility changes; the next
 * visible pane to the right takes the keyboard instead.
 */
import { layoutStore } from './layout.svelte';
import { selectionStore } from './selection.svelte';

/** A pane that can be hidden, as ThreePane tags its slot (`data-pane`). */
export type HideablePane = 'sidebar' | 'list';

/** Whether `element` sits inside the slot of `pane`. */
export function inPane(element: Element | null, pane: HideablePane): boolean {
    return element?.closest(`[data-pane="${pane}"]`) != null;
}

/**
 * Moves focus out of any pane that is no longer on screen: to the article
 * list when it shows, else to the reader (which takes it once an article is
 * open; with none, focus rests on `<body>`, where the list's keys still
 * work). A no-op when focus is already somewhere visible.
 */
export function evacuateHiddenPanes(doc: Document = document): void {
    const active = doc.activeElement;
    const strandedInSidebar =
        !layoutStore.sidebarVisible &&
        (selectionStore.focus === 'sidebar' || inPane(active, 'sidebar'));
    const strandedInList = !layoutStore.listVisible && inPane(active, 'list');
    if (!strandedInSidebar && !strandedInList) {
        return;
    }
    if (active instanceof HTMLElement && (inPane(active, 'sidebar') || inPane(active, 'list'))) {
        active.blur();
    }
    if (layoutStore.listVisible) {
        selectionStore.focusList();
    } else {
        selectionStore.focusReader();
    }
}
