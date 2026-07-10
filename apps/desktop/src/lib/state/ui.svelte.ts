/**
 * UI chrome state: theme (the 9 CSS `[data-theme]` themes plus a `system`
 * mode), pane layout, the modal enum, reader typography, and toasts.
 * The theme is persisted twice on purpose: the settings table is the source
 * of truth, and localStorage (`curio-theme`) is a mirror the app.html
 * preload script reads before first paint to kill FOUC.
 */
import { commands } from '$lib/bindings';
import { type CustomTheme, buildThemeRule, validateStoredTheme } from '$lib/utils/theme-format';
import { SvelteMap } from 'svelte/reactivity';
import { SETTING_KEYS, settingsStore } from './settings.svelte';

export type ThemeId =
    | 'light'
    | 'dark'
    | 'nord'
    | 'catppuccin'
    | 'dracula'
    | 'gruvbox'
    | 'tokyo-night'
    | 'solarized-dark'
    | 'solarized-light';

/** A user-authored custom theme id, applied as `[data-theme="custom-<id>"]`. */
export type CustomThemeId = `custom-${string}`;

/** What the user picks: a built-in theme, a custom theme, or "follow the OS". */
export type ThemePreference = ThemeId | CustomThemeId | 'system';

export interface ThemeInfo {
    readonly id: ThemeId;
    readonly name: string;
    readonly isDark: boolean;
}

/** The 9 themes shipped in app.css — ids match the `[data-theme]` blocks. */
export const THEMES: readonly ThemeInfo[] = [
    { id: 'light', name: 'Light', isDark: false },
    { id: 'dark', name: 'Dark', isDark: true },
    { id: 'nord', name: 'Nord', isDark: true },
    { id: 'catppuccin', name: 'Catppuccin Mocha', isDark: true },
    { id: 'dracula', name: 'Dracula', isDark: true },
    { id: 'gruvbox', name: 'Gruvbox Dark', isDark: true },
    { id: 'tokyo-night', name: 'Tokyo Night', isDark: true },
    { id: 'solarized-dark', name: 'Solarized Dark', isDark: true },
    { id: 'solarized-light', name: 'Solarized Light', isDark: false },
] as const;

/** The localStorage mirror key — must match the app.html preload script. */
export const THEME_STORAGE_KEY = 'curio-theme';

/**
 * The localStorage mirror holding the *active custom theme's* CSS rule text,
 * so the app.html FOUC preload can re-inject it before first paint. Built-in
 * themes leave this empty. Must match the preload script.
 */
export const THEME_CUSTOM_CSS_KEY = 'curio-theme-custom-css';

/** The runtime `<style>` element id holding every custom theme's rule. */
const CUSTOM_STYLE_ELEMENT_ID = 'curio-custom-themes';
/** The FOUC preload's temporary `<style>`, retired once the app injects. */
const PRELOAD_STYLE_ELEMENT_ID = 'curio-custom-preload';

export function isThemeId(value: string): value is ThemeId {
    return THEMES.some((theme) => theme.id === value);
}

/** A `custom-<slug>` id (the slug shape mirrors theme-format's validator). */
export function isCustomThemeId(value: string): value is CustomThemeId {
    return /^custom-[a-z0-9]+(?:-[a-z0-9]+)*$/.test(value);
}

export function isThemePreference(value: string): value is ThemePreference {
    return value === 'system' || isThemeId(value) || isCustomThemeId(value);
}

export type ModalKind = 'add-feed' | 'settings' | 'help' | 'destinations' | 'edit-feed';

/** Which section the edit-feed modal opens focused on. */
export type EditFeedSection = 'details' | 'health';

/** List row density: `comfortable` shows a body preview + thumbnail; `compact` is a tighter, text-only row. */
export type ReadingDensity = 'comfortable' | 'compact';

/** The default list density. */
export const READING_DENSITY_DEFAULT: ReadingDensity = 'comfortable';

/** Default background-refresh cadence (minutes) when the setting is unset. */
export const REFRESH_INTERVAL_DEFAULT = 30;
/** The cadences offered in Settings → General (minutes; `0` = off). */
export const REFRESH_INTERVAL_OPTIONS: readonly number[] = [0, 15, 30, 60, 180];

/** Reader body font — an id mapped to a concrete CSS font stack. */
export type ReaderFontId = 'sans' | 'serif' | 'mono';

