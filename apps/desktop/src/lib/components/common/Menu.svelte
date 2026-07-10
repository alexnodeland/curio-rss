<script lang="ts">
/**
 * A positioned menu level: renders `MenuItem[]` as `role="menuitem"` rows with
 * roving focus (Up/Down/Home/End), Enter/Space activation, Right/hover to open
 * a submenu and Left/Escape to leave it. Recursively renders itself for
 * submenus. Flips at viewport edges on mount. Positioning and dismissal
 * (outside-click, scroll) are owned by MenuHost; this component owns keyboard
 * and focus within a level.
 */
import Icon from '$components/common/Icon.svelte';
import { t } from '$lib/i18n';
import type { MenuItem } from '$lib/state/menu.svelte';
import Self from './Menu.svelte';

let {
    items,
    x,
    y,
    onclose,
    onback,
    ariaLabel,
    autofocus = true,
}: {
    items: readonly MenuItem[];
    x: number;
    y: number;
    /** Dismiss the whole menu tree and restore focus to the invoker. */
    onclose: () => void;
    /** Close this level and return focus to the parent item (submenus only). */
    onback?: () => void;
    ariaLabel?: string;
    autofocus?: boolean;
} = $props();

let menuEl = $state<HTMLElement>();
const itemEls: (HTMLButtonElement | undefined)[] = $state([]);
let activeIndex = $state(-1);
let submenuIndex = $state<number | null>(null);
let submenuPos = $state<{ x: number; y: number }>({ x: 0, y: 0 });
// Viewport-overflow correction applied after the menu is measured; 0 until
// then, so the first paint sits at the requested (x, y) with no flash.
let overflow = $state<{ x: number; y: number }>({ x: 0, y: 0 });
const left = $derived(x + overflow.x);
const top = $derived(y + overflow.y);

const enabledIndexes = $derived(
    items
        .map((item, index) => ({ item, index }))
        .filter(({ item }) => !item.disabled)
        .map(({ index }) => index),
);

function label(item: MenuItem): string {
    if (item.label !== undefined) return item.label;
    return item.labelKey !== undefined ? t(item.labelKey, item.labelParams) : '';
}

// Flip into the viewport once measured (a no-op in jsdom where sizes are 0).
$effect(() => {
    const el = menuEl;
    if (el === undefined) return;
    const { offsetWidth: w, offsetHeight: h } = el;
    if (w === 0 && h === 0) return;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const nx = x + w > vw - 4 ? Math.max(4, vw - w - 4) : x;
    const ny = y + h > vh - 4 ? Math.max(4, vh - h - 4) : y;
    overflow = { x: nx - x, y: ny - y };
});

// Move focus to the active item whenever it changes, scrolling it into view
// so arrow-nav past the fold (a long "Move to folder" list) stays visible.
$effect(() => {
    if (activeIndex >= 0) {
        const el = itemEls[activeIndex];
        el?.focus();
        el?.scrollIntoView?.({ block: 'nearest' });
    }
});

// Focus the first enabled item on mount.
$effect(() => {
    if (autofocus && activeIndex === -1 && enabledIndexes.length > 0) {
        activeIndex = enabledIndexes[0];
    }
});

function move(delta: 1 | -1): void {
    const list = enabledIndexes;
    if (list.length === 0) return;
    const at = list.indexOf(activeIndex);
    const next =
        at === -1 ? (delta > 0 ? 0 : list.length - 1) : (at + delta + list.length) % list.length;
    activeIndex = list[next];
}

function openSubmenu(index: number): void {
    const btn = itemEls[index];
    if (btn === undefined) return;
    const rect = btn.getBoundingClientRect();
    submenuPos = { x: rect.right - 4, y: rect.top - 4 };
    submenuIndex = index;
}

function closeSubmenu(): void {
    const index = submenuIndex;
    submenuIndex = null;
    if (index !== null) {
        activeIndex = index;
        itemEls[index]?.focus();
    }
}

function activate(index: number): void {
    const item = items[index];
    if (item === undefined || item.disabled) return;
    if (item.children !== undefined && item.children.length > 0) {
        openSubmenu(index);
        return;
    }
    item.onSelect?.();
    onclose();
}

function onkeydown(event: KeyboardEvent): void {
    switch (event.key) {
        case 'ArrowDown':
            event.preventDefault();
            event.stopPropagation();
            move(1);
            break;
        case 'ArrowUp':
            event.preventDefault();
            event.stopPropagation();
            move(-1);
            break;
        case 'Home':
            event.preventDefault();
            event.stopPropagation();
            if (enabledIndexes.length > 0) activeIndex = enabledIndexes[0];
            break;
        case 'End':
            event.preventDefault();
            event.stopPropagation();
            if (enabledIndexes.length > 0) activeIndex = enabledIndexes[enabledIndexes.length - 1];
            break;
        case 'ArrowRight': {
            const item = items[activeIndex];
            if (item?.children !== undefined && item.children.length > 0) {
                event.preventDefault();
                event.stopPropagation();
                openSubmenu(activeIndex);
            }
            break;
        }
        case 'ArrowLeft':
            if (onback !== undefined) {
                event.preventDefault();
                event.stopPropagation();
                onback();
            }
            break;
        case 'Enter':
        case ' ':
            event.preventDefault();
            event.stopPropagation();
            activate(activeIndex);
            break;
        case 'Escape':
            event.preventDefault();
            event.stopPropagation();
            onclose();
            break;
        case 'Tab':
            event.preventDefault();
            onclose();
            break;
        default:
            break;
    }
}
</script>

