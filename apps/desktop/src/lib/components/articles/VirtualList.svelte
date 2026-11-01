<script lang="ts" module>
/** What the viewport shows right now (see `onviewport`). */
export interface ViewportReport {
    /** Rows `[0, first)` have fully scrolled up past the viewport top. */
    first: number;
    /** One past the last row intersecting the viewport (no overscan). */
    end: number;
    /** The viewport's scroll offset in px. */
    scrollTop: number;
}
</script>

<script lang="ts" generics="T">
/**
 * The hand-rolled fixed-row-height virtualizer: renders only the rows that
 * intersect the viewport (plus `overscan`), keeps the selected row visible
 * when selection moves (j/k scroll-into-view), and reports proximity to the
 * bottom of the loaded window (`onnearend`) so the owner can page more rows
 * in — re-entrancy is the owner's concern (the list store already guards).
 *
 * ARIA: the scroll container is the listbox; rows carry `role="option"` +
 * `aria-posinset`/`aria-setsize` themselves (the snippet receives the
 * absolute index). In jsdom (no layout) the measured viewport height is 0,
 * so `fallbackViewportHeight` keeps the math real in component tests.
 *
 * The scroll position is the instance's own: an owner showing a different
 * list mounts a fresh instance (`{#key}`) and hands it that list's remembered
 * offset through `initialScrollTop`, so no list inherits another's offset.
 */
import { type Snippet, untrack } from 'svelte';

let {
    items,
    rowHeight,
    key,
    selectedIndex = -1,
    overscan = 4,
    nearEndMargin = 10,
    fallbackViewportHeight = 600,
    initialScrollTop = 0,
    label,
    activeDescendantId,
    focusNonce = 0,
    onnearend,
    onviewport,
    onmove,
    onmenukey,
    row,
}: {
    items: readonly T[];
    rowHeight: number;
    key: (item: T) => string | number;
    selectedIndex?: number;
    overscan?: number;
    nearEndMargin?: number;
    fallbackViewportHeight?: number;
    /** The offset this instance opens at (read once, at mount). */
    initialScrollTop?: number;
    label: string;
    /** The `id` of the active option — mirrored into `aria-activedescendant`
     *  so a screen reader tracks selection while focus stays on the listbox. */
    activeDescendantId?: string;
    /** Bumped by the owner to pull DOM focus onto the listbox (entering the
     *  list from the sidebar). The listbox is the single durable focus holder,
     *  so focus lives here, never on an individual row that can be unmounted. */
    focusNonce?: number;
    onnearend?: () => void;
    /**
     * What the viewport shows: the rows scrolled up past the top, the rows
     * on screen, and the raw offset. Fires at mount and whenever the scroll
     * offset or the items change; the owner decides what to make of it
     * (mark-on-scroll marks rows it saw scroll past; the list remembers its
     * position per view).
     */
    onviewport?: (report: ViewportReport) => void;
    /**
     * A listbox navigation key was pressed while the list held focus. The
     * owner moves selection (the same move `j`/`k` make); the listbox itself
     * stays selection-agnostic. Row-by-row movement is `j`/`k` only — the
     * arrow keys are deliberately unbound.
     */
    onmove?: (to: 'first' | 'last' | 'pageDown' | 'pageUp') => void;
    /**
     * The keyboard context-menu key (ContextMenu / Shift+F10) was pressed while
     * the list held focus. Rows are `tabindex="-1"` so their own menu-key
     * handler never fires; the owner opens the menu for the selected row.
     */
    onmenukey?: () => void;
    row: Snippet<[T, number]>;
} = $props();

/** Maps the roving-navigation keys to a move; other keys fall through. */
const MOVE_KEYS: Record<string, 'first' | 'last' | 'pageDown' | 'pageUp'> = {
    Home: 'first',
    End: 'last',
    PageDown: 'pageDown',
    PageUp: 'pageUp',
};

// Keyboard-navigation mode: once the keyboard drives the list, stale pointer
// `:hover` on the row the mouse last sat over is muted so it can't read as a
// second highlight next to the keyboard-selected row (the mouse-then-j/k
// double-highlight). The next real pointer move clears it.
let keyboardMode = $state(false);

