/**
 * Layout store: the modal enum and edit-feed binding, sidebar collapse, the
 * persisted pane widths (clamped into PANE_LIMITS), the per-feed-type "home"
 * list layout, and reset.
 */
import { LayoutStore, PANE_LIMITS } from '$lib/state/layout.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

describe('layout store — chrome', () => {
    it('modal enum opens and closes', () => {
        const ui = new LayoutStore();
        expect(ui.activeModal).toBeNull();
        ui.openModal('help');
        expect(ui.activeModal).toBe('help');
        ui.closeModal();
        expect(ui.activeModal).toBeNull();
    });

    it('sidebar toggles', () => {
        const ui = new LayoutStore();
        ui.toggleSidebar();
        expect(ui.sidebarCollapsed).toBe(true);
        ui.toggleSidebar();
        expect(ui.sidebarCollapsed).toBe(false);
    });

    it('openEditFeed binds the modal to a feed and section; closeModal unbinds it', () => {
        const ui = new LayoutStore();
        ui.openEditFeed(42, 'health');
        expect(ui.activeModal).toBe('edit-feed');
        expect(ui.editFeedId).toBe(42);
        expect(ui.editFeedSection).toBe('health');

        ui.closeModal();
        expect(ui.activeModal).toBeNull();
        expect(ui.editFeedId).toBeNull();
        expect(ui.editFeedSection).toBe('details');

        ui.openEditFeed(7);
        expect(ui.editFeedSection).toBe('details');
    });

    it('reset closes any modal and re-expands the sidebar', () => {
        const ui = new LayoutStore();
        ui.openEditFeed(3, 'health');
        ui.toggleSidebar();

        ui.reset();

        expect(ui.activeModal).toBeNull();
        expect(ui.editFeedId).toBeNull();
        expect(ui.editFeedSection).toBe('details');
        expect(ui.sidebarCollapsed).toBe(false);
    });
});

describe('layout store — pane layout', () => {
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

        const ui = new LayoutStore();
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

        const ui = new LayoutStore();
        ui.initLayout();
        expect(ui.sidebarWidth).toBe(420); // clamped to PANE_LIMITS.sidebar.max
        expect(ui.listWidth).toBe(360); // fallback: unparseable value
        expect(ui.sidebarCollapsed).toBe(false);
    });

    it('initLayout clamps a too-narrow list width up to the minimum', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.panes.list-width' ? '10' : null),
        });
        await settingsStore.load();

        const ui = new LayoutStore();
        ui.initLayout();
        expect(ui.listWidth).toBe(PANE_LIMITS.list.min);
    });

    it('initLayout leaves the defaults without persisted values', async () => {
        harness = installIpcHarness({ get_setting: null });
        await settingsStore.load();

        const ui = new LayoutStore();
        ui.initLayout();
        expect(ui.sidebarWidth).toBe(280);
        expect(ui.listWidth).toBe(360);
    });
});

describe('layout store — home layouts', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('initLayout adopts the per-type home layouts', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.home.youtube' ? 'true' : null),
        });
        await settingsStore.load();

        const ui = new LayoutStore();
        ui.initLayout();
        expect(ui.isHomeLayout('youtube')).toBe(true);
        expect(ui.isHomeLayout('reddit')).toBe(false);
    });

    it('setHomeLayout flips one feed type and persists it', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new LayoutStore();

        await ui.setHomeLayout('reddit', true);
        expect(ui.isHomeLayout('reddit')).toBe(true);
        expect(ui.isHomeLayout('youtube')).toBe(false);

        await ui.setHomeLayout('youtube', true);
        await ui.setHomeLayout('reddit', false);
        expect(ui.homeYoutube).toBe(true);
        expect(ui.homeReddit).toBe(false);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.home.reddit', value: 'true' },
            { key: 'ui.home.youtube', value: 'true' },
            { key: 'ui.home.reddit', value: 'false' },
        ]);
    });
});
