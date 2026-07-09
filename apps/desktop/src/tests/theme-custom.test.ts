import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { settingsStore } from '$lib/state/settings.svelte';
/**
 * Custom themes end to end: the export → import round-trip reproduces a
 * built-in's tokens exactly (the acceptance criterion), and the store's
 * add/remove/init lifecycle injects the rule, persists the JSON array,
 * re-validates on load, and falls back when the active theme is deleted.
 */
import { UiStore } from '$lib/state/ui.svelte';
import { readComputedTokens } from '$lib/utils/theme-dom';
import {
    CURIO_BASE_TOKENS,
    type CustomTheme,
    buildThemeRule,
    parseTheme,
    serializeTheme,
} from '$lib/utils/theme-format';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

function themeFixture(overrides: Partial<CustomTheme> = {}): CustomTheme {
    const tokens: Record<string, string> = {};
    for (const token of CURIO_BASE_TOKENS) tokens[token] = '#123456';
    return {
        id: 'ocean',
        name: 'Ocean',
        isDark: true,
        tokens: tokens as CustomTheme['tokens'],
        ...overrides,
    };
}

describe('theme round-trip', () => {
    const injected: HTMLStyleElement[] = [];
    const probes: HTMLElement[] = [];

    afterEach(() => {
        for (const element of [...injected, ...probes]) element.remove();
        injected.length = 0;
        probes.length = 0;
    });

    function style(css: string): void {
        const element = document.createElement('style');
        element.textContent = css;
        document.head.append(element);
        injected.push(element);
    }
    function probe(theme: string): HTMLElement {
        const element = document.createElement('div');
        element.setAttribute('data-theme', theme);
        document.body.append(element);
        probes.push(element);
        return element;
    }

    it('exporting built-in dark then importing reproduces identical computed tokens', () => {
        style(readFileSync(resolve(process.cwd(), 'src/app.css'), 'utf8'));

        // Export: read the live dark tokens off a probe, serialize to YAML.
        const darkTokens = readComputedTokens(probe('dark'));
        expect(darkTokens.bg.length).toBeGreaterThan(0); // jsdom resolved the rule
        const yaml = serializeTheme({ id: 'dark', name: 'Dark', isDark: true, tokens: darkTokens });

        // Import as a custom copy, inject its rule, read it back.
        const parsed = parseTheme(yaml.replace('id: dark', 'id: dark-copy'));
        expect(parsed.ok).toBe(true);
        if (!parsed.ok) return;
        style(buildThemeRule(parsed.theme));
        const copyTokens = readComputedTokens(probe('custom-dark-copy'));

        expect(copyTokens).toEqual(darkTokens);
    });
});

describe('UiStore — custom themes', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
        document.getElementById('curio-custom-themes')?.remove();
        try {
            window.localStorage.clear();
        } catch {
            // ignore
        }
    });

    it('addCustomTheme injects the rule and persists the JSON array', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new UiStore();

        await ui.addCustomTheme(themeFixture());

        expect(ui.hasCustomTheme('ocean')).toBe(true);
        const injected = document.getElementById('curio-custom-themes');
        expect(injected?.textContent).toContain('[data-theme="custom-ocean"]');
        const writes = harness.callsFor('set_setting');
        const persisted = writes.find((write) => write.key === 'ui.themes.custom');
        expect(persisted).toBeDefined();
        expect(JSON.parse(persisted?.value as string)).toHaveLength(1);
    });

    it('removeCustomTheme falls back to system when it was active', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new UiStore();
        await ui.addCustomTheme(themeFixture());
        await ui.setThemePreference('custom-ocean');
        expect(ui.resolvedTheme).toBe('custom-ocean');

        await ui.removeCustomTheme('ocean');

        expect(ui.hasCustomTheme('ocean')).toBe(false);
        expect(ui.themePreference).toBe('system');
    });

    it('initCustomThemes loads valid themes and drops invalid ones', async () => {
        const stored = JSON.stringify([
            themeFixture({ id: 'ocean' }),
            { id: 'broken', name: 'Broken' }, // missing tokens → dropped
        ]);
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.themes.custom' ? stored : null),
        });
        await settingsStore.load();

        const ui = new UiStore();
        ui.initCustomThemes();

        expect(ui.customThemes.map((theme) => theme.id)).toEqual(['ocean']);
    });

    it('initCustomThemes tolerates corrupt JSON', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.themes.custom' ? '{not json' : null),
        });
        await settingsStore.load();

        const ui = new UiStore();
        ui.initCustomThemes();

        expect(ui.customThemes).toHaveLength(0);
    });
});
