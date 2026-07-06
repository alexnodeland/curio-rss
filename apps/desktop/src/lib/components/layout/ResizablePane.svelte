<script lang="ts">
/**
 * One pane plus the drag handle on its trailing edge. The handle is a real
 * ARIA separator: pointer-draggable and keyboard-resizable (arrow keys,
 * Home/End), reporting the settled width via `onresizeend` so the owner
 * can persist it.
 */
import type { Snippet } from 'svelte';

let {
    width = $bindable(280),
    min,
    max,
    label,
    onresizeend,
    children,
}: {
    width?: number;
    min: number;
    max: number;
    label: string;
    onresizeend?: (width: number) => void;
    children: Snippet;
} = $props();

const KEYBOARD_STEP = 16;

let dragging = $state(false);
let dragStartX = 0;
let dragStartWidth = 0;

function clamp(value: number): number {
    return Math.min(max, Math.max(min, value));
}

function onPointerDown(event: PointerEvent): void {
    dragging = true;
    dragStartX = event.clientX;
    dragStartWidth = width;
    (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
}

function onPointerMove(event: PointerEvent): void {
    if (!dragging) {
        return;
    }
    width = clamp(dragStartWidth + (event.clientX - dragStartX));
}

function onPointerUp(): void {
    if (!dragging) {
        return;
    }
    dragging = false;
    onresizeend?.(width);
}

function onKeyDown(event: KeyboardEvent): void {
    let next: number | null = null;
    if (event.key === 'ArrowLeft') {
        next = clamp(width - KEYBOARD_STEP);
    } else if (event.key === 'ArrowRight') {
        next = clamp(width + KEYBOARD_STEP);
    } else if (event.key === 'Home') {
        next = min;
    } else if (event.key === 'End') {
        next = max;
    }
    if (next === null || next === width) {
        return;
    }
    event.preventDefault();
    width = next;
    onresizeend?.(width);
}
</script>

<div class="pane" style:flex="0 0 {width}px" style:width="{width}px">
    {@render children()}
</div>
<!--
  The ARIA "window splitter" pattern: a focusable separator with
  aria-valuenow IS interactive (WAI-ARIA APG), but svelte's a11y table
  treats separator as static — hence the two targeted ignores.
-->
<!-- svelte-ignore a11y_no_noninteractive_tabindex -->
<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
    class="pane-handle"
    class:dragging
    role="separator"
    aria-orientation="vertical"
    aria-label={label}
    aria-valuenow={Math.round(width)}
    aria-valuemin={min}
    aria-valuemax={max}
    tabindex="0"
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    onkeydown={onKeyDown}
></div>

<style>
    .pane {
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
    }

    .pane-handle {
        flex: 0 0 5px;
        margin: 0 -2px;
        cursor: col-resize;
        background: transparent;
        border-left: 1px solid var(--border-subtle);
        transition: background 0.15s ease;
        z-index: 1;
    }

    .pane-handle:hover,
    .pane-handle.dragging {
        background: var(--accent-muted);
    }

    .pane-handle:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: -1px;
    }
</style>
