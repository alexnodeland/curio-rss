/**
 * String routing — `t()` resolves a typed message key against the active
 * locale's catalog, falling back to English for any key that locale has not
 * translated, and interpolates `{placeholder}` params. `t()` and the Intl
 * date helpers read the reactive `localeStore.active`, so switching language
 * re-renders every string that flows through them. English is the complete
 * base catalog; every key call site is typechecked against it.
 */
import { de } from './de';
import { en } from './en';
import { es } from './es';
import { fr } from './fr';
import { it } from './it';
import { type LocaleId, localeStore } from './locale.svelte';
import { pl } from './pl';
import { yue } from './yue';
import { zhHans } from './zh-Hans';

export { LOCALES, type LocaleId, localeStore } from './locale.svelte';

/** Every known message key — adding a call site with a typo fails typecheck. */
export type MessageKey = keyof typeof en;

/** Per-locale catalogs: `en` is complete, the rest are partial (fall back). */
const CATALOGS: Record<LocaleId, Partial<Record<MessageKey, string>>> = {
    en,
    es,
    fr,
    de,
    it,
    pl,
    'zh-Hans': zhHans,
    yue,
};

/**
 * A plural block: `{count, plural, one {# feed} other {# feeds}}`. The form for
 * the count's plural category (per `Intl.PluralRules` for the active locale) is
 * chosen — falling back to `other` — and `#` is replaced with the count
 * formatted through `Intl.NumberFormat`. Form bodies are brace-free (no nested
 * placeholders), which is all the count strings need.
 */
const PLURAL_BLOCK = /\{(\w+),\s*plural,\s*((?:\w+\s*\{[^{}]*\}\s*)+)\}/g;
const PLURAL_FORM = /(\w+)\s*\{([^{}]*)\}/g;

function interpolate(
    template: string,
    locale: LocaleId,
    params?: Record<string, string | number>,
): string {
    const p = params ?? {};
    // Plural blocks first (they contain their own `{form}` braces).
    const withPlurals = template.replace(PLURAL_BLOCK, (match, name: string, forms: string) => {
        const count = Number(p[name]);
        if (Number.isNaN(count)) {
            return match;
        }
        const category = new Intl.PluralRules(locale).select(count);
        const byCategory = new Map<string, string>();
        for (const form of forms.matchAll(PLURAL_FORM)) {
            byCategory.set(form[1], form[2]);
        }
        const chosen = byCategory.get(category) ?? byCategory.get('other') ?? '';
        return chosen.replace(/#/g, new Intl.NumberFormat(locale).format(count));
    });
    // Then simple `{name}` placeholders.
    return withPlurals.replace(/\{(\w+)\}/g, (match, name: string) => {
        const value = p[name];
        return value === undefined ? match : String(value);
    });
}

/**
 * Resolves `key` against the active locale (English fallback) and interpolates
 * `{name}` placeholders and `{n, plural, …}` blocks from `params`.
 */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
    const locale = localeStore.active;
    const template = CATALOGS[locale][key] ?? en[key];
    return interpolate(template, locale, params);
}

/** `June 3` this year, `June 3, 2024` otherwise — in the active locale. An
 *  unparseable date (`new Date('garbage')`) yields `''` rather than throwing a
 *  `RangeError` from `Intl.DateTimeFormat().format()` — a malformed
 *  `published_at` must not crash the reader to a blank screen. */
export function formatIntlDate(date: Date, now: Date = new Date()): string {
    if (Number.isNaN(date.getTime())) return '';
    const sameYear = date.getFullYear() === now.getFullYear();
    return new Intl.DateTimeFormat(localeStore.active, {
        month: 'long',
        day: 'numeric',
        ...(sameYear ? {} : { year: 'numeric' }),
    }).format(date);
}

/** How many calendar days back the list says "yesterday" / "3 days ago"
 *  instead of a date. */
export const LIST_DATE_RELATIVE_DAYS = 6;

/** Local calendar days from `date` to `now` (positive = `date` is earlier). */
function calendarDaysBefore(date: Date, now: Date): number {
    const startOf = (value: Date): number =>
        new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
    // Rounded, so a DST day (23 or 25 hours) still counts as one.
    return Math.round((startOf(now) - startOf(date)) / 86_400_000);
}

/** A list row's date, scaled to its age — in the active locale: the time of
 *  day for today (`4:05 PM`), a relative day for the last
 *  {@link LIST_DATE_RELATIVE_DAYS} days (`yesterday`, `3 days ago`), and the
 *  {@link formatIntlDate} date otherwise (also for a future date, which a
 *  feed's clock skew can produce on a later day). `''` when unparseable. */
export function formatIntlListDate(date: Date, now: Date = new Date()): string {
    if (Number.isNaN(date.getTime())) return '';
    const locale = localeStore.active;
    const days = calendarDaysBefore(date, now);
    if (days === 0) {
        return new Intl.DateTimeFormat(locale, { hour: 'numeric', minute: '2-digit' }).format(date);
    }
    if (days > 0 && days <= LIST_DATE_RELATIVE_DAYS) {
        return new Intl.RelativeTimeFormat(locale, { numeric: 'auto' }).format(-days, 'day');
    }
    return formatIntlDate(date, now);
}

/** Full date + time, e.g. `Jun 3, 2025, 4:05 PM` — in the active locale.
 *  Returns `''` for an unparseable date (see `formatIntlDate`). */
export function formatIntlDateTime(date: Date): string {
    if (Number.isNaN(date.getTime())) return '';
    return new Intl.DateTimeFormat(localeStore.active, {
        month: 'short',
        day: 'numeric',
        year: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
    }).format(date);
}
