<script lang="ts">
/**
 * The reader's frame: sidebar | article list | reader, the first two
 * resizable with widths owned by `layoutStore` and persisted to the settings
 * table on resize end. The reader takes whatever remains.
 *
 * The two left panes can leave the screen — the sidebar collapsed (by hand,
 * or on its own in a narrow window), both in focus mode. A hidden pane stays
 * mounted (its scroll position and tree state survive) but slides out past
 * the frame's leading edge and turns `inert`, so neither Tab nor a click nor
 * assistive tech can reach it, and any focus it held moves to the next
 * visible pane. The slide runs only once the layout has changed in this
 * session (never while the persisted layout is adopted at launch), and the
 * global reduced-motion rule collapses it to an instant swap.
 */
import { t } from '$lib/i18n';
import { SETTING_KEYS, settingsStore } from '$lib/state/settings.svelte';
import { PANE_LIMITS, layoutStore } from '$lib/state/layout.svelte';
import { evacuateHiddenPanes } from '$lib/state/pane-focus';
import { type Snippet, untrack } from 'svelte';
import ResizablePane from './ResizablePane.svelte';

let {
    sidebar,
    list,
    reader,
}: {
    sidebar: Snippet;
    list: Snippet;
    reader: Snippet;
} = $props();

/** Each slot's rendered width: how far it slides to clear the frame. */
let sidebarSlotWidth = $state(0);
let listSlotWidth = $state(0);

function persistSidebarWidth(width: number): void {
    void settingsStore.set(SETTING_KEYS.sidebarWidth, String(Math.round(width)));
}

function persistListWidth(width: number): void {
    void settingsStore.set(SETTING_KEYS.listWidth, String(Math.round(width)));
}

/** The leading margin that parks a hidden slot just past the frame's edge. */
function parked(visible: boolean, width: number): string {
    return visible ? '0px' : `-${width}px`;
}

// The narrow-window auto-collapse follows the viewport for the frame's life.
$effect(() => layoutStore.watchViewport());

// Whenever a pane leaves the screen, take the keyboard out of it.
$effect(() => {
    void layoutStore.sidebarVisible;
    void layoutStore.listVisible;
    untrack(() => evacuateHiddenPanes());
});
</script>

<div class="three-pane" class:motion={layoutStore.paneMotion} data-testid="three-pane">
    <div
        class="pane-slot"
        class:parked={!layoutStore.sidebarVisible}
        data-pane="sidebar"
        inert={!layoutStore.sidebarVisible}
        bind:clientWidth={sidebarSlotWidth}
        style:margin-inline-start={parked(layoutStore.sidebarVisible, sidebarSlotWidth)}
    >
        <ResizablePane
            bind:width={layoutStore.sidebarWidth}
            min={PANE_LIMITS.sidebar.min}
            max={PANE_LIMITS.sidebar.max}
            label={t('pane.sidebar.resize')}
            variant="sidebar"
            onresizeend={persistSidebarWidth}
        >
            {@render sidebar()}
        </ResizablePane>
    </div>
    <div
        class="pane-slot"
        class:parked={!layoutStore.listVisible}
        data-pane="list"
        inert={!layoutStore.listVisible}
        bind:clientWidth={listSlotWidth}
        style:margin-inline-start={parked(layoutStore.listVisible, listSlotWidth)}
    >
        <ResizablePane
            bind:width={layoutStore.listWidth}
            min={PANE_LIMITS.list.min}
            max={PANE_LIMITS.list.max}
            label={t('pane.list.resize')}
            variant="list"
            onresizeend={persistListWidth}
        >
            {@render list()}
        </ResizablePane>
    </div>
    <section class="reader-slot">
        {@render reader()}
    </section>
</div>

<style>
    .three-pane {
        display: flex;
        height: 100vh;
        overflow: hidden;
        background: var(--bg);
    }

    /* A slot is exactly its pane plus the drag handle; parking it is a
       negative leading margin of its own width, so the pane slides out whole
       (no reflow inside it) while the reader widens into the space. */
    .pane-slot {
        flex: 0 0 auto;
        display: flex;
        min-width: 0;
        min-height: 0;
    }

    /* Once parked it is not painted at all: `visibility` flips at the end of
       the slide (and at once without motion), so no sliver of a hidden pane
       can show, even before the slot has been measured. */
    .pane-slot.parked {
        visibility: hidden;
    }

    .three-pane.motion .pane-slot {
        transition:
            margin-inline-start var(--dur-base) var(--ease),
            visibility 0s linear var(--dur-base);
    }

    .three-pane.motion .pane-slot:not(.parked) {
        transition:
            margin-inline-start var(--dur-base) var(--ease),
            visibility 0s;
    }

    .reader-slot {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
    }

    /* Responsive degradation. The two left panes are fixed-width (flex 0 0), so
       at the enforced 800px minimum they leave the reader ~150px — a broken,
       mid-word-breaking column. At these widths the sidebar folds away on its
       own (`NARROW_QUERY`), and each pane is capped with max-width (which
       overrides the inline flex-basis) so the reader keeps a legible width all
       the way down to 800px even with the sidebar peeked open, without
       touching the user's stored widths on larger screens. */
    @media (max-width: 1000px) {
        .three-pane :global(.pane.list) {
            max-width: 34vw;
        }
    }

    @media (max-width: 900px) {
        .three-pane :global(.pane.sidebar) {
            max-width: 24vw;
        }
        .three-pane :global(.pane.list) {
            max-width: 30vw;
        }
    }
</style>
