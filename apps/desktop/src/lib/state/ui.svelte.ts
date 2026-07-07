/**
 * UI chrome state: theme (the 9 CSS `[data-theme]` themes plus a `system`
 * mode), pane layout, the modal enum, reader typography, and toasts.
 * The theme is persisted twice on purpose: the settings table is the source
 * of truth, and localStorage (`curio-theme`) is a mirror the app.html
 * preload script reads before first paint to kill FOUC.
 */
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

/** What the user picks: a concrete theme or "follow the OS". */
export type ThemePreference = ThemeId | 'system';

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

export function isThemeId(value: string): value is ThemeId {
    return THEMES.some((theme) => theme.id === value);
}

export function isThemePreference(value: string): value is ThemePreference {
    return value === 'system' || isThemeId(value);
}

export type ModalKind = 'add-feed' | 'settings' | 'help' | 'destinations' | 'feed-health';

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

function systemTheme(): ThemeId {
    if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
        return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
    }
    return 'dark';
}

export class UiStore {
    themePreference: ThemePreference = $state('system');

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

    /** The feed the feed-health panel is bound to while it is open. */
    healthFeedId: number | null = $state(null);

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
     * Mark articles read as they scroll up out of the list viewport. Off by
     * default — reading is deliberate, not incidental; on, the unread count
     * melts away as you skim past, the way a river reader expects.
     */
    markOnScroll: boolean = $state(false);

    fontSize: number = $state(TYPOGRAPHY_LIMITS.fontSize.default);
    lineHeight: number = $state(TYPOGRAPHY_LIMITS.lineHeight.default);
    measure: number = $state(TYPOGRAPHY_LIMITS.measure.default);
    fontFamily: ReaderFontId = $state('serif');

    toasts: Toast[] = $state([]);

    #nextToastId = 1;
    #toastTimers = new SvelteMap<number, ReturnType<typeof setTimeout>>();

    /** The concrete theme the preference resolves to. */
    get resolvedTheme(): ThemeId {
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
        this.#applyTheme();
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

    /** Sets (and persists) the mark-as-read-on-scroll opt-in. */
    async setMarkOnScroll(value: boolean): Promise<void> {
        this.markOnScroll = value;
        await settingsStore.set(SETTING_KEYS.markOnScroll, String(value));
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
        if (typeof document !== 'undefined') {
            document.documentElement.setAttribute('data-theme', this.resolvedTheme);
        }
        writeMirror(this.themePreference);
    }

    toggleSidebar(): void {
        this.sidebarCollapsed = !this.sidebarCollapsed;
    }

    openModal(modal: ModalKind): void {
        this.activeModal = modal;
    }

    /** Opens the feed-health panel bound to one feed. */
    openHealth(feedId: number): void {
        this.healthFeedId = feedId;
        this.activeModal = 'feed-health';
    }

    closeModal(): void {
        this.activeModal = null;
        this.healthFeedId = null;
    }

    /** Queues a toast; duration 0 keeps it until dismissed. */
    showToast(
        message: string,
        tone: ToastTone = 'info',
        durationMs: number = DEFAULT_TOAST_DURATION_MS,
    ): number {
        const id = this.#nextToastId;
        this.#nextToastId += 1;
        this.toasts = [...this.toasts, { id, message, tone }];
        if (durationMs > 0) {
            const timer = setTimeout(() => this.dismissToast(id), durationMs);
            this.#toastTimers.set(id, timer);
        }
        return id;
    }

    dismissToast(id: number): void {
        const timer = this.#toastTimers.get(id);
        if (timer !== undefined) {
            clearTimeout(timer);
            this.#toastTimers.delete(id);
        }
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
        this.healthFeedId = null;
        this.allowRemoteFavicon = false;
        this.mediaPrefetch = false;
        this.markOnScroll = false;
        this.themePreference = 'system';
        this.sidebarCollapsed = false;
        this.fontSize = TYPOGRAPHY_LIMITS.fontSize.default;
        this.lineHeight = TYPOGRAPHY_LIMITS.lineHeight.default;
        this.measure = TYPOGRAPHY_LIMITS.measure.default;
        this.fontFamily = 'sans';
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

/** The app-wide singleton. */
export const uiStore = new UiStore();
