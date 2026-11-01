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

/**
 * Which listener API the fake query exposes: the modern `EventTarget` one,
 * only the legacy `addListener`/`removeListener` (Safari < 14 WebKit), or
 * neither.
 */
type ListenerApi = 'event-target' | 'legacy' | 'none';

function stubAppearance(
    initiallyLight: boolean,
    api: ListenerApi = 'event-target',
): FakeAppearance {
    let light = initiallyLight;
    const listeners = new Set<EventListenerOrEventListenerObject>();
    const query: Record<string, unknown> = {
        get matches(): boolean {
            return light;
        },
        media: '(prefers-color-scheme: light)',
    };
    if (api === 'event-target') {
        query.addEventListener = (
            type: string,
            listener: EventListenerOrEventListenerObject,
        ): void => {
            if (type === 'change') listeners.add(listener);
        };
        query.removeEventListener = (
            type: string,
            listener: EventListenerOrEventListenerObject,
        ): void => {
            if (type === 'change') listeners.delete(listener);
        };
    } else if (api === 'legacy') {
        query.addListener = (listener: EventListener): void => {
            listeners.add(listener);
        };
        query.removeListener = (listener: EventListener): void => {
            listeners.delete(listener);
        };
    }
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
    let store = new ThemeStore();

    beforeEach(() => {
        window.localStorage.clear();
        document.documentElement.setAttribute('data-theme', 'dark');
    });

    afterEach(() => {
        // Detach here, not at the end of each test, so a failed assertion
        // can't leave a listener on the stubbed query.
        store.reset();
        settingsStore.reset();
        vi.unstubAllGlobals();
        harness?.teardown();
        harness = null;
    });

    it('an OS appearance change flips data-theme while the preference is system', async () => {
        const os = stubAppearance(true);
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        store = new ThemeStore();

        store.initTheme();
        expect(dataTheme()).toBe('light');
        expect(os.listeners.size).toBe(1);

        os.set(false);
        expect(dataTheme()).toBe('dark');

        os.set(true);
        expect(dataTheme()).toBe('light');
    });

    it('an OS appearance change re-syncs the localStorage mirrors', async () => {
        const os = stubAppearance(false);
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        store = new ThemeStore();
        store.initTheme();

        window.localStorage.removeItem(THEME_STORAGE_KEY);
        window.localStorage.setItem(THEME_CUSTOM_CSS_KEY, '[data-theme="custom-stale"]{}');
        os.set(true);

        // The mirror holds the preference (the preload resolves `system`
        // itself), rewritten on the change; no custom rule is active.
        expect(window.localStorage.getItem(THEME_STORAGE_KEY)).toBe('system');
        expect(window.localStorage.getItem(THEME_CUSTOM_CSS_KEY)).toBeNull();
    });

    it('picking an explicit theme leaves no listener, and the OS no longer repaints', async () => {
        const os = stubAppearance(true);
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        await settingsStore.load();
        store = new ThemeStore();
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
        store = new ThemeStore();

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
        store = new ThemeStore();
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
    });

    it('follows the OS through the legacy addListener API (Safari < 14 WebKit)', async () => {
        const os = stubAppearance(true, 'legacy');
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        await settingsStore.load();
        store = new ThemeStore();

        store.initTheme();
        expect(store.followsSystem).toBe(true);
        expect(os.listeners.size).toBe(1);
        os.set(false);
        expect(dataTheme()).toBe('dark');

        // An explicit theme detaches through removeListener.
        await store.setThemePreference('nord');
        expect(os.listeners.size).toBe(0);
        expect(store.followsSystem).toBe(false);
        os.set(true);
        expect(dataTheme()).toBe('nord');
    });

    it('resolves the OS appearance but attaches nothing when the query has no listener API', async () => {
        const os = stubAppearance(true, 'none');
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        store = new ThemeStore();

        store.initTheme();

        expect(dataTheme()).toBe('light');
        expect(os.listeners.size).toBe(0);
        expect(store.followsSystem).toBe(false);
    });

    it('stays inert without matchMedia (falls back dark, attaches nothing)', async () => {
        // Seed a non-dark value so the dark fallback is actually written.
        document.documentElement.setAttribute('data-theme', 'nord');
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();
        store = new ThemeStore();

        store.initTheme();

        expect(dataTheme()).toBe('dark');
        expect(store.followsSystem).toBe(false);
    });
});
