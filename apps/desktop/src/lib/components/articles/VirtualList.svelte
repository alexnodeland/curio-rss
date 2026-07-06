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
 */
import type { Snippet } from 'svelte';

let {
    items,
    rowHeight,
    key,
    selectedIndex = -1,
    overscan = 4,
    nearEndMargin = 10,
    fallbackViewportHeight = 600,
    label,
    onnearend,
    row,
}: {
    items: readonly T[];
    rowHeight: number;
    key: (item: T) => string | number;
    selectedIndex?: number;
    overscan?: number;
    nearEndMargin?: number;
    fallbackViewportHeight?: number;
    label: string;
    onnearend?: () => void;
    row: Snippet<[T, number]>;
} = $props();

let viewport: HTMLDivElement | undefined = $state();
let measuredHeight = $state(0);
let scrollTop = $state(0);

const viewportHeight = $derived(measuredHeight > 0 ? measuredHeight : fallbackViewportHeight);
const start = $derived(Math.max(0, Math.floor(scrollTop / rowHeight) - overscan));
const end = $derived(
    Math.min(items.length, Math.ceil((scrollTop + viewportHeight) / rowHeight) + overscan),
);
const visible = $derived(items.slice(start, end));

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
    role="listbox"
    aria-label={label}
    tabindex="0"
    bind:this={viewport}
    bind:clientHeight={measuredHeight}
    onscroll={onScroll}
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
