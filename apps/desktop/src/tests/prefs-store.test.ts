/**
 * Prefs store: the Settings-surface behaviour preferences — reading
 * (favicons, media, mark-on-scroll, density), background refresh, OS
 * notifications, app updates — their persisted init, setters, and reset.
 */
import { PrefsStore } from '$lib/state/prefs.svelte';
import { settingsStore } from '$lib/state/settings.svelte';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

describe('prefs store — background refresh', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('defaults to a 30-minute interval and on-launch on', () => {
        const ui = new PrefsStore();
        expect(ui.refreshIntervalMinutes).toBe(30);
        expect(ui.refreshOnLaunch).toBe(true);
    });

    it('initRefresh adopts persisted values (including 0 = off and on-launch off)', async () => {
        harness = installIpcHarness({
            get_setting: (args) => {
                if (args.key === 'ui.refresh.interval-minutes') return '0';
                if (args.key === 'ui.refresh.on-launch') return 'false';
                return null;
            },
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        ui.initRefresh();
        expect(ui.refreshIntervalMinutes).toBe(0);
        expect(ui.refreshOnLaunch).toBe(false);
    });

    it('initRefresh falls back to the default interval for garbage', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.refresh.interval-minutes' ? 'often' : null),
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        ui.initRefresh();
        expect(ui.refreshIntervalMinutes).toBe(30);
        expect(ui.refreshOnLaunch).toBe(true);
    });

    it('the setters persist to the settings table', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        await ui.setRefreshIntervalMinutes(60);
        await ui.setRefreshOnLaunch(false);

        expect(ui.refreshIntervalMinutes).toBe(60);
        expect(ui.refreshOnLaunch).toBe(false);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.refresh.interval-minutes', value: '60' },
            { key: 'ui.refresh.on-launch', value: 'false' },
        ]);
    });
});

describe('prefs store — reading', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('defaults to comfortable', () => {
        expect(new PrefsStore().readingDensity).toBe('comfortable');
    });

    it('initReading adopts a persisted compact preference', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.reading.density' ? 'compact' : null),
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        ui.initReading();
        expect(ui.readingDensity).toBe('compact');
    });

    it('initReading falls back to comfortable for an unknown value', async () => {
        harness = installIpcHarness({
            get_setting: (args) => (args.key === 'ui.reading.density' ? 'weird' : null),
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        ui.initReading();
        expect(ui.readingDensity).toBe('comfortable');
    });

    it('setReadingDensity persists the chosen density', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        await ui.setReadingDensity('compact');
        expect(ui.readingDensity).toBe('compact');
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.reading.density', value: 'compact' },
        ]);
    });

    it('the media-hint dismissal is a one-shot session flag', () => {
        const ui = new PrefsStore();
        expect(ui.mediaHintDismissed).toBe(false);
        ui.dismissMediaHint();
        expect(ui.mediaHintDismissed).toBe(true);
    });

    it('initReading adopts the privacy opt-ins and mark-on-scroll', async () => {
        const persisted: Record<string, string> = {
            'ui.favicon.allow-remote': 'true',
            'ui.media.prefetch': 'true',
            'ui.reading.mark-on-scroll': 'true',
        };
        harness = installIpcHarness({
            get_setting: (args) => persisted[args.key as string] ?? null,
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        expect(ui.allowRemoteFavicon).toBe(false);
        expect(ui.mediaPrefetch).toBe(false);
        expect(ui.markOnScroll).toBe(false);
        ui.initReading();
        expect(ui.allowRemoteFavicon).toBe(true);
        expect(ui.mediaPrefetch).toBe(true);
        expect(ui.markOnScroll).toBe(true);
    });

    it('the reading opt-in setters persist to the settings table', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        await ui.setAllowRemoteFavicon(true);
        await ui.setMediaPrefetch(true);
        await ui.setMarkOnScroll(true);
        expect(ui.allowRemoteFavicon).toBe(true);
        expect(ui.mediaPrefetch).toBe(true);
        expect(ui.markOnScroll).toBe(true);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.favicon.allow-remote', value: 'true' },
            { key: 'ui.media.prefetch', value: 'true' },
            { key: 'ui.reading.mark-on-scroll', value: 'true' },
        ]);
    });
});

