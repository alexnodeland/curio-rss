/**
 * Persisted preferences — a reactive mirror of the profile's settings table,
 * read and written through the generated `get_setting`/`set_setting`
 * wrappers under namespaced `ui.*` keys. The backend owns storage; this
 * module owns nothing but the in-memory copy — plus a localStorage *boot
 * mirror* of the few keys that shape the first paint (see {@link BOOT_KEYS}),
 * so the shell boots at its settled layout instead of jumping when `load()`
 * resolves. (The `destinations` key is facade-reserved — the backend rejects
 * it; no key here may collide.)
 */
import { type CommandError, commands } from '$lib/bindings';
import type { CommandResult } from './query-cache.svelte';

/** Every persisted UI preference key — the single registry. */
export const SETTING_KEYS = {
    theme: 'ui.theme',
    fontSize: 'ui.typography.font-size',
    lineHeight: 'ui.typography.line-height',
    measure: 'ui.typography.measure',
    fontFamily: 'ui.typography.font-family',
    paragraphSpacing: 'ui.typography.paragraph-spacing',
    textAlign: 'ui.typography.text-align',
    fontWeight: 'ui.typography.font-weight',
    letterSpacing: 'ui.typography.letter-spacing',
    hyphenate: 'ui.typography.hyphenate',
    readingTheme: 'ui.typography.reading-theme',
    sidebarCollapsed: 'ui.panes.sidebar-collapsed',
    focusMode: 'ui.panes.focus-mode',
    sidebarWidth: 'ui.panes.sidebar-width',
    listWidth: 'ui.panes.list-width',
    promoteDestination: 'ui.promote.destination',
    faviconAllowRemote: 'ui.favicon.allow-remote',
    homeYoutube: 'ui.home.youtube',
    homeReddit: 'ui.home.reddit',
    mediaPrefetch: 'ui.media.prefetch',
    markOnScroll: 'ui.reading.mark-on-scroll',
    readingDensity: 'ui.reading.density',
    speechRate: 'ui.reading.speech-rate',
    locale: 'ui.locale',
    customThemes: 'ui.themes.custom',
    collapsedFolders: 'ui.sidebar.collapsed-folders',
    pendingFolders: 'ui.sidebar.pending-folders',
    refreshIntervalMinutes: 'ui.refresh.interval-minutes',
    refreshOnLaunch: 'ui.refresh.on-launch',
    notifyEnabled: 'ui.notify.enabled',
    notifyNewArticles: 'ui.notify.new-articles',
    notifyErrors: 'ui.notify.errors',
    notifyFeedDead: 'ui.notify.feed-dead',
    notifyQuietStart: 'ui.notify.quiet-start',
    notifyQuietEnd: 'ui.notify.quiet-end',
    notifyMutedFeeds: 'ui.notify.muted-feeds',
    updatesAutoCheck: 'ui.updates.auto-check',
    updatesAutoInstall: 'ui.updates.auto-install',
} as const;

export type SettingKey = (typeof SETTING_KEYS)[keyof typeof SETTING_KEYS];

const ALL_KEYS: readonly SettingKey[] = Object.values(SETTING_KEYS);

/** The localStorage key holding the boot mirror (a JSON object, key → value). */
export const BOOT_MIRROR_STORAGE_KEY = 'curio-boot-settings';

/**
 * The settings that shape the first paint — pane widths, collapse and focus
 * mode, the list density and "home" layouts, reader typography and the
 * locale. They are mirrored to localStorage so the stores can adopt them
 * before the backend answers (the theme has its own pre-paint mirror in
 * app.html). The settings table stays the source of truth: every `load()`
 * rewrites the mirror, and every `set()` of one of these keys updates it.
 *
 * Only presentation belongs here. Behaviour and privacy opt-ins (remote
 * favicons, media prefetch, mark-on-scroll) shape no layout, so they stay
 * off until the backend answers rather than trusting an unauthoritative
 * cache that could switch on a network fetch or a state change.
 */
export const BOOT_KEYS: readonly SettingKey[] = [
    SETTING_KEYS.sidebarCollapsed,
    SETTING_KEYS.focusMode,
    SETTING_KEYS.sidebarWidth,
    SETTING_KEYS.listWidth,
    SETTING_KEYS.homeYoutube,
    SETTING_KEYS.homeReddit,
    SETTING_KEYS.readingDensity,
    SETTING_KEYS.fontSize,
    SETTING_KEYS.lineHeight,
    SETTING_KEYS.measure,
    SETTING_KEYS.fontFamily,
    SETTING_KEYS.paragraphSpacing,
    SETTING_KEYS.textAlign,
    SETTING_KEYS.fontWeight,
    SETTING_KEYS.letterSpacing,
    SETTING_KEYS.hyphenate,
    SETTING_KEYS.readingTheme,
    SETTING_KEYS.locale,
];

