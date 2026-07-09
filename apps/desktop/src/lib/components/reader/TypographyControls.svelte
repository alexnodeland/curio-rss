<script lang="ts">
/**
 * Reader typography controls: font family, size, line height, and measure,
 * all over the existing CSS token system and persisted through the settings
 * table (each change writes through `set_setting`). No local state — the
 * uiStore is the single source of truth, clamped on write.
 */
import { t } from '$lib/i18n';
import { READER_FONTS, TYPOGRAPHY_LIMITS, isReaderFontId, uiStore } from '$lib/state/ui.svelte';

function onFont(event: Event & { currentTarget: HTMLSelectElement }): void {
    const value = event.currentTarget.value;
    if (isReaderFontId(value)) {
        void uiStore.setFontFamily(value);
    }
}

function onSize(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setFontSize(Number(event.currentTarget.value));
}

function onLineHeight(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setLineHeight(Number(event.currentTarget.value));
}

function onMeasure(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setMeasure(Number(event.currentTarget.value));
}

function reset(): void {
    void uiStore.setFontFamily('serif');
    void uiStore.setFontSize(TYPOGRAPHY_LIMITS.fontSize.default);
    void uiStore.setLineHeight(TYPOGRAPHY_LIMITS.lineHeight.default);
    void uiStore.setMeasure(TYPOGRAPHY_LIMITS.measure.default);
}
</script>

<div class="typography" role="group" aria-label={t('typography.title')}>
    <label class="row">
        <span class="row-label">{t('typography.font')}</span>
        <select value={uiStore.fontFamily} onchange={onFont}>
            {#each READER_FONTS as font (font.id)}
                <option value={font.id}>{font.name}</option>
            {/each}
        </select>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.size')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.fontSize.min}
            max={TYPOGRAPHY_LIMITS.fontSize.max}
            step="1"
            value={uiStore.fontSize}
            oninput={onSize}
        />
        <span class="row-value">{uiStore.fontSize}px</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.lineHeight')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.lineHeight.min}
            max={TYPOGRAPHY_LIMITS.lineHeight.max}
            step="0.1"
            value={uiStore.lineHeight}
            oninput={onLineHeight}
        />
        <span class="row-value">{uiStore.lineHeight.toFixed(1)}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.measure')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.measure.min}
            max={TYPOGRAPHY_LIMITS.measure.max}
            step="20"
            value={uiStore.measure}
            oninput={onMeasure}
        />
        <span class="row-value">{uiStore.measure}px</span>
    </label>

    <button class="reset" type="button" onclick={reset}>{t('typography.reset')}</button>
</div>

<style>
    .typography {
        display: flex;
        flex-direction: column;
        gap: var(--space-4);
        padding: var(--space-5);
        min-width: 288px;
    }

    .row {
        display: grid;
        grid-template-columns: 5rem 1fr 3rem;
        align-items: center;
        gap: var(--space-3);
        font-size: var(--text-md);
        color: var(--fg);
    }

    .row-label {
        color: var(--fg-muted);
    }

    .row-value {
        text-align: right;
        font-variant-numeric: tabular-nums;
        color: var(--fg-muted);
    }

    select {
        padding: var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background: var(--surface-inset);
        color: var(--fg);
        border: 1px solid var(--hairline);
    }

    input[type='range'] {
        width: 100%;
        accent-color: var(--accent);
    }

    .reset {
        align-self: flex-end;
        padding: var(--space-1) var(--space-3);
        border-radius: var(--radius-md);
        background: transparent;
        color: var(--fg-muted);
        border: 1px solid var(--hairline-strong);
        font-size: var(--text-sm);
        transition:
            background var(--dur-fast) var(--ease),
            color var(--dur-fast) var(--ease);
    }

    .reset:hover {
        background: var(--hover);
        color: var(--fg);
    }
</style>
