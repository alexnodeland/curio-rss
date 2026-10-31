/**
 * App behaviour preferences surfaced in Settings: reading (remote favicons,
 * media prefetch, mark-on-scroll, list density), background refresh, OS
 * notifications, and app updates. Each is persisted to the settings table;
 * several are read Rust-side (the refresh scheduler, the notifier) straight
 * from there, so this store mirrors them for the Settings UI.
 */
import { commands } from '$lib/bindings';
import { SETTING_KEYS, settingsStore } from './settings.svelte';

/** List row density: `comfortable` shows a body preview + thumbnail; `compact` is a tighter, text-only row. */
export type ReadingDensity = 'comfortable' | 'compact';

/** The default list density. */
export const READING_DENSITY_DEFAULT: ReadingDensity = 'comfortable';

/** Default background-refresh cadence (minutes) when the setting is unset. */
export const REFRESH_INTERVAL_DEFAULT = 30;
/** The cadences offered in Settings → General (minutes; `0` = off). */
export const REFRESH_INTERVAL_OPTIONS: readonly number[] = [0, 15, 30, 60, 180];

export class PrefsStore {
    /**
     * Opt-in: when a site declares no favicon, may Curio fall back to
     * Google's favicon service? Off by default — a privacy choice, since it
     * tells Google which sites you preview (see PRIVACY.md).
     */
    allowRemoteFavicon: boolean = $state(false);

    /**
     * Load remote media (YouTube thumbnails, images embedded in posts)
     * through the policed image cache. Off by default — privacy-first, no
     * network until asked; on, real thumbnails replace the gradient posters
     * and inline post images resolve via the `asset:` cache.
     */
    mediaPrefetch: boolean = $state(false);

    /**
     * Session flag (not persisted): the reader's "images are off in this
     * article" hint has been dismissed. Reappears next launch, so a user who
     * keeps media off isn't permanently nagged but is reminded occasionally.
     */
    mediaHintDismissed: boolean = $state(false);

    /**
     * Mark articles read as they scroll up out of the list viewport. Off by
     * default — reading is deliberate, not incidental; on, the unread count
     * melts away as you skim past, the way a river reader expects.
     */
    markOnScroll: boolean = $state(false);

    /**
     * List row density. `comfortable` (default) gives each row a body-text
     * preview and thumbnail; `compact` drops both for a tighter, text-only
     * row. Drives the virtualizer's fixed row height, so the whole list
     * switches heights together. See {@link setReadingDensity}.
     */
    readingDensity: ReadingDensity = $state(READING_DENSITY_DEFAULT);

    /**
     * Background-refresh cadence in minutes (`0` = off); the Rust scheduler
     * reads this from settings each tick. Default 30 — this state mirrors it
     * for the Settings UI. See {@link setRefreshIntervalMinutes}.
     */
    refreshIntervalMinutes: number = $state(REFRESH_INTERVAL_DEFAULT);
    /** Whether to refresh once shortly after launch (default on). */
    refreshOnLaunch: boolean = $state(true);

    /**
     * OS-notification prefs, read Rust-side by the scheduler after a background
     * refresh. Master `notifyEnabled` is opt-in (default off, so no OS prompt
     * until asked); the three per-event flags default on; the quiet window is
     * two `"HH:MM"` strings (empty = no window). See {@link setNotifyEnabled}.
     */
    notifyEnabled: boolean = $state(false);
    notifyNewArticles: boolean = $state(true);
    notifyErrors: boolean = $state(true);
    notifyFeedDead: boolean = $state(true);
    notifyQuietStart: string = $state('');
    notifyQuietEnd: string = $state('');

    /**
     * App auto-update prefs. `updatesAutoCheck` (default on) checks the release
     * feed on launch and surfaces an available update; `updatesAutoInstall`
     * (default off — the aggressive one) downloads, installs, and relaunches
     * without asking. Read by the frontend updater flow (see
     * `lib/utils/updates.ts`). See {@link setUpdatesAutoCheck}.
     */
    updatesAutoCheck: boolean = $state(true);
    updatesAutoInstall: boolean = $state(false);

    /** Adopts persisted reading prefs at startup (after `settingsStore.load()`). */
    initReading(): void {
        this.allowRemoteFavicon = settingsStore.get(SETTING_KEYS.faviconAllowRemote) === 'true';
        this.mediaPrefetch = settingsStore.get(SETTING_KEYS.mediaPrefetch) === 'true';
        this.markOnScroll = settingsStore.get(SETTING_KEYS.markOnScroll) === 'true';
        this.readingDensity =
            settingsStore.get(SETTING_KEYS.readingDensity) === 'compact'
                ? 'compact'
                : 'comfortable';
    }

    /** Sets (and persists) the Google-favicon-fallback opt-in. */
    async setAllowRemoteFavicon(value: boolean): Promise<void> {
        this.allowRemoteFavicon = value;
        await settingsStore.set(SETTING_KEYS.faviconAllowRemote, String(value));
    }

    /** Sets (and persists) the remote-media prefetch opt-in. */
    async setMediaPrefetch(value: boolean): Promise<void> {
        this.mediaPrefetch = value;
        await settingsStore.set(SETTING_KEYS.mediaPrefetch, String(value));
    }

    /** Hides the reader's "images are off" hint for the rest of the session. */
    dismissMediaHint(): void {
        this.mediaHintDismissed = true;
    }

    /** Sets (and persists) the mark-as-read-on-scroll opt-in. */
    async setMarkOnScroll(value: boolean): Promise<void> {
        this.markOnScroll = value;
        await settingsStore.set(SETTING_KEYS.markOnScroll, String(value));
    }

