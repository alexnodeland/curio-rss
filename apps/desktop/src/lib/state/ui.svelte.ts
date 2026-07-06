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

export type ModalKind = 'add-feed' | 'settings' | 'help';

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

    activeModal: ModalKind | null = $state(null);

    fontSize: number = $state(16);
    lineHeight: number = $state(1.6);
    measure: number = $state(720);

    toasts: Toast[] = $state([]);

    #nextToastId = 1;
    #toastTimers = new SvelteMap<number, ReturnType<typeof setTimeout>>();

    /** The concrete theme the preference resolves to. */
    get resolvedTheme(): ThemeId {
        return this.themePreference === 'system' ? systemTheme() : this.themePreference;
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

    closeModal(): void {
        this.activeModal = null;
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
        this.themePreference = 'system';
        this.sidebarCollapsed = false;
    }
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
