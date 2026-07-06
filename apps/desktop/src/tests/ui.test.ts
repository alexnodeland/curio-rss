/**
 * UI store: theme resolution + double persistence (settings table as truth,
 * localStorage as the FOUC-preload mirror), modal enum, and toasts.
 */
import { settingsStore } from '$lib/state/settings.svelte';
import { THEMES, THEME_STORAGE_KEY, UiStore } from '$lib/state/ui.svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

describe('ui store — theme', () => {
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
        const ui = new UiStore();

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

        const ui = new UiStore();
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

        const ui = new UiStore();
        ui.initTheme();

        expect(ui.themePreference).toBe('tokyo-night');
    });

    it('ignores garbage in either store', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.theme' ? 'not-a-theme' : null),
        });
        window.localStorage.setItem(THEME_STORAGE_KEY, 'also-garbage');
        await settingsStore.load();

        const ui = new UiStore();
        ui.initTheme();

        expect(ui.themePreference).toBe('system');
    });

    it('system preference resolves via matchMedia (light) and falls back dark', () => {
        const ui = new UiStore();
        expect(ui.themePreference).toBe('system');
        // jsdom has no matchMedia — the safe fallback is dark.
        expect(ui.resolvedTheme).toBe('dark');

        vi.stubGlobal('matchMedia', (query: string) => ({
            matches: query.includes('light'),
        }));
        expect(ui.resolvedTheme).toBe('light');
    });
});

describe('ui store — chrome', () => {
    it('modal enum opens and closes', () => {
        const ui = new UiStore();
        expect(ui.activeModal).toBeNull();
        ui.openModal('help');
        expect(ui.activeModal).toBe('help');
        ui.closeModal();
        expect(ui.activeModal).toBeNull();
    });

    it('sidebar toggles', () => {
        const ui = new UiStore();
        ui.toggleSidebar();
        expect(ui.sidebarCollapsed).toBe(true);
        ui.toggleSidebar();
        expect(ui.sidebarCollapsed).toBe(false);
    });
});

describe('ui store — toasts', () => {
    beforeEach(() => {
        vi.useFakeTimers();
    });

    afterEach(() => {
        vi.useRealTimers();
    });

    it('auto-dismisses after the duration', () => {
        const ui = new UiStore();
        ui.showToast('saved', 'success', 1000);
        expect(ui.toasts).toHaveLength(1);
        expect(ui.toasts[0]).toMatchObject({ message: 'saved', tone: 'success' });

        vi.advanceTimersByTime(999);
        expect(ui.toasts).toHaveLength(1);
        vi.advanceTimersByTime(1);
        expect(ui.toasts).toHaveLength(0);
    });

    it('duration 0 sticks until dismissed by hand', () => {
        const ui = new UiStore();
        const id = ui.showToast('stuck', 'error', 0);
        vi.advanceTimersByTime(60_000);
        expect(ui.toasts).toHaveLength(1);

        ui.dismissToast(id);
        expect(ui.toasts).toHaveLength(0);
    });

    it('dismissing one toast leaves the others (and clears its timer)', () => {
        const ui = new UiStore();
        const first = ui.showToast('one');
        ui.showToast('two');

        ui.dismissToast(first);
        expect(ui.toasts.map((toast) => toast.message)).toEqual(['two']);

        vi.runAllTimers();
        expect(ui.toasts).toHaveLength(0);
    });
});

describe('ui store — pane layout', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('initLayout adopts persisted widths and collapse state', async () => {
        harness = installIpcHarness({
            get_setting: (args) => {
                const key = args.key as string;
                if (key === 'ui.panes.sidebar-width') {
                    return '320';
                }
                if (key === 'ui.panes.list-width') {
                    return '420';
                }
                if (key === 'ui.panes.sidebar-collapsed') {
                    return 'true';
                }
                return null;
            },
        });
        await settingsStore.load();

        const ui = new UiStore();
        ui.initLayout();
        expect(ui.sidebarWidth).toBe(320);
        expect(ui.listWidth).toBe(420);
        expect(ui.sidebarCollapsed).toBe(true);
    });

    it('initLayout clamps out-of-range widths and ignores garbage', async () => {
        harness = installIpcHarness({
            get_setting: (args) => {
                const key = args.key as string;
                if (key === 'ui.panes.sidebar-width') {
                    return '9999';
                }
                if (key === 'ui.panes.list-width') {
                    return 'wide, please';
                }
                return null;
            },
        });
        await settingsStore.load();

        const ui = new UiStore();
        ui.initLayout();
        expect(ui.sidebarWidth).toBe(420); // clamped to PANE_LIMITS.sidebar.max
        expect(ui.listWidth).toBe(360); // fallback: unparseable value
        expect(ui.sidebarCollapsed).toBe(false);
    });

    it('initLayout leaves the defaults without persisted values', async () => {
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();

        const ui = new UiStore();
        ui.initLayout();
        expect(ui.sidebarWidth).toBe(280);
        expect(ui.listWidth).toBe(360);
    });
});