export interface ReaderFontInfo {
    readonly id: ReaderFontId;
    readonly name: string;
    /** The CSS `font-family` value applied to the reader body. */
    readonly stack: string;
}

/**
 * The three reader fonts. `sans`/`mono` reuse the app's own token stacks;
 * `serif` is a self-contained system-serif stack (no bundled webfont — the
 * CSP blocks remote fonts and we ship none).
 */
export const READER_FONTS: readonly ReaderFontInfo[] = [
    { id: 'sans', name: 'Sans', stack: 'var(--font-family)' },
    {
        id: 'serif',
        name: 'Serif',
        stack: 'Iowan Old Style, "Palatino Linotype", Palatino, Georgia, Cambria, "Times New Roman", serif',
    },
    { id: 'mono', name: 'Mono', stack: 'var(--font-mono)' },
] as const;

export function isReaderFontId(value: string): value is ReaderFontId {
    return READER_FONTS.some((font) => font.id === value);
}

/** Typography clamp bounds — the controls and `initTypography` share them. */
export const TYPOGRAPHY_LIMITS = {
    fontSize: { min: 13, max: 24, default: 16 },
    lineHeight: { min: 1.3, max: 2, default: 1.6 },
    measure: { min: 520, max: 960, default: 720 },
} as const;

function clamp(value: number, min: number, max: number): number {
    return Math.min(max, Math.max(min, value));
}

/** Resize bounds per pane — ThreePane and `initLayout` share them. */
export const PANE_LIMITS = {
    sidebar: { min: 200, max: 420 },
    list: { min: 260, max: 600 },
} as const;

export type ToastTone = 'info' | 'success' | 'warning' | 'error';

export interface Toast {
    readonly id: number;
    readonly message: string;
    readonly tone: ToastTone;
}

export const DEFAULT_TOAST_DURATION_MS = 3000;

/** The most toasts kept on screen at once; older ones drop so the stack can't
 *  overflow off the viewport (e.g. a failing sweep firing one per feed). */
export const MAX_TOASTS = 4;

function systemTheme(): ThemeId {
    if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
        return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
    }
    return 'dark';
}

export class UiStore {
    themePreference: ThemePreference = $state('system');

    /** User-authored custom themes, persisted as a JSON array. */
    customThemes: CustomTheme[] = $state([]);

    sidebarCollapsed: boolean = $state(false);
    sidebarWidth: number = $state(280);
    listWidth: number = $state(360);

    /**
     * Per-feed-type list layout: when a YouTube- or Reddit-typed feed is
     * selected, the middle pane can render its "home page" (a video grid /
     * a post feed) instead of the compact rows. Remembered per type.
     */
    homeYoutube: boolean = $state(false);
    homeReddit: boolean = $state(false);

    activeModal: ModalKind | null = $state(null);

    /** The feed the edit-feed modal is bound to while it is open. */
    editFeedId: number | null = $state(null);
    /** Which section the edit-feed modal scrolls to on open. */
    editFeedSection: EditFeedSection = $state('details');

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

    fontSize: number = $state(TYPOGRAPHY_LIMITS.fontSize.default);
    lineHeight: number = $state(TYPOGRAPHY_LIMITS.lineHeight.default);
    measure: number = $state(TYPOGRAPHY_LIMITS.measure.default);
    fontFamily: ReaderFontId = $state('serif');

    toasts: Toast[] = $state([]);

    #nextToastId = 1;
    #toastTimers = new SvelteMap<number, ReturnType<typeof setTimeout>>();

    /** The concrete theme the preference resolves to (built-in or custom). */
    get resolvedTheme(): ThemeId | CustomThemeId {
        return this.themePreference === 'system' ? systemTheme() : this.themePreference;
    }

    /** The CSS `font-family` value for the current reader font. */
    get readerFontStack(): string {
        const font = READER_FONTS.find((candidate) => candidate.id === this.fontFamily);
        return (font ?? READER_FONTS[0]).stack;
    }

