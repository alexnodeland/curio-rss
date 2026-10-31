/**
 * Reader typography: the typographyStore's persisted-preference methods (init +
 * clamping + font-stack resolution) and the controls that drive them. Each
 * control writes through `set_setting`; init reads back through the settings
 * mirror, clamped into range.
 */
import TypographyControls from '$components/reader/TypographyControls.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { TYPOGRAPHY_LIMITS, typographyStore } from '$lib/state/typography.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness } from './ipc-harness';
import { resetUiStores } from './ui-stores';

describe('reader typography', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        resetUiStores();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('adopts persisted typography at init, clamped into range', async () => {
        const stored: Record<string, string> = {
            'ui.typography.font-size': '100', // above max → clamps down
            'ui.typography.line-height': '1.8',
            'ui.typography.measure': '10', // below min → clamps up
            'ui.typography.font-family': 'serif',
        };
        harness = installIpcHarness({ get_setting: (args) => stored[args.key as string] ?? null });
        await settingsStore.load();
        typographyStore.initTypography();

        expect(typographyStore.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.max);
        expect(typographyStore.lineHeight).toBeCloseTo(1.8);
        expect(typographyStore.measure).toBe(TYPOGRAPHY_LIMITS.measure.min);
        expect(typographyStore.fontFamily).toBe('serif');
        expect(typographyStore.readerFontStack).toContain('serif');
    });

    it('ignores an unknown persisted font id', async () => {
        harness = installIpcHarness({ get_setting: () => 'comic-sans' });
        await settingsStore.load();
        typographyStore.initTypography();
        expect(typographyStore.fontFamily).toBe('serif');
    });

    it('controls write through and persist each preference', async () => {
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        const { getAllByRole } = render(TypographyControls);

        // Two selects: font (first) and text-align (second).
        const [fontSelect, alignSelect] = getAllByRole('combobox');
        await fireEvent.change(fontSelect, { target: { value: 'mono' } });
        await flushIpc();
        expect(typographyStore.fontFamily).toBe('mono');

        // Four sliders in order: size, line height, measure, paragraph spacing.
        const [sizeRange, lineRange, measureRange, spacingRange] = getAllByRole('slider');
        await fireEvent.input(sizeRange, { target: { value: '20' } });
        await fireEvent.input(lineRange, { target: { value: '1.4' } });
        await fireEvent.input(measureRange, { target: { value: '820' } });
        await fireEvent.input(spacingRange, { target: { value: '1.5' } });
        await fireEvent.change(alignSelect, { target: { value: 'justify' } });
        await flushIpc();

        expect(typographyStore.fontSize).toBe(20);
        expect(typographyStore.lineHeight).toBeCloseTo(1.4);
        expect(typographyStore.measure).toBe(820);
        expect(typographyStore.paragraphSpacing).toBeCloseTo(1.5);
        expect(typographyStore.textAlign).toBe('justify');

        const writes = harness.callsFor('set_setting');
        expect(writes).toContainEqual({ key: 'ui.typography.font-family', value: 'mono' });
        expect(writes).toContainEqual({ key: 'ui.typography.font-size', value: '20' });
        expect(writes).toContainEqual({ key: 'ui.typography.measure', value: '820' });
        expect(writes).toContainEqual({ key: 'ui.typography.paragraph-spacing', value: '1.5' });
        expect(writes).toContainEqual({ key: 'ui.typography.text-align', value: 'justify' });
    });

    it('the reading controls write through and persist weight/tracking/theme/hyphens', async () => {
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        const { getAllByRole } = render(TypographyControls);

        // Sliders 5 and 6 are the new weight + letter-spacing controls.
        const sliders = getAllByRole('slider');
        await fireEvent.input(sliders[4], { target: { value: '600' } });
        await fireEvent.input(sliders[5], { target: { value: '0.05' } });
        // The reading-theme select is the third combobox (font, align, theme).
        const themeSelect = getAllByRole('combobox')[2];
        await fireEvent.change(themeSelect, { target: { value: 'sepia' } });
        // The hyphenation checkbox.
        await fireEvent.click(getAllByRole('checkbox')[0]);
        await flushIpc();

        expect(typographyStore.fontWeight).toBe(600);
        expect(typographyStore.letterSpacing).toBeCloseTo(0.05);
        expect(typographyStore.readingTheme).toBe('sepia');
        expect(typographyStore.hyphenate).toBe(true);

        const writes = harness.callsFor('set_setting');
        expect(writes).toContainEqual({ key: 'ui.typography.font-weight', value: '600' });
        expect(writes).toContainEqual({ key: 'ui.typography.letter-spacing', value: '0.05' });
        expect(writes).toContainEqual({ key: 'ui.typography.reading-theme', value: 'sepia' });
        expect(writes).toContainEqual({ key: 'ui.typography.hyphenate', value: 'true' });
    });

    it('adopts the persisted reading controls at init, clamped', async () => {
        const stored: Record<string, string> = {
            'ui.typography.font-weight': '9000', // above max → clamps down
            'ui.typography.letter-spacing': '0.03',
            'ui.typography.hyphenate': 'true',
            'ui.typography.reading-theme': 'paper',
        };
        harness = installIpcHarness({ get_setting: (args) => stored[args.key as string] ?? null });
        await settingsStore.load();
        typographyStore.initTypography();

        expect(typographyStore.fontWeight).toBe(TYPOGRAPHY_LIMITS.fontWeight.max);
        expect(typographyStore.letterSpacing).toBeCloseTo(0.03);
        expect(typographyStore.hyphenate).toBe(true);
        expect(typographyStore.readingTheme).toBe('paper');
    });

    it('reset restores the defaults', async () => {
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        await typographyStore.setFontSize(22);
        await typographyStore.setFontFamily('mono');
        await typographyStore.setFontWeight(700);
        await typographyStore.setHyphenate(true);
        await typographyStore.setReadingTheme('sepia');
        const { getByRole } = render(TypographyControls);

        await fireEvent.click(getByRole('button', { name: 'Reset' }));
        await flushIpc();

        expect(typographyStore.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.default);
        expect(typographyStore.fontFamily).toBe('serif');
        expect(typographyStore.fontWeight).toBe(TYPOGRAPHY_LIMITS.fontWeight.default);
        expect(typographyStore.hyphenate).toBe(false);
        expect(typographyStore.readingTheme).toBe('default');
    });
});
