/**
 * String routing — `t()` resolves a typed message key against the active
 * locale's catalog, falling back to English for any key that locale has not
 * translated, and interpolates `{placeholder}` params. `t()` and the Intl
 * date helpers read the reactive `localeStore.active`, so switching language
 * re-renders every string that flows through them. English is the complete
 * base catalog; every key call site is typechecked against it.
 */
import { en } from './en';
import { es } from './es';
import { type LocaleId, localeStore } from './locale.svelte';
import { yue } from './yue';
import { zhHans } from './zh-Hans';

export { LOCALES, type LocaleId, localeStore } from './locale.svelte';

/** Every known message key — adding a call site with a typo fails typecheck. */
export type MessageKey = keyof typeof en;

/** Per-locale catalogs: `en` is complete, the rest are partial (fall back). */
const CATALOGS: Record<LocaleId, Partial<Record<MessageKey, string>>> = {
    en,
    es,
    'zh-Hans': zhHans,
    yue,
};

function interpolate(template: string, params?: Record<string, string | number>): string {
    if (params === undefined) {
        return template;
    }
    return template.replace(/\{(\w+)\}/g, (match, name: string) => {
        const value = params[name];
        return value === undefined ? match : String(value);
    });
}

/**
 * Resolves `key` against the active locale (English fallback) and interpolates
 * `{name}` placeholders from `params`.
 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
    const template = CATALOGS[localeStore.active][key] ?? en[key];
    return interpolate(template, params);
}

/** `June 3` this year, `June 3, 2024` otherwise — in the active locale. */
export function formatIntlDate(date: Date, now: Date = new Date()): string {
    const sameYear = date.getFullYear() === now.getFullYear();
    return new Intl.DateTimeFormat(localeStore.active, {
        month: 'long',
        day: 'numeric',
        ...(sameYear ? {} : { year: 'numeric' }),
    }).format(date);
}

/** Full date + time, e.g. `Jun 3, 2025, 4:05 PM` — in the active locale. */
export function formatIntlDateTime(date: Date): string {
    return new Intl.DateTimeFormat(localeStore.active, {
        month: 'short',
        day: 'numeric',
        year: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
    }).format(date);
}