    /**
     * Adopts persisted state at startup: the settings table wins; the
     * localStorage mirror (which the preload script already applied) is the
     * fallback. Re-applies the DOM attribute and re-syncs the mirror.
     */
    initTheme(): void {
        const persisted = settingsStore.get(SETTING_KEYS.theme);
        if (persisted !== undefined && isThemePreference(persisted)) {
            this.themePreference = persisted;
        } else {
            const mirrored = readMirror();
            if (mirrored !== null) {
                this.themePreference = mirrored;
            }
        }
        // A custom preference whose theme no longer exists (deleted while
        // active, or a stale mirror) falls back rather than paint unstyled.
        if (isCustomThemeId(this.themePreference) && this.#activeCustom() === undefined) {
            this.themePreference = 'system';
        }
        this.#applyTheme();
    }

    /**
     * Adopts persisted custom themes at startup. Runs BEFORE `initTheme` so a
     * custom preference resolves against a populated list and its rule is
     * injected before first paint. Each stored theme is re-validated (so a
     * tampered settings row can't inject arbitrary CSS).
     */
    initCustomThemes(): void {
        const raw = settingsStore.get(SETTING_KEYS.customThemes);
        if (raw !== undefined) {
            try {
                const parsed: unknown = JSON.parse(raw);
                if (Array.isArray(parsed)) {
                    this.customThemes = parsed
                        .map((entry) => validateStoredTheme(entry))
                        .filter((theme): theme is CustomTheme => theme !== null);
                }
            } catch {
                this.customThemes = [];
            }
        }
        this.#applyCustomStyles();
    }

    /**
     * Adopts persisted pane layout at startup (after `settingsStore.load()`).
     * Unparseable or out-of-range values fall back to the defaults.
     */
    initLayout(): void {
        const collapsed = settingsStore.get(SETTING_KEYS.sidebarCollapsed);
        if (collapsed !== undefined) {
            this.sidebarCollapsed = collapsed === 'true';
        }
        this.sidebarWidth = readWidth(SETTING_KEYS.sidebarWidth, PANE_LIMITS.sidebar, 280);
        this.listWidth = readWidth(SETTING_KEYS.listWidth, PANE_LIMITS.list, 360);
        this.homeYoutube = settingsStore.get(SETTING_KEYS.homeYoutube) === 'true';
        this.homeReddit = settingsStore.get(SETTING_KEYS.homeReddit) === 'true';
    }

    /** Whether the "home" layout is on for a feed type. */
    isHomeLayout(type: 'youtube' | 'reddit'): boolean {
        return type === 'youtube' ? this.homeYoutube : this.homeReddit;
    }

    /** Toggle/set the "home" layout for a feed type, persisted. */
    async setHomeLayout(type: 'youtube' | 'reddit', on: boolean): Promise<void> {
        if (type === 'youtube') {
            this.homeYoutube = on;
            await settingsStore.set(SETTING_KEYS.homeYoutube, String(on));
        } else {
            this.homeReddit = on;
            await settingsStore.set(SETTING_KEYS.homeReddit, String(on));
        }
    }

    /**
     * Adopts persisted reader typography at startup (after
     * `settingsStore.load()`). Each value is clamped into range; anything
     * unparseable or unknown falls back to the default.
     */
    initTypography(): void {
        this.fontSize = readNumber(
            SETTING_KEYS.fontSize,
            TYPOGRAPHY_LIMITS.fontSize,
            TYPOGRAPHY_LIMITS.fontSize.default,
        );
        this.lineHeight = readNumber(
            SETTING_KEYS.lineHeight,
            TYPOGRAPHY_LIMITS.lineHeight,
            TYPOGRAPHY_LIMITS.lineHeight.default,
        );
        this.measure = readNumber(
            SETTING_KEYS.measure,
            TYPOGRAPHY_LIMITS.measure,
            TYPOGRAPHY_LIMITS.measure.default,
        );
        const persistedFont = settingsStore.get(SETTING_KEYS.fontFamily);
        if (persistedFont !== undefined && isReaderFontId(persistedFont)) {
            this.fontFamily = persistedFont;
        }
    }

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

    /** Sets the reader font size (px), clamped, and persists it. */
    async setFontSize(px: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.fontSize;
        this.fontSize = clamp(px, min, max);
        await settingsStore.set(SETTING_KEYS.fontSize, String(this.fontSize));
    }

    /** Sets the reader line height, clamped, and persists it. */
    async setLineHeight(value: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.lineHeight;
        this.lineHeight = clamp(value, min, max);
        await settingsStore.set(SETTING_KEYS.lineHeight, String(this.lineHeight));
    }