describe('prefs store — notifications', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('defaults to off, per-event on, no quiet window', () => {
        const ui = new PrefsStore();
        expect(ui.notifyEnabled).toBe(false);
        expect(ui.notifyNewArticles).toBe(true);
        expect(ui.notifyErrors).toBe(true);
        expect(ui.notifyFeedDead).toBe(true);
        expect(ui.notifyQuietStart).toBe('');
        expect(ui.notifyQuietEnd).toBe('');
    });

    it('initNotifications adopts persisted values (per-event defaults on)', async () => {
        const persisted: Record<string, string> = {
            'ui.notify.enabled': 'true',
            'ui.notify.new-articles': 'false',
            'ui.notify.quiet-start': '22:00',
            'ui.notify.quiet-end': '07:00',
        };
        harness = installIpcHarness({
            get_setting: (args) => persisted[args.key as string] ?? null,
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        ui.initNotifications();
        expect(ui.notifyEnabled).toBe(true);
        expect(ui.notifyNewArticles).toBe(false);
        expect(ui.notifyErrors).toBe(true); // unset → default on
        expect(ui.notifyQuietStart).toBe('22:00');
        expect(ui.notifyQuietEnd).toBe('07:00');
    });

    it('enabling requests OS permission and reports the grant', async () => {
        harness = installIpcHarness({ set_setting: null, request_notification_permission: true });
        const ui = new PrefsStore();

        const granted = await ui.setNotifyEnabled(true);
        expect(granted).toBe(true);
        expect(ui.notifyEnabled).toBe(true);
        expect(harness.callsFor('request_notification_permission')).toHaveLength(1);
    });

    it('disabling persists without touching OS permission', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        expect(await ui.setNotifyEnabled(false)).toBe(true);
        expect(harness.callsFor('request_notification_permission')).toHaveLength(0);
    });

    it('the per-event setters persist each toggle', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        await ui.setNotifyNewArticles(false);
        await ui.setNotifyErrors(false);
        await ui.setNotifyFeedDead(false);
        expect(ui.notifyNewArticles).toBe(false);
        expect(ui.notifyErrors).toBe(false);
        expect(ui.notifyFeedDead).toBe(false);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.notify.new-articles', value: 'false' },
            { key: 'ui.notify.errors', value: 'false' },
            { key: 'ui.notify.feed-dead', value: 'false' },
        ]);
    });

    it('the quiet-hours setters persist the HH:MM strings', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        await ui.setNotifyQuietStart('23:30');
        await ui.setNotifyQuietEnd('06:15');
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.notify.quiet-start', value: '23:30' },
            { key: 'ui.notify.quiet-end', value: '06:15' },
        ]);
    });
});

describe('prefs store — updates', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('defaults to auto-check on, auto-install off', () => {
        const ui = new PrefsStore();
        expect(ui.updatesAutoCheck).toBe(true);
        expect(ui.updatesAutoInstall).toBe(false);
    });

    it('initUpdates adopts persisted values', async () => {
        harness = installIpcHarness({
            get_setting: (args) => {
                if (args.key === 'ui.updates.auto-check') return 'false';
                if (args.key === 'ui.updates.auto-install') return 'true';
                return null;
            },
        });
        await settingsStore.load();

        const ui = new PrefsStore();
        ui.initUpdates();
        expect(ui.updatesAutoCheck).toBe(false);
        expect(ui.updatesAutoInstall).toBe(true);
    });

    it('the setters persist to the settings table', async () => {
        harness = installIpcHarness({ set_setting: null });
        const ui = new PrefsStore();

        await ui.setUpdatesAutoCheck(false);
        await ui.setUpdatesAutoInstall(true);
        expect(ui.updatesAutoCheck).toBe(false);
        expect(ui.updatesAutoInstall).toBe(true);
        expect(harness.callsFor('set_setting')).toEqual([
            { key: 'ui.updates.auto-check', value: 'false' },
            { key: 'ui.updates.auto-install', value: 'true' },
        ]);
    });
});

describe('prefs store — reset', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        settingsStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('reset restores every preference to its default', async () => {
        harness = installIpcHarness({ set_setting: null, request_notification_permission: true });
        const ui = new PrefsStore();

        await ui.setAllowRemoteFavicon(true);
        await ui.setMediaPrefetch(true);
        ui.dismissMediaHint();
        await ui.setMarkOnScroll(true);
        await ui.setReadingDensity('compact');
        await ui.setRefreshIntervalMinutes(180);
        await ui.setRefreshOnLaunch(false);
        await ui.setNotifyEnabled(true);
        await ui.setNotifyNewArticles(false);
        await ui.setNotifyErrors(false);
        await ui.setNotifyFeedDead(false);
        await ui.setNotifyQuietStart('22:00');
        await ui.setNotifyQuietEnd('07:00');
        await ui.setUpdatesAutoCheck(false);
        await ui.setUpdatesAutoInstall(true);

        ui.reset();

        const fresh = new PrefsStore();
        for (const key of [
            'allowRemoteFavicon',
            'mediaPrefetch',
            'mediaHintDismissed',
            'markOnScroll',
            'readingDensity',
            'refreshIntervalMinutes',
            'refreshOnLaunch',
            'notifyEnabled',
            'notifyNewArticles',
            'notifyErrors',
            'notifyFeedDead',
            'notifyQuietStart',
            'notifyQuietEnd',
            'updatesAutoCheck',
            'updatesAutoInstall',
        ] as const) {
            expect(ui[key], key).toBe(fresh[key]);
        }
    });
});
