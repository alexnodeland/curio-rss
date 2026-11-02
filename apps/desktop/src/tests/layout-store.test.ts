/**
 * Layout store: the modal enum and edit-feed binding, sidebar collapse and
 * focus mode (persisted round trips), the narrow-window auto-collapse, the
 * persisted pane widths (clamped into PANE_LIMITS), the per-feed-type "home"
 * list layout, and reset.
 */
import { LayoutStore, NARROW_QUERY, PANE_LIMITS } from '$lib/state/layout.svelte';
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

    it('sidebar toggles', async () => {
        const harness = installIpcHarness({ set_setting: null });
        const ui = new LayoutStore();
        await ui.toggleSidebar();
        expect(ui.sidebarCollapsed).toBe(true);
        expect(ui.sidebarVisible).toBe(false);
        await ui.toggleSidebar();
        expect(ui.sidebarCollapsed).toBe(false);
        expect(ui.sidebarVisible).toBe(true);
        harness.teardown();
        settingsStore.reset();
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

    it('reset closes any modal and brings every pane back', () => {
        const ui = new LayoutStore();
        ui.openEditFeed(3, 'health');
        ui.sidebarCollapsed = true;
        ui.focusMode = true;
        ui.setNarrow(true);
        ui.sidebarPeek = true;

        ui.reset();

        expect(ui.activeModal).toBeNull();
        expect(ui.editFeedId).toBeNull();
        expect(ui.editFeedSection).toBe('details');
        expect(ui.sidebarCollapsed).toBe(false);
        expect(ui.focusMode).toBe(false);
        expect(ui.narrow).toBe(false);
        expect(ui.sidebarPeek).toBe(false);
        expect(ui.paneMotion).toBe(false);
        expect(ui.sidebarVisible).toBe(true);
        expect(ui.listVisible).toBe(true);
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

/**
 * A settings table the harness keeps in memory, so a write by one store is
 * what the next store's `initLayout` reads back — the round trip a relaunch
 * makes.
 */
function settingsTable(): { harness: IpcHarness; values: Map<string, string> } {
    const values = new Map<string, string>();
    const harness = installIpcHarness({
        get_setting: (args) => values.get(args.key as string) ?? null,
        set_setting: (args) => {
            values.set(args.key as string, args.value as string);
            return null;
        },
    });
    return { harness, values };
}

/** Relaunch: a fresh store adopting whatever the table now holds. */
async function relaunch(): Promise<LayoutStore> {
    settingsStore.reset();
    await settingsStore.load();
    const ui = new LayoutStore();
    ui.initLayout();
    return ui;
}

describe('layout store — sidebar collapse and focus mode', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('the sidebar toggle round-trips through settings', async () => {
        const table = settingsTable();
        harness = table.harness;
        const ui = await relaunch();
        expect(ui.sidebarVisible).toBe(true);

        await ui.toggleSidebar();
        expect(table.values.get('ui.panes.sidebar-collapsed')).toBe('true');
        const collapsed = await relaunch();
        expect(collapsed.sidebarCollapsed).toBe(true);
        expect(collapsed.sidebarVisible).toBe(false);

        await collapsed.toggleSidebar();
        expect(table.values.get('ui.panes.sidebar-collapsed')).toBe('false');
        expect((await relaunch()).sidebarVisible).toBe(true);
    });

    it('focus mode hides both side panes and round-trips through settings', async () => {
        const table = settingsTable();
        harness = table.harness;
        const ui = await relaunch();

        await ui.toggleFocusMode();
        expect(ui.focusMode).toBe(true);
        expect(ui.listVisible).toBe(false);
        expect(ui.sidebarVisible).toBe(false);
        expect(table.values.get('ui.panes.focus-mode')).toBe('true');

        const focused = await relaunch();
        expect(focused.focusMode).toBe(true);
        expect(focused.listVisible).toBe(false);
        // Focus mode leaves the stored sidebar choice alone.
        expect(focused.sidebarCollapsed).toBe(false);

        await focused.toggleFocusMode();
        expect(focused.listVisible).toBe(true);
        expect(focused.sidebarVisible).toBe(true);
        expect((await relaunch()).focusMode).toBe(false);
    });

    it('the sidebar toggle in focus mode leaves it, sidebar showing', async () => {
        const table = settingsTable();
        harness = table.harness;
        const ui = await relaunch();
        await ui.toggleSidebar(); // collapsed
        await ui.toggleFocusMode();

        await ui.toggleSidebar();

        expect(ui.focusMode).toBe(false);
        expect(ui.sidebarVisible).toBe(true);
        expect(ui.listVisible).toBe(true);
        expect(table.values.get('ui.panes.focus-mode')).toBe('false');
        expect(table.values.get('ui.panes.sidebar-collapsed')).toBe('false');
    });

    it('revealSidebar is a no-op when the sidebar already shows', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new LayoutStore();
        await ui.revealSidebar();
        expect(ui.sidebarVisible).toBe(true);
        expect(ui.paneMotion).toBe(false);
        expect(harness.callsFor('set_setting')).toEqual([]);
    });

    it('only a layout change after launch turns pane motion on', async () => {
        const table = settingsTable();
        harness = table.harness;
        table.values.set('ui.panes.sidebar-collapsed', 'true');
        table.values.set('ui.panes.focus-mode', 'true');
        const ui = await relaunch();
        // Adopting the persisted layout must not slide the panes away.
        expect(ui.paneMotion).toBe(false);
        await ui.toggleFocusMode();
        expect(ui.paneMotion).toBe(true);
    });
});

describe('layout store — narrow-window auto-collapse', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    type Listener = (event: MediaQueryListEvent) => void;

    /** A stand-in `matchMedia` whose one query the test flips. */
    function fakeViewport(initiallyNarrow: boolean) {
        const listeners = new Set<Listener>();
        const query = {
            matches: initiallyNarrow,
            addEventListener: (_type: string, listener: Listener) => listeners.add(listener),
            removeEventListener: (_type: string, listener: Listener) => listeners.delete(listener),
        };
        const queries: string[] = [];
        const win = {
            matchMedia: (media: string) => {
                queries.push(media);
                return query as unknown as MediaQueryList;
            },
        };
        const resize = (narrow: boolean): void => {
            query.matches = narrow;
            for (const listener of listeners) {
                listener({ matches: narrow } as MediaQueryListEvent);
            }
        };
        return { win, queries, listeners, resize };
    }

    it('folds the sidebar away below the breakpoint and brings it back above', () => {
        const ui = new LayoutStore();
        const viewport = fakeViewport(false);
        const stop = ui.watchViewport(viewport.win);
        expect(viewport.queries).toEqual([NARROW_QUERY]);
        expect(ui.sidebarVisible).toBe(true);

        viewport.resize(true);
        expect(ui.narrow).toBe(true);
        expect(ui.sidebarVisible).toBe(false);
        expect(ui.listVisible).toBe(true);
        expect(ui.paneMotion).toBe(true);
        // The stored wide choice is untouched.
        expect(ui.sidebarCollapsed).toBe(false);

        viewport.resize(false);
        expect(ui.sidebarVisible).toBe(true);

        stop();
        expect(viewport.listeners.size).toBe(0);
    });

    it('a window that launches narrow starts collapsed, without motion', () => {
        const ui = new LayoutStore();
        ui.watchViewport(fakeViewport(true).win);
        expect(ui.narrow).toBe(true);
        expect(ui.sidebarVisible).toBe(false);
        expect(ui.paneMotion).toBe(false);
    });

    it('a narrow window peeks the sidebar for the session, never persisting it', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new LayoutStore();
        const viewport = fakeViewport(true);
        ui.watchViewport(viewport.win);

        await ui.toggleSidebar();
        expect(ui.sidebarPeek).toBe(true);
        expect(ui.sidebarVisible).toBe(true);
        await ui.toggleSidebar();
        expect(ui.sidebarVisible).toBe(false);
        expect(harness.callsFor('set_setting')).toEqual([]);

        // A peek never outlives a crossing of the breakpoint.
        await ui.toggleSidebar();
        viewport.resize(false);
        viewport.resize(true);
        expect(ui.sidebarPeek).toBe(false);
        expect(ui.sidebarVisible).toBe(false);
    });

    it('a wide collapse stays collapsed through a narrow spell', () => {
        const ui = new LayoutStore();
        ui.sidebarCollapsed = true;
        const viewport = fakeViewport(false);
        ui.watchViewport(viewport.win);
        viewport.resize(true);
        viewport.resize(false);
        expect(ui.sidebarVisible).toBe(false);
    });

    it('revealSidebar peeks a narrow window open without touching the stored choice', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new LayoutStore();
        ui.sidebarCollapsed = true;
        ui.watchViewport(fakeViewport(true).win);

        await ui.revealSidebar();

        expect(ui.sidebarVisible).toBe(true);
        expect(ui.sidebarCollapsed).toBe(true);
        expect(harness.callsFor('set_setting')).toEqual([]);
    });

    it('setNarrow ignores a repeat of the current width class', () => {
        const ui = new LayoutStore();
        ui.setNarrow(false);
        expect(ui.paneMotion).toBe(false);
    });

    it('is a no-op on a host without matchMedia', () => {
        const ui = new LayoutStore();
        const stop = ui.watchViewport({} as Pick<Window, 'matchMedia'>);
        stop();
        expect(ui.narrow).toBe(false);
    });
});