<div
    bind:this={menuEl}
    class="menu"
    role="menu"
    aria-label={ariaLabel}
    tabindex="-1"
    style="left: {left}px; top: {top}px;"
    {onkeydown}
>
    {#each items as item, index (item.id)}
        {#if item.separatorBefore && index > 0}
            <div class="menu-sep" role="separator"></div>
        {/if}
        <button
            bind:this={itemEls[index]}
            type="button"
            role="menuitem"
            class="menu-item"
            class:danger={item.danger}
            class:has-children={item.children !== undefined && item.children.length > 0}
            aria-haspopup={item.children !== undefined && item.children.length > 0 ? 'menu' : undefined}
            aria-expanded={item.children !== undefined && item.children.length > 0
                ? submenuIndex === index
                : undefined}
            aria-disabled={item.disabled ? 'true' : undefined}
            tabindex={index === activeIndex ? 0 : -1}
            disabled={item.disabled}
            onclick={() => activate(index)}
            onmouseenter={() => {
                if (!item.disabled) activeIndex = index;
                if (item.children !== undefined && item.children.length > 0) openSubmenu(index);
                else submenuIndex = null;
            }}
        >
            {#if item.icon}
                <span class="menu-icon" aria-hidden="true"><Icon name={item.icon} size={15} /></span>
            {:else}
                <span class="menu-icon" aria-hidden="true"></span>
            {/if}
            <span class="menu-label truncate">{label(item)}</span>
            {#if item.kbd}
                <span class="menu-kbd" aria-hidden="true">{item.kbd}</span>
            {/if}
            {#if item.children !== undefined && item.children.length > 0}
                <span class="menu-caret" aria-hidden="true"><Icon name="chevron" size={13} /></span>
            {/if}
        </button>
    {/each}
</div>

{#if submenuIndex !== null && items[submenuIndex]?.children}
    <Self
        items={items[submenuIndex].children ?? []}
        x={submenuPos.x}
        y={submenuPos.y}
        {onclose}
        onback={closeSubmenu}
    />
{/if}

<style>
    .menu {
        position: fixed;
        z-index: 1000;
        min-width: 200px;
        max-width: 320px;
        /* Never taller than the viewport: a long "Move to folder" list (many
           folders) scrolls instead of running its lowest items off-screen
           where they're physically unreachable. */
        max-height: calc(100vh - 16px);
        overflow-y: auto;
        padding: var(--space-1);
        background: var(--surface-overlay);
        border: 1px solid var(--hairline);
        border-radius: var(--radius-lg);
        box-shadow: var(--shadow-lg);
        display: flex;
        flex-direction: column;
        gap: 1px;
        animation: menu-in var(--dur-fast) var(--ease);
    }

    @keyframes menu-in {
        from {
            opacity: 0;
            transform: translateY(-4px) scale(0.98);
        }
    }

    .menu:focus-visible {
        outline: none;
    }

    .menu-item {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        width: 100%;
        padding: var(--space-2) var(--space-2);
        border-radius: var(--radius-sm);
        background: transparent;
        color: var(--fg);
        text-align: left;
        font-size: var(--text-md);
        line-height: 1.3;
        cursor: default;
        transition: background var(--dur-fast) var(--ease);
    }

    .menu-item:hover:not(:disabled),
    .menu-item:focus-visible {
        background: var(--hover);
        outline: none;
    }

    .menu-item:focus-visible {
        box-shadow: inset 0 0 0 1px color-mix(in srgb, var(--accent), transparent 40%);
    }

    .menu-item:disabled {
        color: var(--fg-subtle);
        cursor: default;
    }

    .menu-item.danger {
        color: var(--error-text);
    }

    .menu-item.danger:hover:not(:disabled),
    .menu-item.danger:focus-visible {
        background: var(--error-bg);
    }

    .menu-icon {
        flex: 0 0 auto;
        width: 15px;
        height: 15px;
        display: inline-grid;
        place-items: center;
        color: var(--fg-muted);
    }

    .menu-item.danger .menu-icon {
        color: var(--error-text);
    }

    .menu-label {
        flex: 1 1 auto;
        min-width: 0;
    }

    .menu-kbd {
        flex: 0 0 auto;
        margin-left: var(--space-3);
        font-size: var(--text-xs);
        color: var(--fg-subtle);
        font-variant-numeric: tabular-nums;
    }

    .menu-caret {
        flex: 0 0 auto;
        margin-left: auto;
        color: var(--fg-subtle);
        display: inline-grid;
        place-items: center;
    }

    .menu-item.has-children .menu-kbd {
        margin-left: var(--space-3);
    }

    .menu-sep {
        height: 1px;
        margin: var(--space-1) var(--space-1);
        background: var(--hairline);
    }
</style>
