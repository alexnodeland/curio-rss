/**
 * Modal action for a dialog rendered *inside* the app frame (the reader's
 * image lightbox), where the shell's `inert` main region can't be used —
 * the dialog would be inert too. It pairs {@link trapFocus} with the other
 * half of `aria-modal="true"`: the rest of the app frame is made `inert`,
 * by walking from the dialog up to the main region and marking every
 * sibling at each level. The dialog's own ancestors stay live, so nothing
 * it needs is hidden, while a screen reader's virtual cursor (and the
 * pointer, and Tab) can no longer wander into the page behind it. Outside
 * the main region (toasts, live regions) stays as it is for a shell modal.
 *
 * Only elements this action marked are restored on teardown — anything
 * already inert is left as it was.
 */
import { trapFocus } from '$lib/actions/trap-focus';

/** The shell's main region; the walk stops there (see the module doc). */
const MAIN_ID = 'main-content';

/** Marks every element of the app frame outside `node` inert; returns what it marked. */
export function inertOutside(node: HTMLElement): Element[] {
    const marked: Element[] = [];
    let current: Element = node;
    while (current.id !== MAIN_ID && current !== document.body) {
        const parent: Element | null = current.parentElement;
        if (parent === null) {
            break;
        }
        for (const sibling of parent.children) {
            if (sibling !== current && !sibling.hasAttribute('inert')) {
                sibling.setAttribute('inert', '');
                marked.push(sibling);
            }
        }
        current = parent;
    }
    return marked;
}

export function modalLayer(node: HTMLElement): { destroy(): void } {
    // Trap first: it records the trigger and moves focus into the dialog
    // while the trigger is still live (making its ancestor inert would
    // blur it to <body>).
    const trap = trapFocus(node);
    const marked = inertOutside(node);
    return {
        destroy(): void {
            for (const element of marked) {
                element.removeAttribute('inert');
            }
            // Restore focus only once the trigger is live again.
            trap.destroy();
        },
    };
}