    /** Sets the reader measure (max line length, px), clamped, and persists it. */
    async setMeasure(px: number): Promise<void> {
        const { min, max } = TYPOGRAPHY_LIMITS.measure;
        this.measure = clamp(px, min, max);
        await settingsStore.set(SETTING_KEYS.measure, String(this.measure));
    }

    /** Sets the reader font family and persists it. */
    async setFontFamily(id: ReaderFontId): Promise<void> {
        this.fontFamily = id;
        await settingsStore.set(SETTING_KEYS.fontFamily, id);
    }

    /** Picks a theme: applies it, mirrors it, persists it. */
    async setThemePreference(preference: ThemePreference): Promise<void> {
        this.themePreference = preference;
        this.#applyTheme();
        await settingsStore.set(SETTING_KEYS.theme, preference);
    }

    #applyTheme(): void {
        this.#applyCustomStyles();
        if (typeof document !== 'undefined') {
            document.documentElement.setAttribute('data-theme', this.resolvedTheme);
        }
        writeMirror(this.themePreference);
        // Mirror the active custom's rule so the FOUC preload can re-inject it.
        const active = this.#activeCustom();
        writeCustomMirror(active === undefined ? null : buildThemeRule(active));
    }

    /** (Re)injects the `<style>` element holding every custom theme's rule. */
    #applyCustomStyles(): void {
        const cssText = this.customThemes.map((theme) => buildThemeRule(theme)).join('\n');
        writeCustomStyles(cssText);
    }

    /** The custom theme the current preference resolves to, if any. */
    #activeCustom(): CustomTheme | undefined {
        const resolved = this.resolvedTheme;
        if (!isCustomThemeId(resolved)) return undefined;
        const id = resolved.slice('custom-'.length);
        return this.customThemes.find((theme) => theme.id === id);
    }

    /** Whether a custom theme with this id already exists. */
    hasCustomTheme(id: string): boolean {
        return this.customThemes.some((theme) => theme.id === id);
    }

    /**
     * Adds (or replaces, by id) a custom theme: re-injects the stylesheet and
     * persists the list. Does not switch to it — the caller decides.
     */
    async addCustomTheme(theme: CustomTheme): Promise<void> {
        this.customThemes = [
            ...this.customThemes.filter((existing) => existing.id !== theme.id),
            theme,
        ];
        this.#applyCustomStyles();
        await this.#persistCustomThemes();
    }

    /**
     * Removes a custom theme. If it was the active preference, falls back to
     * `system`. Re-injects and persists.
     */
    async removeCustomTheme(id: string): Promise<void> {
        this.customThemes = this.customThemes.filter((theme) => theme.id !== id);
        if (this.themePreference === `custom-${id}`) {
            await this.setThemePreference('system');
        } else {
            this.#applyCustomStyles();
        }
        await this.#persistCustomThemes();
    }

    async #persistCustomThemes(): Promise<void> {
        await settingsStore.set(SETTING_KEYS.customThemes, JSON.stringify(this.customThemes));
    }

    toggleSidebar(): void {
        this.sidebarCollapsed = !this.sidebarCollapsed;
    }

    openModal(modal: ModalKind): void {
        this.activeModal = modal;
    }

    /** Opens the edit-feed modal bound to one feed, optionally scrolled to a section. */
    openEditFeed(feedId: number, section: EditFeedSection = 'details'): void {
        this.editFeedId = feedId;
        this.editFeedSection = section;
        this.activeModal = 'edit-feed';
    }

    closeModal(): void {
        this.activeModal = null;
        this.editFeedId = null;
        this.editFeedSection = 'details';
    }

    /** Queues a toast; duration 0 keeps it until dismissed. */
    showToast(
        message: string,
        tone: ToastTone = 'info',
        durationMs: number = DEFAULT_TOAST_DURATION_MS,
    ): number {
        // Dedup: an identical message+tone already showing just refreshes its
        // timer, so a repeated failure doesn't stack N copies.
        const existing = this.toasts.find(
            (toast) => toast.message === message && toast.tone === tone,
        );
        if (existing !== undefined) {
            this.#armToastTimer(existing.id, durationMs);
            return existing.id;
        }
        const id = this.#nextToastId;
        this.#nextToastId += 1;
        let next = [...this.toasts, { id, message, tone }];
        // Cap the stack so it can't overflow off-screen; drop the oldest.
        if (next.length > MAX_TOASTS) {
            for (const toast of next.slice(0, next.length - MAX_TOASTS)) {
                this.#clearToastTimer(toast.id);
            }
            next = next.slice(next.length - MAX_TOASTS);
        }
        this.toasts = next;
        this.#armToastTimer(id, durationMs);
        return id;
    }

    /** (Re)arms the auto-dismiss timer for a toast; duration 0 means sticky. */
    #armToastTimer(id: number, durationMs: number): void {
        this.#clearToastTimer(id);
        if (durationMs > 0) {
            this.#toastTimers.set(
                id,
                setTimeout(() => this.dismissToast(id), durationMs),
            );
        }
    }

    #clearToastTimer(id: number): void {
        const timer = this.#toastTimers.get(id);
        if (timer !== undefined) {
            clearTimeout(timer);
            this.#toastTimers.delete(id);
        }
    }

    dismissToast(id: number): void {
        this.#clearToastTimer(id);
        this.toasts = this.toasts.filter((toast) => toast.id !== id);
    }

    /** Test isolation. */
    reset(): void {
        for (const timer of this.#toastTimers.values()) {
            clearTimeout(timer);
        }
        this.#toastTimers.clear();
        this.toasts = [];
        this.activeModal = null;
        this.editFeedId = null;
        this.editFeedSection = 'details';
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
        this.themePreference = 'system';
        this.customThemes = [];
        this.sidebarCollapsed = false;
        this.fontSize = TYPOGRAPHY_LIMITS.fontSize.default;
        this.lineHeight = TYPOGRAPHY_LIMITS.lineHeight.default;
        this.measure = TYPOGRAPHY_LIMITS.measure.default;
        this.fontFamily = 'serif';
    }
}

