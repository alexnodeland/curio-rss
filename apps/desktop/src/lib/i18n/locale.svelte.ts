/**
 * The active-locale rune. `t()` and the Intl helpers read `localeStore.active`,
 * so switching language re-renders every string that flows through them —
 * no page reload, no per-component wiring. The choice persists like any other
 * setting and drives `<html lang>` (assistive tech, hyphenation) and `<html
 * dir>` (writing direction) — the latter is RTL scaffolding: no shipped locale
 * is RTL yet, but adding one flips the document direction with no other change.
 */
import { SETTING_KEYS, settingsStore } from '$lib/state/settings.svelte';

/** The shippable locales, with their endonym for the language picker. */
export const LOCALES = [
    { id: 'en', name: 'English' },
    { id: 'es', name: 'Español' },
    { id: 'fr', name: 'Français' },
    { id: 'de', name: 'Deutsch' },
    { id: 'it', name: 'Italiano' },
    { id: 'pl', name: 'Polski' },
    { id: 'zh-Hans', name: '简体中文' },
    { id: 'yue', name: '廣東話' },
] as const;

export type LocaleId = (typeof LOCALES)[number]['id'];

export function isLocaleId(value: string | undefined): value is LocaleId {
    return LOCALES.some((locale) => locale.id === value);
}

/**
 * Language subtags that render right-to-left. None of the shipped locales are
 * RTL yet, so this is scaffolding: adding an RTL locale to {@link LOCALES} flips
 * `<html dir>` automatically, and the layout (built on logical properties)
 * follows without further wiring.
 */
const RTL_LANGUAGES: ReadonlySet<string> = new Set(['ar', 'he', 'fa', 'ur', 'ps', 'dv', 'yi']);

/** The writing direction for a locale, derived from its base language subtag. */
export function localeDir(id: string): 'ltr' | 'rtl' {
    const base = id.split('-')[0]?.toLowerCase() ?? '';
    return RTL_LANGUAGES.has(base) ? 'rtl' : 'ltr';
}

/**
 * Chinese written in Traditional script — `zh-Hant*`, or a region that writes
 * it (Hong Kong, Macau, Taiwan) unless the tag names Simplified script. The
 * shipped Traditional-script catalog is `yue`; `zh-Hans` (Simplified) would
 * be the wrong script.
 */
function isTraditionalChinese(subtags: readonly string[]): boolean {
    const rest = subtags.slice(1);
    return (
        subtags[0] === 'zh' &&
        !rest.includes('hans') &&
        rest.some((subtag) => ['hant', 'hk', 'mo', 'tw'].includes(subtag))
    );
}

/**
 * The first shipped locale matching a BCP 47 preference list (e.g.
 * `navigator.languages`): an exact id first, then the script (`zh-HK`,
 * `zh-Hant-TW` → `yue`, the Traditional-script catalog), then the base
 * language (`de-AT` → `de`, `zh-CN` → `zh-Hans`). `null` when none ships.
 */
export function matchLocale(languages: readonly string[]): LocaleId | null {
    for (const tag of languages) {
        const lower = tag.toLowerCase();
        const exact = LOCALES.find((locale) => locale.id.toLowerCase() === lower);
        if (exact) {
            return exact.id;
        }
        if (isTraditionalChinese(lower.split('-'))) {
            return 'yue';
        }
        const base = lower.split('-')[0];
        const byBase = LOCALES.find((locale) => locale.id.toLowerCase().split('-')[0] === base);
        if (byBase) {
            return byBase.id;
        }
    }
    return null;
}

function systemLanguages(): readonly string[] {
    return typeof navigator === 'undefined' ? [] : navigator.languages;
}

class LocaleStore {
    active: LocaleId = $state('en');

    /** The active locale's writing direction (`ltr`/`rtl`) — see {@link localeDir}. */
    get dir(): 'ltr' | 'rtl' {
        return localeDir(this.active);
    }

    /** Adopts the persisted locale at startup (after `settingsStore.load()`). */
    init(): void {
        const persisted = settingsStore.get(SETTING_KEYS.locale);
        if (isLocaleId(persisted)) {
            this.#apply(persisted);
        }
    }

    /**
     * Adopts the system language without persisting it — for the recovery
     * screen, where the persisted choice lives in a database that did not
     * open.
     */
    initFromSystem(languages: readonly string[] = systemLanguages()): void {
        const matched = matchLocale(languages);
        if (matched !== null) {
            this.#apply(matched);
        }
    }

    /** Switches locale and persists it. */
    async set(id: LocaleId): Promise<void> {
        this.#apply(id);
        await settingsStore.set(SETTING_KEYS.locale, id);
    }

    #apply(id: LocaleId): void {
        this.active = id;
        if (typeof document !== 'undefined') {
            document.documentElement.lang = id;
            document.documentElement.dir = localeDir(id);
        }
    }

    /** Test isolation. */
    reset(): void {
        this.#apply('en');
    }
}

export const localeStore = new LocaleStore();
