/**
 * Focus-trap action for modal dialogs. On mount it remembers what held
 * focus, moves focus onto the dialog (so a screen reader announces its
 * label), and keeps Tab / Shift+Tab cycling within the node. On teardown
 * it restores focus to wherever it came from — so opening and closing a
 * modal never strands the keyboard user out in the (inert) background.
 *
 * This is the *focus* half of a modal: pair it with `role="dialog"`,
 * `aria-modal="true"`, and a background marked `inert` (which hides the
 * rest of the app from assistive tech and pointer/focus alike).
 */

/** The selector for elements that can hold keyboard focus. */
const FOCUSABLE = [
    'a[href]',
    'button:not([disabled])',
    'input:not([disabled])',
    'select:not([disabled])',
    'textarea:not([disabled])',
    '[tabindex]:not([tabindex="-1"])',
].join(',');

export function trapFocus(node: HTMLElement): { destroy(): void } {
    const previouslyFocused = document.activeElement as HTMLElement | null;

    /** Visible, focusable descendants in document (tab) order. */
    function focusable(): HTMLElement[] {
        return Array.from(node.querySelectorAll<HTMLElement>(FOCUSABLE)).filter(
            // offsetParent is null for display:none / detached elements.
            (element) => element.offsetParent !== null || element === document.activeElement,
        );
    }

    /**
     * The element Tab should wrap to, or `null` to let focus move normally.
     * Backwards off the first element (or the shell) wraps to the last;
     * forwards off the last wraps to the first.
     */
    function wrapTarget(backwards: boolean, items: HTMLElement[]): HTMLElement | null {
        const first = items[0];
        const last = items[items.length - 1];
        const active = document.activeElement;
        if (backwards) {
            return active === first || active === node ? last : null;
        }
        return active === last ? first : null;
    }

    function onKeydown(event: KeyboardEvent): void {
        if (event.key !== 'Tab') {
            return;
        }
        const items = focusable();
        if (items.length === 0) {
            // Nothing to land on — keep focus on the dialog itself.
            event.preventDefault();
            node.focus();
            return;
        }
        const target = wrapTarget(event.shiftKey, items);
        if (target) {
            event.preventDefault();
            target.focus();
        }
    }

    // Focus the dialog shell so its aria-label is announced; Tab then walks
    // into the trapped children.
    node.focus();
    node.addEventListener('keydown', onKeydown);

    return {
        destroy(): void {
            node.removeEventListener('keydown', onKeydown);
            // Restore focus to the trigger; but a modal opened from a context
            // menu was triggered by a menu item that unmounts when the menu
            // closes, so `previouslyFocused` is detached by now. Fall back to
            // the main content region (a `tabindex=-1` focus holder) so focus
            // never drops to <body> and strands the keyboard user.
            if (previouslyFocused?.isConnected) {
                previouslyFocused.focus();
            } else {
                document.getElementById('main-content')?.focus();
            }
        },
    };
}