    /** Sets (and persists) the list row density. */
    async setReadingDensity(value: ReadingDensity): Promise<void> {
        this.readingDensity = value;
        await settingsStore.set(SETTING_KEYS.readingDensity, value);
    }

    /** Adopts persisted background-refresh prefs at startup. */
    initRefresh(): void {
        const raw = settingsStore.get(SETTING_KEYS.refreshIntervalMinutes);
        const parsed = raw === undefined ? Number.NaN : Number.parseInt(raw, 10);
        this.refreshIntervalMinutes = Number.isFinite(parsed) ? parsed : REFRESH_INTERVAL_DEFAULT;
        this.refreshOnLaunch = settingsStore.get(SETTING_KEYS.refreshOnLaunch) !== 'false';
    }

    /**
     * Sets (and persists) the background-refresh cadence in minutes (`0` = off).
     * The Rust scheduler re-reads the setting each tick, so this takes effect
     * within a minute without a restart.
     */
    async setRefreshIntervalMinutes(minutes: number): Promise<void> {
        this.refreshIntervalMinutes = minutes;
        await settingsStore.set(SETTING_KEYS.refreshIntervalMinutes, String(minutes));
    }

    /** Sets (and persists) whether to refresh once shortly after launch. */
    async setRefreshOnLaunch(value: boolean): Promise<void> {
        this.refreshOnLaunch = value;
        await settingsStore.set(SETTING_KEYS.refreshOnLaunch, String(value));
    }

    /** Adopts persisted notification prefs at startup. */
    initNotifications(): void {
        this.notifyEnabled = settingsStore.get(SETTING_KEYS.notifyEnabled) === 'true';
        this.notifyNewArticles = settingsStore.get(SETTING_KEYS.notifyNewArticles) !== 'false';
        this.notifyErrors = settingsStore.get(SETTING_KEYS.notifyErrors) !== 'false';
        this.notifyFeedDead = settingsStore.get(SETTING_KEYS.notifyFeedDead) !== 'false';
        this.notifyQuietStart = settingsStore.get(SETTING_KEYS.notifyQuietStart) ?? '';
        this.notifyQuietEnd = settingsStore.get(SETTING_KEYS.notifyQuietEnd) ?? '';
    }

    /**
     * Turns notifications on/off. When turning on, requests OS permission now
     * (so the prompt lands here, not at a later background refresh) and returns
     * whether it was granted — the caller can nudge the user if the OS blocked
     * it. The persisted pref is independent of the OS grant; the scheduler
     * re-checks permission on every refresh.
     */
    async setNotifyEnabled(value: boolean): Promise<boolean> {
        this.notifyEnabled = value;
        await settingsStore.set(SETTING_KEYS.notifyEnabled, String(value));
        if (!value) {
            return true;
        }
        const result = await commands.requestNotificationPermission();
        return result.status === 'ok' ? result.data : false;
    }

    /** Sets (and persists) a per-event notification toggle. */
    async setNotifyNewArticles(value: boolean): Promise<void> {
        this.notifyNewArticles = value;
        await settingsStore.set(SETTING_KEYS.notifyNewArticles, String(value));
    }
    async setNotifyErrors(value: boolean): Promise<void> {
        this.notifyErrors = value;
        await settingsStore.set(SETTING_KEYS.notifyErrors, String(value));
    }
    async setNotifyFeedDead(value: boolean): Promise<void> {
        this.notifyFeedDead = value;
        await settingsStore.set(SETTING_KEYS.notifyFeedDead, String(value));
    }

    /** Sets (and persists) a quiet-hours bound (`""` clears it). */
    async setNotifyQuietStart(value: string): Promise<void> {
        this.notifyQuietStart = value;
        await settingsStore.set(SETTING_KEYS.notifyQuietStart, value);
    }
    async setNotifyQuietEnd(value: string): Promise<void> {
        this.notifyQuietEnd = value;
        await settingsStore.set(SETTING_KEYS.notifyQuietEnd, value);
    }

    /** Adopts persisted app-update prefs at startup. */
    initUpdates(): void {
        this.updatesAutoCheck = settingsStore.get(SETTING_KEYS.updatesAutoCheck) !== 'false';
        this.updatesAutoInstall = settingsStore.get(SETTING_KEYS.updatesAutoInstall) === 'true';
    }

    /** Sets (and persists) whether to check for updates on launch. */
    async setUpdatesAutoCheck(value: boolean): Promise<void> {
        this.updatesAutoCheck = value;
        await settingsStore.set(SETTING_KEYS.updatesAutoCheck, String(value));
    }

    /** Sets (and persists) whether to install updates automatically. */
    async setUpdatesAutoInstall(value: boolean): Promise<void> {
        this.updatesAutoInstall = value;
        await settingsStore.set(SETTING_KEYS.updatesAutoInstall, String(value));
    }

    /** Test isolation. */
    reset(): void {
        this.allowRemoteFavicon = false;
        this.mediaPrefetch = false;
        this.mediaHintDismissed = false;
        this.markOnScroll = false;
        this.readingDensity = READING_DENSITY_DEFAULT;
        this.refreshIntervalMinutes = REFRESH_INTERVAL_DEFAULT;
        this.refreshOnLaunch = true;
        this.notifyEnabled = false;
        this.notifyNewArticles = true;
        this.notifyErrors = true;
        this.notifyFeedDead = true;
        this.notifyQuietStart = '';
        this.notifyQuietEnd = '';
        this.updatesAutoCheck = true;
        this.updatesAutoInstall = false;
    }
}

/** The app-wide singleton. */
export const prefsStore = new PrefsStore();
