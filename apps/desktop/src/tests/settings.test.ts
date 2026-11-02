/**
 * Settings mirror: load pulls every registered `ui.*` key through
 * `get_setting`, `set` writes through `set_setting`, and errors surface
 * without corrupting the mirror. The localStorage boot mirror carries only
 * the first-paint keys, and the backend's values always win over it.
 */
import {
    BOOT_KEYS,
    BOOT_MIRROR_STORAGE_KEY,
    SETTING_KEYS,
    type SettingKey,
    SettingsStore,
} from '$lib/state/settings.svelte';
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

describe('settings boot mirror (shift-free boot)', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        harness?.teardown();
        harness = null;
        window.localStorage.clear();
    });

    function mirror(): Record<string, string> {
        return JSON.parse(window.localStorage.getItem(BOOT_MIRROR_STORAGE_KEY) ?? '{}');
    }

    it('load mirrors only the boot-shaping keys to localStorage', async () => {
        const stored: Record<string, string> = {
            'ui.panes.list-width': '480',
            'ui.reading.density': 'compact',
            'ui.typography.font-size': '19',
            'ui.locale': 'de',
            // Not boot-shaping: stays out of the mirror.
            'ui.notify.muted-feeds': '[1,2]',
        };
        harness = installIpcHarness({ get_setting: (args) => stored[args.key as string] ?? null });
        await new SettingsStore().load();

        expect(mirror()).toEqual({
            'ui.panes.list-width': '480',
            'ui.reading.density': 'compact',
            'ui.typography.font-size': '19',
            'ui.locale': 'de',
        });
        for (const key of Object.keys(mirror())) {
            expect(BOOT_KEYS).toContain(key as SettingKey);
        }
    });

    it('set refreshes the mirror for a boot key and leaves it alone otherwise', async () => {
        harness = installIpcHarness({ set_setting: null });
        const store = new SettingsStore();

        await store.set(SETTING_KEYS.sidebarWidth, '300');
        expect(mirror()).toEqual({ 'ui.panes.sidebar-width': '300' });

        await store.set(SETTING_KEYS.notifyEnabled, 'false');
        expect(mirror()).toEqual({ 'ui.panes.sidebar-width': '300' });
    });

    it('a failed boot-key write does not touch the mirror', async () => {
        harness = installIpcHarness({ set_setting: rejectWith(commandErrorFixture()) });
        await new SettingsStore().set(SETTING_KEYS.listWidth, '500');
        expect(window.localStorage.getItem(BOOT_MIRROR_STORAGE_KEY)).toBeNull();
    });

    it('primeFromMirror seeds the boot keys before load answers', () => {
        window.localStorage.setItem(
            BOOT_MIRROR_STORAGE_KEY,
            JSON.stringify({
                'ui.panes.list-width': '480',
                'ui.typography.reading-theme': 'sepia',
                // Never adopted from the mirror: not a boot key / not a string.
                'ui.theme': 'nord',
                'ui.panes.sidebar-width': 300,
            }),
        );
        const store = new SettingsStore();
        store.primeFromMirror();

        expect(store.loaded).toBe(false);
        expect(store.get(SETTING_KEYS.listWidth)).toBe('480');
        expect(store.get(SETTING_KEYS.readingTheme)).toBe('sepia');
        expect(store.get(SETTING_KEYS.theme)).toBeUndefined();
        expect(store.get(SETTING_KEYS.sidebarWidth)).toBeUndefined();
    });

    it('primeFromMirror boots at defaults from a corrupt or non-object mirror', () => {
        const store = new SettingsStore();
        for (const raw of ['{not json', '[1,2]', 'null', '"text"']) {
            window.localStorage.setItem(BOOT_MIRROR_STORAGE_KEY, raw);
            store.primeFromMirror();
            expect(store.get(SETTING_KEYS.listWidth)).toBeUndefined();
        }
    });

    it('primeFromMirror never overrides the loaded backend values', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.panes.list-width' ? '420' : null),
        });
        const store = new SettingsStore();
        await store.load();
        window.localStorage.setItem(BOOT_MIRROR_STORAGE_KEY, '{"ui.panes.list-width":"999"}');
        store.primeFromMirror();
        expect(store.get(SETTING_KEYS.listWidth)).toBe('420');
    });
});
