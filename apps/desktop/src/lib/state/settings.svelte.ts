/**
 * Persisted preferences — a reactive mirror of the profile's settings table,
 * read and written through the generated `get_setting`/`set_setting`
 * wrappers under namespaced `ui.*` keys. The backend owns storage; this
 * module owns nothing but the in-memory copy. (The `destinations` key is
 * facade-reserved — the backend rejects it; no key here may collide.)
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
    sidebarCollapsed: 'ui.panes.sidebar-collapsed',
    sidebarWidth: 'ui.panes.sidebar-width',
    listWidth: 'ui.panes.list-width',
    promoteDestination: 'ui.promote.destination',
    faviconAllowRemote: 'ui.favicon.allow-remote',
    homeYoutube: 'ui.home.youtube',
    homeReddit: 'ui.home.reddit',
    mediaPrefetch: 'ui.media.prefetch',
    markOnScroll: 'ui.reading.mark-on-scroll',
    locale: 'ui.locale',
    customThemes: 'ui.themes.custom',
} as const;

export type SettingKey = (typeof SETTING_KEYS)[keyof typeof SETTING_KEYS];

const ALL_KEYS: readonly SettingKey[] = Object.values(SETTING_KEYS);

export class SettingsStore {
    #values: Partial<Record<SettingKey, string>> = $state({});

    /** True once `load()` has settled (even partially). */
    loaded: boolean = $state(false);

    /** The first load/write failure, for the doctor surface. */
    error: CommandError | null = $state(null);

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

function internalError(thrown: unknown): CommandResult<never> {
    return {
        status: 'error',
        error: {
            kind: 'internal',
            code: 'INTERNAL',
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
