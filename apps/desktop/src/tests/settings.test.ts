/**
 * Settings mirror: load pulls every registered `ui.*` key through
 * `get_setting`, `set` writes through `set_setting`, and errors surface
 * without corrupting the mirror.
 */
import { SETTING_KEYS, SettingsStore } from '$lib/state/settings.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, commandErrorFixture, installIpcHarness, rejectWith } from './ipc-harness';

describe('settings store', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        harness?.teardown();
        harness = null;
    });

    it('loads every registered key and mirrors present values', async () => {
        const stored: Record<string, string> = {
            'ui.theme': 'nord',
            'ui.typography.font-size': '18',
        };
        harness = installIpcHarness({
            get_setting: (args) => stored[args.key as string] ?? null,
        });
        const store = new SettingsStore();
        await store.load();

        expect(store.loaded).toBe(true);
        expect(store.get(SETTING_KEYS.theme)).toBe('nord');
        expect(store.get(SETTING_KEYS.fontSize)).toBe('18');
        expect(store.get(SETTING_KEYS.listWidth)).toBeUndefined();

        const asked = harness.callsFor('get_setting').map((call) => call.key);
        expect(asked.sort()).toEqual(Object.values(SETTING_KEYS).sort());
    });

    it('set writes through and updates the mirror on success', async () => {
        harness = installIpcHarness({ get_setting: null, set_setting: null });
        const store = new SettingsStore();

        const result = await store.set(SETTING_KEYS.measure, '680');
        expect(result.status).toBe('ok');
        expect(store.get(SETTING_KEYS.measure)).toBe('680');
        expect(harness.callsFor('set_setting')[0]).toEqual({
            key: 'ui.typography.measure',
            value: '680',
        });
    });

    it('a failed write leaves the mirror untouched and records the error', async () => {
        const error = commandErrorFixture({ kind: 'internal', code: 'STORAGE' });
        harness = installIpcHarness({ set_setting: rejectWith(error) });
        const store = new SettingsStore();

        const result = await store.set(SETTING_KEYS.theme, 'nord');
        expect(result.status).toBe('error');
        expect(store.get(SETTING_KEYS.theme)).toBeUndefined();
        expect(store.error).toEqual(error);
    });

    it('survives a missing Tauri runtime as an internal error', async () => {
        harness = installIpcHarness({}); // every command throws a real Error
        const store = new SettingsStore();
        await store.load();

        expect(store.loaded).toBe(true);
        expect(store.error?.kind).toBe('internal');
        expect(store.get(SETTING_KEYS.theme)).toBeUndefined();
    });
});
