/**
 * The active-locale rune. `t()` and the Intl helpers read `localeStore.active`,
 * so switching language re-renders every string that flows through them —
 * no page reload, no per-component wiring. The choice persists like any other
 * setting and drives `<html lang>` for assistive tech and hyphenation.
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

class LocaleStore {
    active: LocaleId = $state('en');

    /** Adopts the persisted locale at startup (after `settingsStore.load()`). */
    init(): void {
        const persisted = settingsStore.get(SETTING_KEYS.locale);
        if (isLocaleId(persisted)) {
            this.#apply(persisted);
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
        }
    }

    /** Test isolation. */
    reset(): void {
        this.#apply('en');
    }
}

export const localeStore = new LocaleStore();