function isBootKey(key: string): key is SettingKey {
    return (BOOT_KEYS as readonly string[]).includes(key);
}

export class SettingsStore {
    #values: Partial<Record<SettingKey, string>> = $state({});

    /** True once `load()` has settled (even partially). */
    loaded: boolean = $state(false);

    /** The first load/write failure, for the doctor surface. */
    error: CommandError | null = $state(null);

    /**
     * Seeds the in-memory copy from the localStorage boot mirror, so the
     * stores' `init*()` can adopt the last-known layout before `load()`
     * answers. Only {@link BOOT_KEYS} with string values are taken; a missing,
     * unreadable or malformed mirror leaves the copy empty (defaults). A no-op
     * once `load()` has settled — the backend's values always win.
     */
    primeFromMirror(): void {
        if (this.loaded) {
            return;
        }
        const values: Partial<Record<SettingKey, string>> = {};
        for (const [key, value] of Object.entries(readBootMirror())) {
            if (isBootKey(key) && typeof value === 'string') {
                values[key] = value;
            }
        }
        this.#values = values;
    }

    /** Reads every registered key from the backend into the mirror. */
    async load(): Promise<void> {
        const results = await Promise.all(
            ALL_KEYS.map(async (key) => ({ key, result: await safeGet(key) })),
        );
        const values: Partial<Record<SettingKey, string>> = {};
        for (const { key, result } of results) {
            if (result.status === 'ok') {
                if (result.data !== null) {
                    values[key] = result.data;
                }
            } else {
                this.error = result.error;
            }
        }
        this.#values = values;
        this.loaded = true;
        writeBootMirror(values);
    }

    /** The mirrored value, if the profile has one. */
    get(key: SettingKey): string | undefined {
        return this.#values[key];
    }

    /** Writes through to the backend and updates the mirror on success. */
    async set(key: SettingKey, value: string): Promise<CommandResult<null>> {
        const result = await safeSet(key, value);
        if (result.status === 'ok') {
            this.#values = { ...this.#values, [key]: value };
            if (isBootKey(key)) {
                writeBootMirror(this.#values);
            }
        } else {
            this.error = result.error;
        }
        return result;
    }

    /** Test isolation. */
    reset(): void {
        this.#values = {};
        this.loaded = false;
        this.error = null;
    }
}

/** The parsed boot mirror — `{}` when absent, unreadable or not an object. */
function readBootMirror(): Record<string, unknown> {
    try {
        const raw = window.localStorage.getItem(BOOT_MIRROR_STORAGE_KEY);
        if (raw === null) {
            return {};
        }
        const parsed: unknown = JSON.parse(raw);
        return typeof parsed === 'object' && parsed !== null && !Array.isArray(parsed)
            ? (parsed as Record<string, unknown>)
            : {};
    } catch {
        // localStorage unavailable or the JSON is corrupt — boot at defaults.
        return {};
    }
}

/** Rewrites the boot mirror with the boot-shaping subset of `values`. */
function writeBootMirror(values: Partial<Record<SettingKey, string>>): void {
    const mirror: Partial<Record<SettingKey, string>> = {};
    for (const key of BOOT_KEYS) {
        const value = values[key];
        if (value !== undefined) {
            mirror[key] = value;
        }
    }
    try {
        window.localStorage.setItem(BOOT_MIRROR_STORAGE_KEY, JSON.stringify(mirror));
    } catch {
        // localStorage unavailable — the next boot starts at defaults, then settles.
    }
}

function internalError(thrown: unknown): CommandResult<never> {
    return {
        status: 'error',
        error: {
            kind: 'internal',
            code: 'INTERNAL',
            params: {},
            message: thrown instanceof Error ? thrown.message : String(thrown),
            recoverable: false,
        },
    };
}

async function safeGet(key: string): Promise<CommandResult<string | null>> {
    try {
        return await commands.getSetting(key);
    } catch (thrown) {
        return internalError(thrown);
    }
}

async function safeSet(key: string, value: string): Promise<CommandResult<null>> {
    try {
        return await commands.setSetting(key, value);
    } catch (thrown) {
        return internalError(thrown);
    }
}

/** The app-wide singleton. */
export const settingsStore = new SettingsStore();
