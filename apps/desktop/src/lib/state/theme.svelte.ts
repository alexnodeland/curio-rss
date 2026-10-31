/**
 * App theme state: the 9 CSS `[data-theme]` themes, user-authored custom
 * themes, and a `system` mode. The theme is persisted twice on purpose: the
 * settings table is the source of truth, and localStorage (`curio-theme`) is a
 * mirror the app.html preload script reads before first paint to kill FOUC.
 */
import { type CustomTheme, buildThemeRule, validateStoredTheme } from '$lib/utils/theme-format';
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

function systemTheme(): ThemeId {
    if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
        return window.matchMedia('(prefers-color-scheme: light)').matches ? 'light' : 'dark';
    }
    return 'dark';
}

export class ThemeStore {
    themePreference: ThemePreference = $state('system');

    /** User-authored custom themes, persisted as a JSON array. */
    customThemes: CustomTheme[] = $state([]);

    /** The concrete theme the preference resolves to (built-in or custom). */
    get resolvedTheme(): ThemeId | CustomThemeId {
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

    /** Test isolation. */
    reset(): void {
        this.themePreference = 'system';
        this.customThemes = [];
    }
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
export const themeStore = new ThemeStore();
