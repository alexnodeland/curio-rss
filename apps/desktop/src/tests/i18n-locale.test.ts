/**
 * Locale switching: `t()` resolves against the active locale, every shipped
 * non-English locale fully covers the English catalog (with placeholders
 * intact), and `set()` persists the choice and drives `<html lang>`.
 */
import { LOCALES, localeStore, t } from '$lib/i18n';
import { en } from '$lib/i18n/en';
import { es } from '$lib/i18n/es';
import { yue } from '$lib/i18n/yue';
import { zhHans } from '$lib/i18n/zh-Hans';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

type MessageKey = keyof typeof en;

const CATALOGS = [
    { id: 'es', catalog: es },
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

    it('ships English plus Spanish, Mandarin and Cantonese', () => {
        expect(LOCALES.map((locale) => locale.id)).toEqual(
            expect.arrayContaining(['en', 'es', 'zh-Hans', 'yue']),
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
