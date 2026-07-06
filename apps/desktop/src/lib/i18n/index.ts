/**
 * String routing — `t()` resolves a typed message key against the active
 * catalog (en only in v1) and interpolates `{placeholder}` params. Dates go
 * through `Intl` via the helpers below so locale handling stays in one place.
 */
import { en } from './en';

/** Every known message key — adding a call site with a typo fails typecheck. */
export type MessageKey = keyof typeof en;

/** The active BCP-47 locale. v1 ships English only; the type stays open. */
export const locale: string = 'en';

const catalog: Record<MessageKey, string> = en;

/** Resolves `key`, interpolating `{name}` placeholders from `params`. */
export function t(key: MessageKey, params?: Record<string, string | number>): string {
    const template = catalog[key];
    if (params === undefined) {
        return template;
    }
    return template.replace(/\{(\w+)\}/g, (match, name: string) => {
        const value = params[name];
        return value === undefined ? match : String(value);
    });
}

/** `June 3` this year, `June 3, 2024` otherwise — via `Intl`. */
export function formatIntlDate(date: Date, now: Date = new Date()): string {
    const sameYear = date.getFullYear() === now.getFullYear();
    return new Intl.DateTimeFormat(locale, {
        month: 'long',
        day: 'numeric',
        ...(sameYear ? {} : { year: 'numeric' }),
    }).format(date);
}

/** Full date + time, e.g. `Jun 3, 2025, 4:05 PM` — via `Intl`. */
export function formatIntlDateTime(date: Date): string {
    return new Intl.DateTimeFormat(locale, {
        month: 'short',
        day: 'numeric',
        year: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
    }).format(date);
}
