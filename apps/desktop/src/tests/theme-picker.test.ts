/**
 * The theme picker wires the built-in themes into `uiStore.setThemePreference`
 * (persist + apply the DOM attribute), and drives the WP6 custom-theme flows:
 * importing a validated `.yaml` theme applies + persists it, exporting serializes
 * the active theme — both through the PathToken commands, so no raw path ever
 * crosses IPC (asserted the way opml.test.ts does).
 */
import ThemePicker from '$components/common/ThemePicker.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { uiStore } from '$lib/state/ui.svelte';
import { CURIO_BASE_TOKENS, type CustomTheme, serializeTheme } from '$lib/utils/theme-format';
import { cleanup, fireEvent, render } from '@testing-library/svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, flushIpc, installIpcHarness, pathTokenFixture } from './ipc-harness';

function validThemeYaml(id = 'ocean', name = 'Ocean'): string {
    const tokens: Record<string, string> = {};
    for (const token of CURIO_BASE_TOKENS) tokens[token] = '#123456';
    return serializeTheme({ id, name, isDark: true, tokens: tokens as CustomTheme['tokens'] });
}

async function settle(): Promise<void> {
    for (let i = 0; i < 6; i += 1) {
        await flushIpc();
    }
}

describe('ThemePicker', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        uiStore.reset();
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('offers system plus the nine built-in themes and the import/export actions', () => {
        harness = installIpcHarness({});
        const { getAllByLabelText, getByText } = render(ThemePicker);
        expect(getAllByLabelText(/^Use the .+ theme$/)).toHaveLength(10);
        expect(getByText('Import theme…')).toBeTruthy();
        expect(getByText('Export as YAML…')).toBeTruthy();
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

    it('imports a valid theme, applies + persists it, and never leaks the path', async () => {
        const PATH = '/Users/me/themes/ocean.yaml';
        harness = installIpcHarness({
            set_setting: null,
            pick_theme_import_file: pathTokenFixture({ token: 'TOK-IN', path: PATH }),
            read_text_file: () => validThemeYaml('ocean', 'Ocean'),
        });
        const { getByText } = render(ThemePicker);

        await fireEvent.click(getByText('Import theme…'));
        await settle();

        expect(uiStore.customThemes.map((theme) => theme.id)).toEqual(['ocean']);
        expect(uiStore.themePreference).toBe('custom-ocean');
        expect(document.documentElement.getAttribute('data-theme')).toBe('custom-ocean');

        const writes = harness.callsFor('set_setting');
        expect(writes.some((write) => write.key === 'ui.themes.custom')).toBe(true);
        expect(writes).toContainEqual({ key: 'ui.theme', value: 'custom-ocean' });

        // PathToken discipline: the raw picked path never crosses IPC.
        for (const call of harness.calls) {
            expect(JSON.stringify(call.args)).not.toContain(PATH);
        }
    });

    it('rejects an invalid theme file with a specific error and adds nothing', async () => {
        harness = installIpcHarness({
            pick_theme_import_file: pathTokenFixture({ token: 'TOK-IN', path: '/x/bad.yaml' }),
            read_text_file: () => 'schema: not-curio\n',
        });
        const { getByText } = render(ThemePicker);

        await fireEvent.click(getByText('Import theme…'));
        await settle();

        expect(uiStore.customThemes).toHaveLength(0);
        expect(uiStore.toasts.some((toast) => toast.tone === 'error')).toBe(true);
    });

    it('exports the active theme through the PathToken flow', async () => {
        const PATH = '/Users/me/themes/out.yaml';
        harness = installIpcHarness({
            set_setting: null,
            pick_theme_export_path: pathTokenFixture({ token: 'TOK-OUT', path: PATH }),
            export_text_file: null,
        });
        const { getByText } = render(ThemePicker);

        await fireEvent.click(getByText('Export as YAML…'));
        await settle();

        const calls = harness.callsFor('export_text_file');
        expect(calls).toHaveLength(1);
        expect(calls[0].pathToken).toBe('TOK-OUT');
        expect(String(calls[0].contents)).toContain('schema: curio.theme/v1');
        for (const call of harness.calls) {
            expect(JSON.stringify(call.args)).not.toContain(PATH);
        }
    });
});
