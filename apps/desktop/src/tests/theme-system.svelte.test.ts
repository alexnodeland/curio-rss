/**
 * Theme store, `system` mode: while the preference is "follow the OS", an
 * OS appearance change (macOS Auto at sunset) repaints live — the DOM
 * attribute, the localStorage mirror and the reactive `resolvedTheme` — and
 * an explicit theme detaches the listener. Lives in a `.svelte.test.ts` so
 * the reactivity check can use `$effect.root`.
 */
import { settingsStore } from '$lib/state/settings.svelte';
import { THEME_CUSTOM_CSS_KEY, THEME_STORAGE_KEY, ThemeStore } from '$lib/state/theme.svelte';
import { flushSync } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

/** A controllable `matchMedia` for the OS appearance query. */
interface FakeAppearance {
    /** Flips the OS appearance and fires `change` like the webview does. */
    set(light: boolean): void;
    /** The listeners currently attached to the query. */
    readonly listeners: Set<EventListenerOrEventListenerObject>;
}

function stubAppearance(initiallyLight: boolean): FakeAppearance {
    let light = initiallyLight;
    const listeners = new Set<EventListenerOrEventListenerObject>();
    const query = {
        get matches(): boolean {
            return light;
        },
        media: '(prefers-color-scheme: light)',
        addEventListener: (type: string, listener: EventListenerOrEventListenerObject): void => {
            if (type === 'change') listeners.add(listener);
        },
        removeEventListener: (type: string, listener: EventListenerOrEventListenerObject): void => {
            if (type === 'change') listeners.delete(listener);
        },
    };
    vi.stubGlobal('matchMedia', (media: string) => {
        expect(media).toBe('(prefers-color-scheme: light)');
        return query;
    });
    return {
        set(next: boolean): void {
            light = next;
            const event = new Event('change');
            for (const listener of [...listeners]) {
                if (typeof listener === 'function') listener(event);
                else listener.handleEvent(event);
            }
        },
        listeners,
    };
}

function dataTheme(): string | null {
    return document.documentElement.getAttribute('data-theme');
}

describe('theme store — following the OS appearance', () => {
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

    it('an OS appearance change flips data-theme while the preference is system', async () => {
        const os = stubAppearance(true);
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        const store = new ThemeStore();

        store.initTheme();
        expect(dataTheme()).toBe('light');
        expect(os.listeners.size).toBe(1);

        os.set(false);
        expect(dataTheme()).toBe('dark');

        os.set(true);
        expect(dataTheme()).toBe('light');
        store.reset();
    });

    it('an OS appearance change re-syncs the localStorage mirrors', async () => {
        const os = stubAppearance(false);
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        const store = new ThemeStore();
        store.initTheme();

        window.localStorage.removeItem(THEME_STORAGE_KEY);
        window.localStorage.setItem(THEME_CUSTOM_CSS_KEY, '[data-theme="custom-stale"]{}');
        os.set(true);

        // The mirror holds the preference (the preload resolves `system`
        // itself), rewritten on the change; no custom rule is active.
        expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('system');
        expect(window.localStorage.getItem(THEME_CUSTOM_CSS_KEY)).toBeNull();
        store.reset();
    });

    it('picking an explicit theme leaves no listener, and the OS no longer repaints', async () => {
        const os = stubAppearance(true);
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        await settingsStore.load();
        const store = new ThemeStore();
        store.initTheme();
        expect(store.followsSystem).toBe(true);

        await store.setThemePreference('nord');

        expect(os.listeners.size).toBe(0);
        expect(store.followsSystem).toBe(false);
        os.set(false);
        expect(dataTheme()).toBe('nord');
        expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('nord');
    });

    it('returning to system re-attaches exactly one listener', async () => {
        const os = stubAppearance(true);
        harness = installIpcHarness({ set_setting: null });
        const store = new ThemeStore();

        await store.setThemePreference('system');
        await store.setThemePreference('system');
        expect(os.listeners.size).toBe(1);

        await store.setThemePreference('dracula');
        await store.setThemePreference('system');
        expect(os.listeners.size).toBe(1);

        os.set(false);
        expect(dataTheme()).toBe('dark');
        store.reset();
        expect(os.listeners.size).toBe(0);
    });

    it('resolvedTheme re-derives for reactive readers on an OS change', async () => {
        const os = stubAppearance(true);
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        const store = new ThemeStore();
        store.initTheme();

        const seen: string[] = [];
        const dispose = $effect.root(() => {
            $effect(() => {
                seen.push(store.resolvedTheme);
            });
        });
        flushSync();
        os.set(false);
        flushSync();

        expect(seen).toEqual(['light', 'dark']);
        dispose();
        store.reset();
    });

    it('stays inert without matchMedia (falls back dark, attaches nothing)', async () => {
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        const store = new ThemeStore();

        store.initTheme();

        expect(dataTheme()).toBe('dark');
        expect(store.followsSystem).toBe(false);
    });
});