function readNumber(
    key: (typeof SETTING_KEYS)[keyof typeof SETTING_KEYS],
    limits: { min: number; max: number },
    fallback: number,
): number {
    const raw = settingsStore.get(key);
    if (raw === undefined) {
        return fallback;
    }
    const parsed = Number.parseFloat(raw);
    if (Number.isNaN(parsed)) {
        return fallback;
    }
    return clamp(parsed, limits.min, limits.max);
}

function readWidth(
    key: (typeof SETTING_KEYS)[keyof typeof SETTING_KEYS],
    limits: { min: number; max: number },
    fallback: number,
): number {
    const raw = settingsStore.get(key);
    if (raw === undefined) {
        return fallback;
    }
    const parsed = Number.parseInt(raw, 10);
    if (Number.isNaN(parsed)) {
        return fallback;
    }
    return Math.min(limits.max, Math.max(limits.min, parsed));
}

function readMirror(): ThemePreference | null {
    try {
        const stored = window.localStorage.getItem(THEME_STORAGE_KEY);
        return stored !== null && isThemePreference(stored) ? stored : null;
    } catch {
        return null;
    }
}

function writeMirror(preference: ThemePreference): void {
    try {
        window.localStorage.setItem(THEME_STORAGE_KEY, preference);
    } catch {
        // localStorage unavailable — the preload falls back to the system.
    }
}

/** Mirrors the active custom theme's rule text for the FOUC preload. */
function writeCustomMirror(ruleText: string | null): void {
    try {
        if (ruleText === null) {
            window.localStorage.removeItem(THEME_CUSTOM_CSS_KEY);
        } else {
            window.localStorage.setItem(THEME_CUSTOM_CSS_KEY, ruleText);
        }
    } catch {
        // localStorage unavailable — a custom theme flashes once, then settles.
    }
}

/**
 * Writes every custom theme's rule into a single managed `<style>` element and
 * retires the preload's temporary one. A plain `<style>` is used rather than
 * `adoptedStyleSheets` so it works identically in the webview and under jsdom.
 */
function writeCustomStyles(cssText: string): void {
    if (typeof document === 'undefined') return;
    let element = document.getElementById(CUSTOM_STYLE_ELEMENT_ID);
    if (element === null) {
        element = document.createElement('style');
        element.id = CUSTOM_STYLE_ELEMENT_ID;
        document.head.append(element);
    }
    element.textContent = cssText;
    document.getElementById(PRELOAD_STYLE_ELEMENT_ID)?.remove();
}

/** The app-wide singleton. */
export const uiStore = new UiStore();