function onKeydown(event: KeyboardEvent): void {
    if (
        onmenukey !== undefined &&
        (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10'))
    ) {
        event.preventDefault();
        keyboardMode = true;
        onmenukey();
        return;
    }
    if (event.key === ' ') {
        // Suppress Space's native page-scroll; selection is driven by j/k.
        event.preventDefault();
        return;
    }
    const move = MOVE_KEYS[event.key];
    if (move === undefined || onmove === undefined) {
        return;
    }
    // We do our own selection move + scroll-into-view; suppress the
    // listbox's native line-scroll so the two don't fight.
    event.preventDefault();
    keyboardMode = true;
    onmove(move);
}

let viewport: HTMLDivElement | undefined = $state();
let measuredHeight = $state(0);
// The opening offset is read once: later changes to the prop are ignored (a
// different list is a different instance).
let scrollTop = $state(untrack(() => Math.max(0, initialScrollTop)));

const viewportHeight = $derived(measuredHeight > 0 ? measuredHeight : fallbackViewportHeight);
const start = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscan));
const end = $derived(
    Math.min(items.length, Math.ceil((scrollTop + viewportHeight) / rowHeight) + overscan),
);
const visible = $derived(items.slice(start, end));

// aria-activedescendant must name a *rendered* option; when the active row has
// scrolled out of the virtualized window, clear it rather than dangle a pointer
// at an element that no longer exists in the DOM.
const activeRendered = $derived(selectedIndex >= start && selectedIndex < end);
const effectiveActiveDescendant = $derived(activeRendered ? activeDescendantId : undefined);

function onScroll(): void {
    if (viewport !== undefined) {
        scrollTop = viewport.scrollTop;
    }
}

// Near-end detection reacts to both scrolling and the window growing.
$effect(() => {
    if (items.length > 0 && end >= items.length - nearEndMargin) {
        onnearend?.();
    }
});

// Rows whose bottom edge has cleared the viewport top have "scrolled past";
// rows from there to the viewport bottom are on screen.
const firstVisibleIndex = $derived(Math.min(items.length, Math.floor(scrollTop / rowHeight)));
const endVisibleIndex = $derived(
    Math.min(items.length, Math.ceil((scrollTop + viewportHeight) / rowHeight)),
);
$effect(() => {
    void items; // a refetch that swaps rows in place re-reports too
    onviewport?.({ first: firstVisibleIndex, end: endVisibleIndex, scrollTop });
});

// Open at the handed-in offset: the DOM container starts at 0, so seat it once
// on mount (before the scroll-into-view effect below reads it). A browser that
// clamps the offset fires a scroll event, which re-syncs `scrollTop`.
let seated = false;
$effect(() => {
    if (seated || viewport === undefined) {
        return;
    }
    seated = true;
    const opening = untrack(() => scrollTop);
    if (opening > 0) {
        viewport.scrollTop = opening;
    }
});

// Entering the list from elsewhere (the owner bumps `focusNonce`): pull DOM
// focus onto the listbox itself, so Home/End/PageUp/PageDown/menu reach
// this handler instead of landing on <body>.
$effect(() => {
    if (focusNonce > 0) {
        viewport?.focus();
    }
});

// Scroll-into-view on selection change: scrollTop math, not
// `scrollIntoView` (which jsdom lacks and which overshoots overscan rows).
$effect(() => {
    const index = selectedIndex;
    if (index < 0 || index >= items.length || viewport === undefined) {
        return;
    }
    const rowTop = index * rowHeight;
    const rowBottom = rowTop + rowHeight;
    if (rowTop < viewport.scrollTop) {
        viewport.scrollTop = rowTop;
        scrollTop = rowTop;
    } else if (rowBottom > viewport.scrollTop + viewportHeight) {
        const target = rowBottom - viewportHeight;
        viewport.scrollTop = target;
        scrollTop = target;
    }
});
</script>

<div
    class="virtual-list"
    class:keyboard-mode={keyboardMode}
    role="listbox"
    aria-label={label}
    aria-activedescendant={effectiveActiveDescendant}
    tabindex="0"
    bind:this={viewport}
    bind:clientHeight={measuredHeight}
    onscroll={onScroll}
    onkeydown={onKeydown}
    onpointermove={() => {
        keyboardMode = false;
    }}
>
    <div class="virtual-spacer" role="presentation" style:height="{items.length * rowHeight}px">
        <div
            class="virtual-window"
            role="presentation"
            style:transform="translateY({start * rowHeight}px)"
        >
            {#each visible as item, offset (key(item))}
                {@render row(item, start + offset)}
            {/each}
        </div>
    </div>
</div>

<style>
    .virtual-list {
        flex: 1 1 auto;
        min-height: 0;
        overflow-y: auto;
        overflow-x: hidden;
    }

    .virtual-list:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: -2px;
    }

    /* While the keyboard drives the list, mute the stale pointer hover on the
       row the mouse last sat over (kept until the pointer actually moves) so it
       can't read as a second highlight beside the keyboard-selected row. The
       selected row keeps its own background. */
    .virtual-list.keyboard-mode :global(.article-row:hover:not(.selected)) {
        background: transparent;
    }

    .virtual-spacer {
        position: relative;
        width: 100%;
    }

    .virtual-window {
        position: absolute;
        top: 0;
        left: 0;
        right: 0;
        will-change: transform;
    }
</style>
