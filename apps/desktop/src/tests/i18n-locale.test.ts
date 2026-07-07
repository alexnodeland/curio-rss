/**
 * Locale switching: `t()` resolves against the active locale and falls back
 * to English for keys a partial catalog hasn't translated; `set()` persists
 * the choice and drives `<html lang>`.
 */
import { LOCALES, localeStore, t } from '$lib/i18n';
import { afterEach, describe, expect, it } from 'vitest';
import { type IpcHarness, installIpcHarness } from './ipc-harness';

describe('locale switching', () => {
    let harness: IpcHarness | null = null;

    afterEach(() => {
        localeStore.reset();
        harness?.teardown();
        harness = null;
    });

    it('ships English plus at least one more locale', () => {
        expect(LOCALES.map((locale) => locale.id)).toContain('en');
        expect(LOCALES.length).toBeGreaterThan(1);
    });

    it('resolves the active locale and falls back to English', () => {
        expect(t('modal.close')).toBe('Close');

        localeStore.active = 'es';
        // A translated key resolves in Spanish...
        expect(t('modal.close')).toBe('Cerrar');
        // ...while an untranslated key falls back to English rather than
        // showing a raw key or an empty string.
        expect(t('help.chord.then')).toBe('then');
    });

    it('persists the choice and sets <html lang> through set()', async () => {
        harness = installIpcHarness({ set_setting: null });
        await localeStore.set('es');

        expect(localeStore.active).toBe('es');
        expect(document.documentElement.lang).toBe('es');
        expect(harness.callsFor('set_setting')).toEqual([{ key: 'ui.locale', value: 'es' }]);
    });
});
