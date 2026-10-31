/**
 * Theme store: theme resolution + double persistence (settings table as
 * truth, localStorage as the FOUC-preload mirror), the id guards, and reset.
 * The custom-theme lifecycle lives in theme-custom.test.ts.
 */
import { settingsStore } from '$lib/state/settings.svelte';
import {
    THEMES,
    THEME_CUSTOM_CSS_KEY,
    THEME_STORAGE_KEY,
    ThemeStore,
    isCustomThemeId,
    isThemeId,
    isThemePreference,
} from '$lib/state/theme.svelte';
import type { CustomTheme } from '$lib/utils/theme-format';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

describe('theme store', () => {
    let harness: IpcHarness | null = null;

    beforeEach(() => {
        window.localStorage.clear();
        document.documentElement.setAttribute('data-theme', 'dark');
    });

    afterEach(() => {
        settingsStore.reset();
        vi.unstubAllGlobals();
        harness?.teardown();
        harness = null;
    });

    it('ships exactly the 9 app.css themes', () => {
        expect(THEMES.map((theme) => theme.id)).toEqual([
            'light',
            'dark',
            'nord',
            'catppuccin',
            'dracula',
            'gruvbox',
            'tokyo-night',
            'solarized-dark',
            'solarized-light',
        ]);
    });

    it('setThemePreference applies the DOM attribute, mirrors, and persists', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new ThemeStore();

        await ui.setThemePreference('nord');

        expect(document.documentElement.getAttribute('data-theme')).toBe('nord');
        expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('nord');
        expect(harness.callsFor('set_setting')[0]).toEqual({ key: 'ui.theme', value: 'nord' });
    });

    it('initTheme lets the settings table win over the localStorage mirror', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.theme' ? 'gruvbox' : null),
        });
        window.localStorage.setItem(THEME_STORAGE_KEY, 'dracula');
        await settingsStore.load();

        const ui = new ThemeStore();
        ui.initTheme();

        expect(ui.themePreference).toBe('gruvbox');
        expect(document.documentElement.getAttribute('data-theme')).toBe('gruvbox');
        // The mirror re-syncs to the source of truth.
        expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('gruvbox');
    });

    it('initTheme falls back to the mirror when settings hold nothing', async () => {
        harness = installIpcHarness({ get_setting: null });
        window.localStorage.setItem(THEME_STORAGE_KEY, 'tokyo-night');
        await settingsStore.load();

        const ui = new ThemeStore();
        ui.initTheme();

        expect(ui.themePreference).toBe('tokyo-night');
    });

    it('ignores garbage in either store', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.theme' ? 'not-a-theme' : null),
        });
        window.localStorage.setItem(THEME_STORAGE_KEY, 'also-garbage');
        await settingsStore.load();

        const ui = new ThemeStore();
        ui.initTheme();

        expect(ui.themePreference).toBe('system');
    });

    it('initTheme drops a custom preference whose theme no longer exists', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.theme' ? 'custom-gone' : null),
        });
        window.localStorage.setItem(THEME_CUSTOM_CSS_KEY, '[data-theme="custom-gone"]{}');
        await settingsStore.load();

        const ui = new ThemeStore();
        ui.initTheme();

        expect(ui.themePreference).toBe('system');
        expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('system');
        // No active custom → the preload's rule mirror is cleared.
        expect(window.localStorage.getItem(THEME_CUSTOM_CSS_KEY)).toBeNull();
    });

    it('system preference resolves via matchMedia (light) and falls back dark', () => {
        const ui = new ThemeStore();
        expect(ui.themePreference).toBe('system');
        // jsdom has no matchMedia — the safe fallback is dark.
        expect(ui.resolvedTheme).toBe('dark');

        vi.stubGlobal('matchMedia', (query: string) => ({
            matches: query.includes('light'),
        }));
        expect(ui.resolvedTheme).toBe('light');
    });

    it('the id guards accept built-ins, custom slugs, and system only', () => {
        expect(isThemeId('nord')).toBe(true);
        expect(isThemeId('custom-ocean')).toBe(false);
        expect(isCustomThemeId('custom-deep-ocean')).toBe(true);
        expect(isCustomThemeId('custom-Bad_Slug')).toBe(false);
        expect(isCustomThemeId('custom-')).toBe(false);
        expect(isThemePreference('system')).toBe(true);
        expect(isThemePreference('custom-ocean')).toBe(true);
        expect(isThemePreference('neon')).toBe(false);
    });

    it('reset restores the system preference and drops custom themes', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new ThemeStore();
        await ui.setThemePreference('dracula');
        ui.customThemes = [
            { id: 'ocean', name: 'Ocean', isDark: true, tokens: {} as CustomTheme['tokens'] },
        ];

        ui.reset();

        expect(ui.themePreference).toBe('system');
        expect(ui.customThemes).toEqual([]);
    });
});
