/**
 * Typography store: persisted init (clamped, unknown ids ignored), the
 * clamping setters, font-stack resolution, the value guards, and reset. The
 * controls that drive it live in typography-controls.test.ts.
 */
import { settingsStore } from '$lib/state/settings.svelte';
import {
    READER_FONTS,
    TYPOGRAPHY_LIMITS,
    TypographyStore,
    isReaderFontId,
    isReaderTextAlign,
    isReaderTheme,
} from '$lib/state/typography.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

describe('typography store', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('initTypography adopts every persisted value, clamped into range', async () => {
        const persisted: Record<string, string> = {
            'ui.typography.paragraph-spacing': '5', // above max
            'ui.typography.text-align': 'justify',
            'ui.typography.font-weight': '100', // below min
            'ui.typography.letter-spacing': '0.05',
            'ui.typography.hyphenate': 'true',
            'ui.typography.reading-theme': 'sepia',
            'ui.typography.font-family': 'charter',
        };
        harness = installIpcHarness({
            get_setting: (args) => persisted[args.key as string] ?? null,
        });
        await settingsStore.load();

        const ui = new TypographyStore();
        ui.initTypography();
        expect(ui.paragraphSpacing).toBe(TYPOGRAPHY_LIMITS.paragraphSpacing.max);
        expect(ui.textAlign).toBe('justify');
        expect(ui.fontWeight).toBe(TYPOGRAPHY_LIMITS.fontWeight.min);
        expect(ui.letterSpacing).toBeCloseTo(0.05);
        expect(ui.hyphenate).toBe(true);
        expect(ui.readingTheme).toBe('sepia');
        expect(ui.fontFamily).toBe('charter');
        // Unset keys keep their defaults.
        expect(ui.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.default);
    });

    it('initTypography ignores garbage and unknown ids', async () => {
        harness = installIpcHarness({ get_setting: () => 'nonsense' });
        await settingsStore.load();

        const ui = new TypographyStore();
        ui.initTypography();
        expect(ui.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.default);
        expect(ui.lineHeight).toBe(TYPOGRAPHY_LIMITS.lineHeight.default);
        expect(ui.fontFamily).toBe('serif');
        expect(ui.textAlign).toBe('left');
        expect(ui.readingTheme).toBe('default');
        expect(ui.hyphenate).toBe(false);
    });

    it('the numeric setters clamp before persisting', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new TypographyStore();

        await ui.setFontSize(99);
        await ui.setLineHeight(0.5);
        await ui.setMeasure(2000);
        await ui.setParagraphSpacing(0);
        await ui.setFontWeight(900);
        await ui.setLetterSpacing(-1);

        expect(ui.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.max);
        expect(ui.lineHeight).toBe(TYPOGRAPHY_LIMITS.lineHeight.min);
        expect(ui.measure).toBe(TYPOGRAPHY_LIMITS.measure.max);
        expect(ui.paragraphSpacing).toBe(TYPOGRAPHY_LIMITS.paragraphSpacing.min);
        expect(ui.fontWeight).toBe(TYPOGRAPHY_LIMITS.fontWeight.max);
        expect(ui.letterSpacing).toBe(TYPOGRAPHY_LIMITS.letterSpacing.min);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.typography.font-size', value: '24' },
            { key: 'ui.typography.line-height', value: '1.3' },
            { key: 'ui.typography.measure', value: '960' },
            { key: 'ui.typography.paragraph-spacing', value: '0.4' },
            { key: 'ui.typography.font-weight', value: '700' },
            { key: 'ui.typography.letter-spacing', value: '-0.02' },
        ]);
    });

    it('the enum setters persist the chosen value', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new TypographyStore();

        await ui.setFontFamily('mono');
        await ui.setTextAlign('justify');
        await ui.setHyphenate(true);
        await ui.setReadingTheme('paper');

        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.typography.font-family', value: 'mono' },
            { key: 'ui.typography.text-align', value: 'justify' },
            { key: 'ui.typography.hyphenate', value: 'true' },
            { key: 'ui.typography.reading-theme', value: 'paper' },
        ]);
    });

    it('readerFontStack resolves the chosen font', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new TypographyStore();
        expect(ui.readerFontStack).toBe(READER_FONTS.find((font) => font.id === 'serif')?.stack);

        await ui.setFontFamily('mono');
        expect(ui.readerFontStack).toBe('var(--font-mono)');
    });

    it('the value guards accept only the shipped options', () => {
        expect(isReaderFontId('georgia')).toBe(true);
        expect(isReaderFontId('papyrus')).toBe(false);
        expect(isReaderTextAlign('justify')).toBe(true);
        expect(isReaderTextAlign('center')).toBe(false);
        expect(isReaderTheme('sepia')).toBe(true);
        expect(isReaderTheme('neon')).toBe(false);
    });

    it('reset restores the reader font to the state default (serif, not sans)', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new TypographyStore();

        // The constructed default is the invariant reset must land back on.
        const defaultFont = ui.fontFamily;
        expect(defaultFont).toBe('serif');

        await ui.setFontFamily('mono');
        expect(ui.fontFamily).toBe('mono');

        ui.reset();
        expect(ui.fontFamily).toBe(defaultFont);
        expect(ui.fontFamily).toBe('serif');
    });

    it('reset restores size, line height, and measure', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new TypographyStore();
        await ui.setFontSize(20);
        await ui.setLineHeight(1.9);
        await ui.setMeasure(600);

        ui.reset();
        expect(ui.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.default);
        expect(ui.lineHeight).toBe(TYPOGRAPHY_LIMITS.lineHeight.default);
        expect(ui.measure).toBe(TYPOGRAPHY_LIMITS.measure.default);
    });
});
