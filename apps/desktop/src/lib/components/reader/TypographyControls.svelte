<script lang="ts">
/**
 * Reader typography controls: font family, size, line height, and measure,
 * all over the existing CSS token system and persisted through the settings
 * table (each change writes through `set_setting`). No local state — the
 * typographyStore is the single source of truth, clamped on write.
 */
import { rangeFill } from '$lib/actions/range-fill';
import { t } from '$lib/i18n';
import {
    READER_FONTS,
    TYPOGRAPHY_LIMITS,
    isReaderFontId,
    isReaderTextAlign,
    isReaderTheme,
    typographyStore,
} from '$lib/state/typography.svelte';

function onFont(event: Event & { currentTarget: HTMLSelectElement }): void {
    const value = event.currentTarget.value;
    if (isReaderFontId(value)) {
        void typographyStore.setFontFamily(value);
    }
}

function onSize(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setFontSize(Number(event.currentTarget.value));
}

function onLineHeight(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setLineHeight(Number(event.currentTarget.value));
}

function onMeasure(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setMeasure(Number(event.currentTarget.value));
}

function onSpacing(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setParagraphSpacing(Number(event.currentTarget.value));
}

function onAlign(event: Event & { currentTarget: HTMLSelectElement }): void {
    if (isReaderTextAlign(event.currentTarget.value)) {
        void typographyStore.setTextAlign(event.currentTarget.value);
    }
}

function onWeight(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setFontWeight(Number(event.currentTarget.value));
}

function onLetterSpacing(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setLetterSpacing(Number(event.currentTarget.value));
}

function onHyphenate(event: Event & { currentTarget: HTMLInputElement }): void {
    void typographyStore.setHyphenate(event.currentTarget.checked);
}

function onReadingTheme(event: Event & { currentTarget: HTMLSelectElement }): void {
    if (isReaderTheme(event.currentTarget.value)) {
        void typographyStore.setReadingTheme(event.currentTarget.value);
    }
}

function reset(): void {
    void typographyStore.setFontFamily('serif');
    void typographyStore.setFontSize(TYPOGRAPHY_LIMITS.fontSize.default);
    void typographyStore.setLineHeight(TYPOGRAPHY_LIMITS.lineHeight.default);
    void typographyStore.setMeasure(TYPOGRAPHY_LIMITS.measure.default);
    void typographyStore.setParagraphSpacing(TYPOGRAPHY_LIMITS.paragraphSpacing.default);
    void typographyStore.setTextAlign('left');
    void typographyStore.setFontWeight(TYPOGRAPHY_LIMITS.fontWeight.default);
    void typographyStore.setLetterSpacing(TYPOGRAPHY_LIMITS.letterSpacing.default);
    void typographyStore.setHyphenate(false);
    void typographyStore.setReadingTheme('default');
}
</script>

<div class="typography" role="group" aria-label={t('typography.title')}>
    <label class="row">
        <span class="row-label">{t('typography.font')}</span>
        <select value={typographyStore.fontFamily} onchange={onFont}>
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
            value={typographyStore.fontSize}
            oninput={onSize}
            use:rangeFill={typographyStore.fontSize}
        />
        <span class="row-value">{typographyStore.fontSize}px</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.lineHeight')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.lineHeight.min}
            max={TYPOGRAPHY_LIMITS.lineHeight.max}
            step="0.1"
            value={typographyStore.lineHeight}
            oninput={onLineHeight}
            use:rangeFill={typographyStore.lineHeight}
        />
        <span class="row-value">{typographyStore.lineHeight.toFixed(1)}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.measure')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.measure.min}
            max={TYPOGRAPHY_LIMITS.measure.max}
            step="20"
            value={typographyStore.measure}
            oninput={onMeasure}
            use:rangeFill={typographyStore.measure}
        />
        <span class="row-value">{typographyStore.measure}px</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.spacing')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.paragraphSpacing.min}
            max={TYPOGRAPHY_LIMITS.paragraphSpacing.max}
            step="0.1"
            value={typographyStore.paragraphSpacing}
            oninput={onSpacing}
            use:rangeFill={typographyStore.paragraphSpacing}
        />
        <span class="row-value">{typographyStore.paragraphSpacing.toFixed(1)}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.align')}</span>
        <select value={typographyStore.textAlign} onchange={onAlign}>
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
            value={typographyStore.fontWeight}
            oninput={onWeight}
            use:rangeFill={typographyStore.fontWeight}
        />
        <span class="row-value">{typographyStore.fontWeight}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.letterSpacing')}</span>
        <input
            type="range"
            min={TYPOGRAPHY_LIMITS.letterSpacing.min}
            max={TYPOGRAPHY_LIMITS.letterSpacing.max}
            step="0.005"
            value={typographyStore.letterSpacing}
            oninput={onLetterSpacing}
            use:rangeFill={typographyStore.letterSpacing}
        />
        <span class="row-value">{typographyStore.letterSpacing.toFixed(3)}</span>
    </label>

    <label class="row">
        <span class="row-label">{t('typography.readingTheme')}</span>
        <select value={typographyStore.readingTheme} onchange={onReadingTheme}>
            <option value="default">{t('typography.readingTheme.default')}</option>
            <option value="sepia">{t('typography.readingTheme.sepia')}</option>
            <option value="paper">{t('typography.readingTheme.paper')}</option>
        </select>
    </label>

    <label class="row row-toggle">
        <span class="row-label">{t('typography.hyphens')}</span>
        <input type="checkbox" checked={typographyStore.hyphenate} onchange={onHyphenate} />
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
