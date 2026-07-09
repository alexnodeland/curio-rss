/**
 * The tooltip singleton — one styled tooltip is visible at a time. The
 * `tooltip` action drives it on hover and focus; TooltipHost renders it. A
 * single stable id backs `aria-describedby` wiring on whichever element owns
 * the tooltip right now.
 */
export interface ActiveTooltip {
    readonly text: string;
    readonly kbd?: string;
    /** The trigger's bounding rect, in viewport coordinates. */
    readonly rect: DOMRect;
}

class TooltipStore {
    current: ActiveTooltip | null = $state(null);

    /** The element the visible tooltip belongs to (guards stale hides). */
    owner: HTMLElement | null = null;

    /** Stable id for the single tooltip element (aria-describedby target). */
    readonly id = 'cx-tooltip';

    show(node: HTMLElement, text: string, kbd: string | undefined, rect: DOMRect): void {
        this.owner = node;
        this.current = { text, kbd, rect };
    }

    /** Hides only if `node` currently owns the tooltip. */
    hide(node: HTMLElement): void {
        if (this.owner === node) {
            this.owner = null;
            this.current = null;
        }
    }

    /** Force-hide regardless of owner (scroll/blur). */
    hideAll(): void {
        this.owner = null;
        this.current = null;
    }

    /** Test isolation. */
    reset(): void {
        this.hideAll();
    }
}

export const tooltipStore = new TooltipStore();
