<script lang="ts">
/**
 * The reader's frame: sidebar | article list | reader, the first two
 * resizable with widths owned by `uiStore` and persisted to the settings
 * table on resize end. The reader takes whatever remains.
 */
import { t } from '$lib/i18n';
import { SETTING_KEYS, settingsStore } from '$lib/state/settings.svelte';
import { PANE_LIMITS, uiStore } from '$lib/state/ui.svelte';
import type { Snippet } from 'svelte';
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

function persistSidebarWidth(width: number): void {
    void settingsStore.set(SETTING_KEYS.sidebarWidth, String(Math.round(width)));
}

function persistListWidth(width: number): void {
    void settingsStore.set(SETTING_KEYS.listWidth, String(Math.round(width)));
}
</script>

<div class="three-pane">
    {#if !uiStore.sidebarCollapsed}
        <ResizablePane
            bind:width={uiStore.sidebarWidth}
            min={PANE_LIMITS.sidebar.min}
            max={PANE_LIMITS.sidebar.max}
            label={t('pane.sidebar.resize')}
            onresizeend={persistSidebarWidth}
        >
            {@render sidebar()}
        </ResizablePane>
    {/if}
    <ResizablePane
        bind:width={uiStore.listWidth}
        min={PANE_LIMITS.list.min}
        max={PANE_LIMITS.list.max}
        label={t('pane.list.resize')}
        onresizeend={persistListWidth}
    >
        {@render list()}
    </ResizablePane>
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

    .reader-slot {
        flex: 1 1 auto;
        display: flex;
        flex-direction: column;
        min-width: 0;
        min-height: 0;
        overflow: hidden;
    }
</style>
