/**
 * The theme picker wires the 9 themes plus a system mode into
 * `uiStore.setThemePreference`, which persists through `set_setting` and
 * applies the DOM attribute the CSS token system keys off.
 */
import ThemePicker from '$components/common/ThemePicker.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness } from './ipc-harness';

describe('ThemePicker', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        uiStore.reset();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('offers system plus the nine themes', () => {
        harness = installIpcHarness({});
        const { getAllByRole } = render(ThemePicker);
        expect(getAllByRole('button')).toHaveLength(10);
    });

    it('selecting a theme persists it and applies the DOM attribute', async () => {
        harness = installIpcHarness({ set_setting: null });
        const { getByLabelText } = render(ThemePicker);

        await fireEvent.click(getByLabelText('Use the Nord theme'));
        await flushIpc();

        expect(uiStore.themePreference).toBe('nord');
        expect(document.documentElement.getAttribute('data-theme')).toBe('nord');
        expect(harness.callsFor('set_setting')).toEqual([{ key: 'ui.theme', value: 'nord' }]);
    });

    it('the selected theme is pressed', async () => {
        harness = installIpcHarness({ set_setting: null });
        const { getByLabelText } = render(ThemePicker);

        const dracula = getByLabelText('Use the Dracula theme');
        await fireEvent.click(dracula);
        await flushIpc();

        expect(dracula.getAttribute('aria-pressed')).toBe('true');
    });
});
