/**
 * Locale switching: `t()` resolves against the active locale, every shipped
 * non-English locale fully covers the English catalog (with placeholders
 * intact), and `set()` persists the choice and drives `<html lang>`.
 */
import { LOCALES, formatIntlDate, formatIntlDateTime, localeStore, t } from '$lib/i18n';
import { de } from '$lib/i18n/de';
import { en } from '$lib/i18n/en';
import { es } from '$lib/i18n/es';
import { fr } from '$lib/i18n/fr';
import { it as itCatalog } from '$lib/i18n/it';
import { pl } from '$lib/i18n/pl';
import { yue } from '$lib/i18n/yue';
import { zhHans } from '$lib/i18n/zh-Hans';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

type MessageKey = keyof typeof en;

const CATALOGS = [
    { id: 'es', catalog: es },
    { id: 'fr', catalog: fr },
    { id: 'de', catalog: de },
    { id: 'it', catalog: itCatalog },
    { id: 'pl', catalog: pl },
    { id: 'zh-Hans', catalog: zhHans },
    { id: 'yue', catalog: yue },
];

/** The `{placeholder}` tokens in a string, sorted for order-insensitive compare. */
function placeholders(text: string): string[] {
    return (text.match(/\{\w+\}/g) ?? []).sort();
}

describe('locale switching', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        localeStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('ships English plus seven translated locales', () => {
        expect(LOCALES.map((locale) => locale.id)).toEqual(
            expect.arrayContaining(['en', 'es', 'fr', 'de', 'it', 'pl', 'zh-Hans', 'yue']),
        );
    });

    it.each(CATALOGS)(
        '$id translates every English key, with placeholders preserved',
        ({ catalog }) => {
            for (const key of Object.keys(en) as MessageKey[]) {
                const value = catalog[key];
                expect(value, `missing translation for "${key}"`).toBeDefined();
                expect(placeholders(value ?? ''), `placeholder drift on "${key}"`).toEqual(
                    placeholders(en[key]),
                );
            }
        },
    );

    it('resolves the active locale', () => {
        expect(t('modal.close')).toBe('Close');

        localeStore.active = 'es';
        expect(t('modal.close')).toBe('Cerrar');

        localeStore.active = 'fr';
        expect(t('modal.close')).toBe('Fermer');

        localeStore.active = 'de';
        expect(t('modal.close')).toBe('Schließen');

        localeStore.active = 'it';
        expect(t('modal.close')).toBe('Chiudi');

        localeStore.active = 'pl';
        expect(t('modal.close')).toBe('Zamknij');

        localeStore.active = 'zh-Hans';
        expect(t('modal.close')).toBe('关闭'); // mainland Simplified

        localeStore.active = 'yue';
        expect(t('list.empty')).toBe('呢度冇文章'); // genuine colloquial Cantonese (冇, 呢度)
    });

    it('interpolates placeholders in the active locale', () => {
        localeStore.active = 'zh-Hans';
        expect(t('shell.unread.count', { count: 3 })).toBe('3 条未读');
    });

    it('falls back to English for a key the active locale has not translated', () => {
        // The catalogs are complete today, so simulate a locale that lags the
        // base by one key — `t()` must serve English rather than a blank.
        const original = es['modal.close'];
        try {
            es['modal.close'] = undefined;
            localeStore.active = 'es';
            expect(t('modal.close')).toBe('Close');
        } finally {
            es['modal.close'] = original;
        }
    });

    it('persists the choice and sets <html lang> through set()', async () => {
        harness = installIpcHarness({ set_setting: null });
        await localeStore.set('es');

        expect(localeStore.active).toBe('es');
        expect(document.documentElement.lang).toBe('es');
        expect(harness.callsFor('set_setting')).toEqual([{ key: 'ui.locale', value: 'es' }]);
    });
});

describe('date formatting — malformed dates never throw', () => {
    it('returns an empty string instead of a RangeError-crashing the reader', () => {
        // Intl.DateTimeFormat().format(invalidDate) throws RangeError; a
        // malformed published_at must degrade to no date, not a blank reader.
        const bad = new Date('not-a-date');
        expect(Number.isNaN(bad.getTime())).toBe(true);
        expect(formatIntlDate(bad)).toBe('');
        expect(formatIntlDateTime(bad)).toBe('');
    });

    it('still formats a valid date', () => {
        localeStore.active = 'en';
        const good = new Date('2025-06-03T16:05:00Z');
        expect(formatIntlDate(good, good)).not.toBe('');
        expect(formatIntlDateTime(good)).not.toBe('');
    });
});
