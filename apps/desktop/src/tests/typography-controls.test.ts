/**
 * Reader typography: the uiStore's persisted-preference methods (init +
 * clamping + font-stack resolution) and the controls that drive them. Each
 * control writes through `set_setting`; init reads back through the settings
 * mirror, clamped into range.
 */
import TypographyControls from '$components/reader/TypographyControls.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { TYPOGRAPHY_LIMITS, uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness } from './ipc-harness';

describe('reader typography', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        uiStore.reset();
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
        uiStore.initTypography();

        expect(uiStore.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.max);
        expect(uiStore.lineHeight).toBeCloseTo(1.8);
        expect(uiStore.measure).toBe(TYPOGRAPHY_LIMITS.measure.min);
        expect(uiStore.fontFamily).toBe('serif');
        expect(uiStore.readerFontStack).toContain('serif');
    });

    it('ignores an unknown persisted font id', async () => {
        harness = installIpcHarness({ get_setting: () => 'comic-sans' });
        await settingsStore.load();
        uiStore.initTypography();
        expect(uiStore.fontFamily).toBe('sans');
    });

    it('controls write through and persist each preference', async () => {
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        const { getByRole, getAllByRole } = render(TypographyControls);

        await fireEvent.change(getByRole('combobox'), { target: { value: 'mono' } });
        await flushIpc();
        expect(uiStore.fontFamily).toBe('mono');

        const [sizeRange, lineRange, measureRange] = getAllByRole('slider');
        await fireEvent.input(sizeRange, { target: { value: '20' } });
        await fireEvent.input(lineRange, { target: { value: '1.4' } });
        await fireEvent.input(measureRange, { target: { value: '820' } });
        await flushIpc();

        expect(uiStore.fontSize).toBe(20);
        expect(uiStore.lineHeight).toBeCloseTo(1.4);
        expect(uiStore.measure).toBe(820);

        const writes = harness.callsFor('set_setting');
        expect(writes).toContainEqual({ key: 'ui.typography.font-family', value: 'mono' });
        expect(writes).toContainEqual({ key: 'ui.typography.font-size', value: '20' });
        expect(writes).toContainEqual({ key: 'ui.typography.measure', value: '820' });
    });

    it('reset restores the defaults', async () => {
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        await uiStore.setFontSize(22);
        await uiStore.setFontFamily('serif');
        const { getByRole } = render(TypographyControls);

        await fireEvent.click(getByRole('button', { name: 'Reset' }));
        await flushIpc();

        expect(uiStore.fontSize).toBe(TYPOGRAPHY_LIMITS.fontSize.default);
        expect(uiStore.fontFamily).toBe('sans');
    });
});
