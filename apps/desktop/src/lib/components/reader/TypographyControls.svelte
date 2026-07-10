<script lang="ts">
/**
 * Reader typography controls: font family, size, line height, and measure,
 * all over the existing CSS token system and persisted through the settings
 * table (each change writes through `set_setting`). No local state — the
 * uiStore is the single source of truth, clamped on write.
 */
import { rangeFill } from '$lib/actions/range-fill';
import { t } from '$lib/i18n';
import {
    READER_FONTS,
    TYPOGRAPHY_LIMITS,
    isReaderFontId,
    isReaderTextAlign,
    isReaderTheme,
    uiStore,
} from '$lib/state/ui.svelte';

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

function onSpacing(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setParagraphSpacing(Number(event.currentTarget.value));
}

function onAlign(event: Event & { currentTarget: HTMLSelectElement }): void {
    if (isReaderTextAlign(event.currentTarget.value)) {
        void uiStore.setTextAlign(event.currentTarget.value);
    }
}

function onWeight(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setFontWeight(Number(event.currentTarget.value));
}

function onLetterSpacing(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setLetterSpacing(Number(event.currentTarget.value));
}

function onHyphenate(event: Event & { currentTarget: HTMLInputElement }): void {
    void uiStore.setHyphenate(event.currentTarget.checked);
}

function onReadingTheme(event: Event & { currentTarget: HTMLSelectElement }): void {
    if (isReaderTheme(event.currentTarget.value)) {
        void uiStore.setReadingTheme(event.currentTarget.value);
    }
}

function reset(): void {
    void uiStore.setFontFamily('serif');
    void uiStore.setFontSize(TYPOGRAPHY_LIMITS.fontSize.default);
    void uiStore.setLineHeight(TYPOGRAPHY_LIMITS.lineHeight.default);
    void uiStore.setMeasure(TYPOGRAPHY_LIMITS.measure.default);
    void uiStore.setParagraphSpacing(TYPOGRAPHY_LIMITS.paragraphSpacing.default);
    void uiStore.setTextAlign('left');
    void uiStore.setFontWeight(TYPOGRAPHY_LIMITS.fontWeight.default);
    void uiStore.setLetterSpacing(TYPOGRAPHY_LIMITS.letterSpacing.default);
    void uiStore.setHyphenate(false);
    void uiStore.setReadingTheme('default');
}
</script>

<div class="typography" role="group" aria-label={t('typography.title')}>
    <label class="row">
        <span class="row-label">{t('typography.font')}</span>
        <select value={uiStore.fontFamily} onchange={onFont}>
            {#each READER_FONTS as font (font.id)}
                <option value={font.id} style="font-family: {font.stack}">{font.name}</option>
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
            use:rangeFill={uiStore.fontSize}
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
            use:rangeFill={uiStore.lineHeight}
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
            use:rangeFill={uiStore.measure}
        />
        <span class="row-value">{uiStore.measure}px</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.spacing')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.paragraphSpacing.min}
            max={TYPOGRAPHY_LIMITS.paragraphSpacing.max}
            step="0.1"
            value={uiStore.paragraphSpacing}
            oninput={onSpacing}
            use:rangeFill={uiStore.paragraphSpacing}
        />
        <span class="row-value">{uiStore.paragraphSpacing.toFixed(1)}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.align')}</span>
        <select value={uiStore.textAlign} onchange={onAlign}>
            <option value="left">{t('typography.align.left')}</option>
            <option value="justify">{t('typography.align.justify')}</option>
        </select>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.weight')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.fontWeight.min}
            max={TYPOGRAPHY_LIMITS.fontWeight.max}
            step="25"
            value={uiStore.fontWeight}
            oninput={onWeight}
            use:rangeFill={uiStore.fontWeight}
        />
        <span class="row-value">{uiStore.fontWeight}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.letterSpacing')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.letterSpacing.min}
            max={TYPOGRAPHY_LIMITS.letterSpacing.max}
            step="0.005"
            value={uiStore.letterSpacing}
            oninput={onLetterSpacing}
            use:rangeFill={uiStore.letterSpacing}
        />
        <span class="row-value">{uiStore.letterSpacing.toFixed(3)}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.readingTheme')}</span>
        <select value={uiStore.readingTheme} onchange={onReadingTheme}>
            <option value="default">{t('typography.readingTheme.default')}</option>
            <option value="sepia">{t('typography.readingTheme.sepia')}</option>
            <option value="paper">{t('typography.readingTheme.paper')}</option>
        </select>
    </label>

    <label class="row row-toggle">
        <span class="row-label">{t('typography.hyphens')}</span>
        <input type="checkbox" checked={uiStore.hyphenate} onchange={onHyphenate} />
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

    /* The hyphenation toggle: a checkbox sitting in the control column. */
    .row-toggle input {
        justify-self: start;
        width: 1.1rem;
        height: 1.1rem;
        accent-color: var(--accent);
    }

    select {
        padding: var(--space-1) calc(var(--space-3) + 1em) var(--space-1) var(--space-2);
        border-radius: var(--radius-md);
        background-color: var(--surface-inset);
        color: var(--fg);
        border: 1px solid var(--hairline);
    }

    /* The bespoke range treatment is global (app.css); the fill tracks each
       slider's value via use:rangeFill. */

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
