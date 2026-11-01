<script lang="ts">
/**
 * The single mount point for the app's one-at-a-time tooltip (driven by the
 * `tooltip` action via `tooltipStore`). Positions above the trigger by
 * default, flipping below when there is no room; centred on the trigger,
 * then nudged so the whole box stays on screen (a trigger hugging the window
 * edge — the sidebar's offline badge — would otherwise push half of it off).
 * `pointer-events: none` so it never blocks the control under it.
 */
import { tooltipStore } from '$lib/state/tooltip.svelte';

const ABOVE_MIN_TOP = 44;
const GAP = 6;
/** Minimum distance between the tooltip box and the viewport edge. */
const EDGE = 8;

/** The rendered tooltip's width (0 until first laid out). */
let width = $state(0);

const placement = $derived.by(() => {
    const active = tooltipStore.current;
    if (active === null) return null;
    const { rect } = active;
    const above = rect.top >= ABOVE_MIN_TOP;
    const half = width / 2;
    const centerX = Math.min(
        window.innerWidth - EDGE - half,
        Math.max(EDGE + half, rect.left + rect.width / 2),
    );
    return {
        left: centerX,
        top: above ? rect.top - GAP : rect.bottom + GAP,
        above,
    };
});

function onScroll(): void {
    tooltipStore.hideAll();
}
</script>

<svelte:window onscrollcapture={onScroll} onresize={onScroll} onblur={onScroll} />

{#if tooltipStore.current !== null && placement !== null}
    <div
        id={tooltipStore.id}
        class="tooltip"
        class:above={placement.above}
        role="tooltip"
        bind:offsetWidth={width}
        style="left: {placement.left}px; top: {placement.top}px;"
    >
        <span class="tooltip-text">{tooltipStore.current.text}</span>
        {#if tooltipStore.current.kbd}
            <kbd class="tooltip-kbd">{tooltipStore.current.kbd}</kbd>
        {/if}
    </div>
{/if}

<style>
    .tooltip {
        position: fixed;
        z-index: 1100;
        transform: translate(-50%, 0);
        pointer-events: none;
        display: inline-flex;
        align-items: center;
        gap: var(--space-2);
        max-width: 280px;
        padding: 5px var(--space-2);
        border-radius: var(--radius-sm);
        background: var(--surface-raised);
        border: 1px solid var(--hairline);
        box-shadow: var(--shadow-md);
        color: var(--fg);
        font-size: var(--text-xs);
        line-height: 1.35;
        white-space: nowrap;
        animation: tooltip-in var(--dur-fast) var(--ease);
    }

    .tooltip.above {
        transform: translate(-50%, -100%);
    }

    @keyframes tooltip-in {
        from {
            opacity: 0;
        }
    }

    .tooltip-kbd {
        flex: 0 0 auto;
        padding: 1px 5px;
        border-radius: var(--radius-sm);
        background: var(--surface-inset);
        border: 1px solid var(--hairline);
        font-family: var(--font-mono);
        font-size: 0.6875rem;
        color: var(--fg-muted);
        font-variant-numeric: tabular-nums;
    }
</style>
