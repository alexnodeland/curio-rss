/**
 * The root layout's startup gate: the reader (`children`) mounts only once
 * the backend says the profile opened. On a failed start the recovery screen
 * renders instead, the reader never mounts (its commands would all fail with
 * no core), and the locale comes from the system rather than from settings
 * that live in the database that didn't open.
 */
import { localeStore } from '$lib/i18n';
import { layoutStore } from '$lib/state/layout.svelte';
import { prefsStore } from '$lib/state/prefs.svelte';
import { BOOT_MIRROR_STORAGE_KEY, settingsStore } from '$lib/state/settings.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import { typographyStore } from '$lib/state/typography.svelte';
import { windowChromeStore } from '$lib/state/window-chrome.svelte';
import { cleanup, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Layout from '../routes/+layout.svelte';
import { type IpcHarness, flushIpc, installIpcHarness, startupFailureFixture } from './ipc-harness';
import { resetUiStores } from './ui-stores';

const reader = createRawSnippet(() => ({
    render: () => '<div data-testid="reader">reader</div>',
}));

describe('root layout startup gate', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        startupStore.reset();
        windowChromeStore.reset();
        localeStore.reset();
        settingsStore.reset();
        resetUiStores();
        layoutStore.sidebarWidth = 280;
        layoutStore.listWidth = 360;
        window.localStorage.clear();
        harness?.teardown();
        harness = null;
    });

    it('shows the recovery screen and never mounts the reader when the profile failed', async () => {
        harness = installIpcHarness({ get_startup_failure: startupFailureFixture() });
        const load = vi.spyOn(settingsStore, 'load');
        const fromSystem = vi.spyOn(localeStore, 'initFromSystem');
        render(Layout, { children: reader });
        await flushIpc();

        expect(screen.queryByTestId('reader')).toBeNull();
        expect(screen.getByRole('main', { name: /./ })).toBeTruthy();
        expect(screen.getByRole('heading', { level: 1 })).toBeTruthy();
        expect(fromSystem).toHaveBeenCalledOnce();
        expect(load).not.toHaveBeenCalled();
        // Nothing but the gate check and the window chrome (which needs no
        // core) reached the backend.
        expect(harness.calls.map((call) => call.cmd).sort()).toEqual([
            'get_startup_failure',
            'window_chrome',
        ]);
    });

    it('knows the window chrome before either screen renders', async () => {
        harness = installIpcHarness({
            get_startup_failure: startupFailureFixture(),
            window_chrome: { overlay_title_bar: true, fullscreen: false },
        });
        render(Layout, { children: reader });
        await flushIpc();

        expect(document.documentElement.dataset.titlebar).toBe('overlay');
        expect(screen.getByRole('main').getAttribute('data-tauri-drag-region')).toBe('true');
    });

    it('mounts the reader and loads settings on a normal start', async () => {
        harness = installIpcHarness({ get_startup_failure: null, connectivity_online: true });
        const load = vi.spyOn(settingsStore, 'load').mockResolvedValue();
        const fromSystem = vi.spyOn(localeStore, 'initFromSystem');
        render(Layout, { children: reader });
        await flushIpc();

        expect(screen.getByTestId('reader')).toBeTruthy();
        expect(screen.queryByRole('heading', { level: 1 })).toBeNull();
        expect(load).toHaveBeenCalledOnce();
        expect(fromSystem).not.toHaveBeenCalled();
    });

    it('boots the shell at the mirrored layout before settings answer (no shift)', async () => {
        window.localStorage.setItem(
            BOOT_MIRROR_STORAGE_KEY,
            JSON.stringify({
                'ui.panes.sidebar-width': '320',
                'ui.panes.list-width': '480',
                'ui.reading.density': 'compact',
                'ui.typography.font-size': '19',
                'ui.locale': 'de',
            }),
        );
        harness = installIpcHarness({ get_startup_failure: null });
        // The backend never answers: everything the shell shows comes from the mirror.
        vi.spyOn(settingsStore, 'load').mockReturnValue(new Promise(() => {}));
        render(Layout, { children: reader });
        await flushIpc();

        expect(screen.getByTestId('reader')).toBeTruthy();
        expect(layoutStore.sidebarWidth).toBe(320);
        expect(layoutStore.listWidth).toBe(480);
        expect(prefsStore.readingDensity).toBe('compact');
        expect(typographyStore.fontSize).toBe(19);
        expect(localeStore.active).toBe('de');
    });

    it('mounts nothing while the gate check is still pending', () => {
        harness = installIpcHarness({ get_startup_failure: () => new Promise(() => {}) });
        render(Layout, { children: reader });
        expect(screen.queryByTestId('reader')).toBeNull();
        expect(screen.queryByRole('heading', { level: 1 })).toBeNull();
    });
});
