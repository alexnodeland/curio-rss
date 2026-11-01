/**
 * The root layout's startup gate: the reader (`children`) mounts only once
 * the backend says the profile opened. On a failed start the recovery screen
 * renders instead, the reader never mounts (its commands would all fail with
 * no core), and the locale comes from the system rather than from settings
 * that live in the database that didn't open.
 */
import { localeStore } from '$lib/i18n';
import { settingsStore } from '$lib/state/settings.svelte';
import { startupStore } from '$lib/state/startup.svelte';
import { cleanup, render, screen } from '@testing-library/svelte';
import { createRawSnippet } from 'svelte';
import { afterEach, describe, expect, it, vi } from 'vitest';
import Layout from '../routes/+layout.svelte';
import { type IpcHarness, flushIpc, installIpcHarness, startupFailureFixture } from './ipc-harness';

const reader = createRawSnippet(() => ({
    render: () => '<div data-testid="reader">reader</div>',
}));

describe('root layout startup gate', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        cleanup();
        vi.restoreAllMocks();
        startupStore.reset();
        localeStore.reset();
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
        // Nothing but the gate check reached the backend.
        expect(harness.calls.map((call) => call.cmd)).toEqual(['get_startup_failure']);
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

    it('mounts nothing while the gate check is still pending', () => {
        harness = installIpcHarness({ get_startup_failure: () => new Promise(() => {}) });
        render(Layout, { children: reader });
        expect(screen.queryByTestId('reader')).toBeNull();
        expect(screen.queryByRole('heading', { level: 1 })).toBeNull();
    });
});
